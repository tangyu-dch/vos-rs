use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Instant;

use call_core::CallError;
use sip_core::{HeaderName, HeaderValue, Method, SipRequest, SipUri};
use tracing::{debug, info, warn};

use super::{
    call_error_for_unknown_request, parse_sip_info_dtmf, percent_decode, register_relay_target,
    response_for_dialog_validation_error,
};
use crate::config::EdgeConfig;
use crate::edge_state::{
    extract_uri_from_contact, parse_target_addr_from_route, DialogLeg, DialogLegState, EdgeState,
    PendingDatagram, TransferDialogState,
};
use crate::media;
use crate::sip::{outbound, response};
use crate::timers::calculate_mos_for_legs;

pub(crate) async fn handle_in_dialog_request(
    request: SipRequest,
    peer: SocketAddr,
    edge_state: &EdgeState,
    edge_config: &EdgeConfig,
) -> Vec<PendingDatagram> {
    let Some(call_id) = request
        .headers
        .get("call-id")
        .map(|v| v.as_str().to_string())
    else {
        if matches!(&request.method, Method::Ack) {
            return Vec::new();
        }

        let error = CallError::MissingRequiredHeader("Call-ID");
        return vec![PendingDatagram::new(
            peer.to_string(),
            response::error_for_call_error(&request, &error),
        )];
    };

    let mutable_request = request;

    let (transaction, transaction_call_id, source_leg) = {
        let Some(mut t) = edge_state.inbound_transactions.get_mut(call_id.as_str()) else {
            if matches!(&mutable_request.method, Method::Ack) {
                return Vec::new();
            }

            let error = call_error_for_unknown_request(&mutable_request);
            return vec![PendingDatagram::new(
                peer.to_string(),
                response::error_for_call_error(&mutable_request, &error),
            )];
        };

        let (source_leg, cseq_update) = match t.validate_in_dialog_request(&mutable_request, peer) {
            Ok(result) => result,
            Err(error) => {
                return vec![PendingDatagram::new(
                    peer.to_string(),
                    response_for_dialog_validation_error(&mutable_request, &error),
                )];
            }
        };

        if let Some(cseq) = cseq_update {
            match source_leg {
                DialogLeg::Caller => t.dialogs.caller.remote_cseq = Some(cseq),
                DialogLeg::Gateway => t.dialogs.gateway.remote_cseq = Some(cseq),
                DialogLeg::Transfer => {
                    if let Some(transfer) = &mut t.transfer_dialog {
                        transfer.dialog.remote_cseq = Some(cseq);
                    }
                }
            }
        }

        let caller_call_id = t.dialogs.caller.call_id.clone();
        (t.clone(), caller_call_id, source_leg)
    };

    // A B2BUA terminates ACK on the receiving leg. The peer leg ACK is generated from that
    // leg's own INVITE client transaction when its final response arrives.
    if matches!(&mutable_request.method, Method::Ack) {
        return Vec::new();
    }

    let mut datagrams = Vec::new();
    match &mutable_request.method {
        Method::Bye | Method::Cancel => {
            let mut caller_rtcp = None;
            let mut gateway_rtcp = None;

            if let Some(endpoint) = &transaction.caller_relay_rtp {
                caller_rtcp = Some(
                    edge_state
                        .media_relay
                        .metrics_for_port(endpoint.port)
                        .rtcp_quality,
                );
            }
            if let Some(endpoint) = &transaction.gateway_relay_rtp {
                gateway_rtcp = Some(
                    edge_state
                        .media_relay
                        .metrics_for_port(endpoint.port)
                        .rtcp_quality,
                );
            }

            let metrics = if caller_rtcp.is_some() || gateway_rtcp.is_some() {
                Some(calculate_mos_for_legs(
                    caller_rtcp.as_ref(),
                    gateway_rtcp.as_ref(),
                ))
            } else {
                None
            };

            let media_session_id = transaction.session_id.as_str();
            let dtmf_digits = edge_state.media_relay.get_dtmf_digits(media_session_id);
            if let Some(digits) = &dtmf_digits {
                info!(
                    session_id = media_session_id,
                    digits = %digits,
                    "collected DTMF digits for call"
                );
            }
            edge_state.media_relay.clear_dtmf_digits(media_session_id);

            // Collect DTMF audit events for persistence to the detail table.
            let mut dtmf_events = edge_state.media_relay.take_dtmf_events(media_session_id);
            if !dtmf_events.is_empty() {
                info!(
                    session_id = media_session_id,
                    count = dtmf_events.len(),
                    "collected DTMF audit events for call"
                );
                if let Some(db) = edge_state.db_store.clone() {
                    let call_id = transaction.dialogs.caller.call_id.clone();
                    for event in &mut dtmf_events {
                        event.call_id.clone_from(&call_id);
                    }
                    tokio::spawn(async move {
                        if let Err(error) = db.insert_dtmf_events_batch(&dtmf_events).await {
                            warn!(%error, %call_id, "failed to persist DTMF audit events");
                        }
                    });
                }
            } else {
                edge_state.media_relay.clear_dtmf_events(media_session_id);
            }

            let mut termination_request = mutable_request.clone();
            termination_request.headers.replace(
                HeaderName::new("call-id").unwrap(),
                HeaderValue::new_owned(transaction_call_id.clone()),
            );
            match edge_state.call_manager.handle_inbound_termination(
                &termination_request,
                metrics,
                dtmf_digits,
            ) {
                Ok(outcome) => {
                    // Decrement active call count for the gateway.
                    if let Some(gw_id) = edge_state
                        .call_manager
                        .current_gateway_id(&transaction_call_id)
                    {
                        edge_state.gateway_health.decrement_active(&gw_id);
                        let status = edge_state.gateway_health.get_gateway_status(&gw_id);
                        crate::timers::persist_gateway_health(edge_state, gw_id.clone(), status);
                    }

                    crate::billing_settlement::settle_completed_call(edge_state, &outcome.call_id);

                    // 如果是会议呼叫（单腿 UAS 呼叫），直接在本地终结并返回 200 OK，不转发给其他任何节点
                    let out_user = transaction.outbound_uri.user.as_deref().unwrap_or("");
                    if out_user.starts_with("conf_")
                        || out_user.starts_with("room_")
                        || out_user == "vosrs-playback"
                        || out_user == "vosrs-gather"
                        || out_user == "vosrs-stream"
                    {
                        let username = transaction.original_request.as_ref().and_then(|req| {
                            crate::edge_state::EdgeState::username_from_request(req.as_ref())
                        });
                        if let Some(ref uname) = username {
                            edge_state.decrement_user_concurrency(uname);
                        }
                        let duration_secs = transaction
                            .established_at
                            .map(|i| i.elapsed().as_secs())
                            .unwrap_or(0);
                        if edge_config.webhooks.control_mode == "http"
                            || edge_config.webhooks.control_mode == "nats"
                        {
                            let edge_state_clone = edge_state
                                .self_weak
                                .get()
                                .and_then(|w| w.upgrade())
                                .unwrap();
                            let edge_config_clone = edge_config.clone();
                            let cid_clone = call_id.to_string();
                            tokio::spawn(async move {
                                let event = call_core::WebhookEvent {
                                    event_id: uuid::Uuid::new_v4().to_string(),
                                    schema_version: "1.0".to_string(),
                                    call_id: cid_clone,
                                    sequence: 5,
                                    occurred_at_ms: std::time::SystemTime::now()
                                        .duration_since(std::time::UNIX_EPOCH)
                                        .unwrap_or_default()
                                        .as_millis()
                                        as i64,
                                    event: call_core::CallEvent::CallFinished {
                                        duration_secs,
                                        sip_status: Some(200),
                                        q850_cause: Some(16),
                                        reason: "Normal clearing (local session ended)".to_string(),
                                        leg: "a_leg".to_string(),
                                    },
                                };
                                let _ =
                                    crate::sip::handlers::interactive_control::post_webhook_event(
                                        &edge_state_clone,
                                        &edge_config_clone,
                                        &event,
                                    )
                                    .await;
                            });
                        }
                        edge_state.teardown_call_transaction(&transaction_call_id);

                        datagrams.push(PendingDatagram::new(
                            peer.to_string(),
                            response::ok_for_request(&mutable_request),
                        ));
                        return datagrams;
                    }

                    if let Some(transfer) = &transaction.transfer_dialog {
                        let transferee_port = match transfer.transferee_leg {
                            DialogLeg::Caller => {
                                transaction.caller_relay_rtp.as_ref().map(|ep| ep.port)
                            }
                            DialogLeg::Gateway => {
                                transaction.gateway_relay_rtp.as_ref().map(|ep| ep.port)
                            }
                            DialogLeg::Transfer => None,
                        };
                        if let Some(tp) = transferee_port {
                            if let Some(cp) = edge_state.media_relay.peer_port_for(tp) {
                                edge_state.media_relay.clear_target(cp);
                            }
                        }
                    }

                    datagrams.push(PendingDatagram::new(
                        peer.to_string(),
                        response::ok_for_request(&mutable_request),
                    ));
                }
                Err(error) => {
                    // 即使 call_manager 找不到记录（UnknownCall），也必须把 200 OK 发回给发送方
                    // 并继续转发 BYE 给对端，否则另一方的呼叫永远无法挂断。
                    warn!(
                        call_id = call_id.as_str(),
                        %error,
                        source_leg = ?source_leg,
                        "handle_inbound_termination failed; still forwarding BYE to peer leg"
                    );
                    datagrams.push(PendingDatagram::new(
                        peer.to_string(),
                        response::ok_for_request(&mutable_request),
                    ));
                }
            }
        }
        Method::Info => {
            let content_type = mutable_request
                .headers
                .get("content-type")
                .map(|v| v.as_str())
                .unwrap_or("");
            if let Some(digit) = parse_sip_info_dtmf(content_type, &mutable_request.body) {
                edge_state
                    .media_relay
                    .register_info_dtmf_digit(&transaction.session_id, digit);
            }

            datagrams.push(PendingDatagram::new(
                peer.to_string(),
                response::ok_for_request(&mutable_request),
            ));
        }
        Method::Prack => {
            let rack_valid = if let Some(rack) = mutable_request.headers.get("rack") {
                let parts = rack.as_str().split_whitespace().collect::<Vec<_>>();
                if parts.len() == 3 {
                    let rseq_ok = parts[0].parse::<u32>().is_ok();
                    let cseq_ok = parts[1].parse::<u32>().is_ok();
                    let method_ok = !parts[2].is_empty();
                    rseq_ok && cseq_ok && method_ok
                } else {
                    false
                }
            } else {
                false
            };

            if !rack_valid {
                warn!("received PRACK with missing or invalid RAck header");
                datagrams.push(PendingDatagram::new(
                    peer.to_string(),
                    response::build_response_with_owned_headers(
                        &mutable_request,
                        400,
                        "Bad Request - Invalid RAck",
                        &[],
                        "",
                    ),
                ));
                return datagrams;
            }

            debug!(
                call_id = mutable_request
                    .headers
                    .get("call-id")
                    .map(|v| v.as_str())
                    .unwrap_or("?"),
                "received PRACK from caller — responding 200 OK (already confirmed to gateway)"
            );
            datagrams.push(PendingDatagram::new(
                peer.to_string(),
                response::ok_for_request(&mutable_request),
            ));
            return datagrams;
        }
        Method::Refer => {
            // RFC 3515 Blind Transfer B2BUA handling
            let refer_to_str = mutable_request.headers.get("refer-to").map(|v| v.as_str());
            let target_uri = refer_to_str.and_then(extract_uri_from_contact);

            datagrams.push(PendingDatagram::new(
                peer.to_string(),
                response::accepted_202_for_request(&mutable_request),
            ));

            if let Some(target_uri) = target_uri {
                let local_cseq = transaction.last_inbound_cseq.unwrap_or(1) + 50;

                let notify_body = "SIP/2.0 100 Trying\r\n";
                let notify = outbound::build_notify_sipfrag(
                    call_id.as_str(),
                    mutable_request
                        .headers
                        .get("from")
                        .map(|v| v.as_str())
                        .unwrap_or(""),
                    mutable_request
                        .headers
                        .get("to")
                        .map(|v| v.as_str())
                        .unwrap_or(""),
                    local_cseq,
                    &edge_config.advertised_addr,
                    notify_body,
                );
                datagrams.push(PendingDatagram::new(peer.to_string(), notify));

                let outbound_uri =
                    if let Some(contact) = edge_state.lookup_contact(&target_uri).await {
                        SipUri::from_str(&contact.uri).ok()
                    } else {
                        edge_state
                            .call_manager
                            .routes()
                            .select(&target_uri)
                            .ok()
                            .map(|sr| sr.outbound_uri)
                    };

                if outbound_uri.is_none() {
                    let notify_404 = outbound::build_notify_sipfrag_with_state(
                        call_id.as_str(),
                        mutable_request
                            .headers
                            .get("from")
                            .map(|v| v.as_str())
                            .unwrap_or(""),
                        mutable_request
                            .headers
                            .get("to")
                            .map(|v| v.as_str())
                            .unwrap_or(""),
                        local_cseq + 1,
                        &edge_config.advertised_addr,
                        "SIP/2.0 404 Not Found\r\n",
                        "terminated;reason=noresource",
                    );
                    datagrams.push(PendingDatagram::new(peer.to_string(), notify_404));
                    return datagrams;
                }
                let outbound_uri = outbound_uri.unwrap();

                let target_relay_rtp = match edge_state
                    .media_relay
                    .allocate_endpoint_for_call(&edge_config.media, &transaction.session_id)
                {
                    Ok(ep) => ep,
                    Err(error) => {
                        warn!(%error, "failed to allocate media relay endpoint for transfer target");
                        let notify_503 = outbound::build_notify_sipfrag_with_state(
                            call_id.as_str(),
                            mutable_request
                                .headers
                                .get("from")
                                .map(|v| v.as_str())
                                .unwrap_or(""),
                            mutable_request
                                .headers
                                .get("to")
                                .map(|v| v.as_str())
                                .unwrap_or(""),
                            local_cseq + 1,
                            &edge_config.advertised_addr,
                            "SIP/2.0 503 Service Unavailable\r\n",
                            "terminated;reason=noresource",
                        );
                        datagrams.push(PendingDatagram::new(peer.to_string(), notify_503));
                        return datagrams;
                    }
                };

                let transferee_leg = match source_leg {
                    DialogLeg::Caller => DialogLeg::Gateway,
                    DialogLeg::Gateway => DialogLeg::Caller,
                    DialogLeg::Transfer => transaction
                        .transfer_dialog
                        .as_ref()
                        .map(|transfer| transfer.transferee_leg)
                        .unwrap_or(DialogLeg::Caller),
                };
                let transferee_relay_rtp = match transferee_leg {
                    DialogLeg::Caller => transaction.caller_relay_rtp.clone(),
                    DialogLeg::Gateway => transaction.gateway_relay_rtp.clone(),
                    DialogLeg::Transfer => None,
                };

                if let Some(transferee_relay) = &transferee_relay_rtp {
                    edge_state
                        .media_relay
                        .pair_ports(target_relay_rtp.port, transferee_relay.port);
                }

                let transfer_call_id = format!("vosrs-transfer-{}", uuid::Uuid::new_v4().simple());
                let transferee_dialog = match transferee_leg {
                    DialogLeg::Caller => &transaction.dialogs.caller,
                    DialogLeg::Gateway => &transaction.dialogs.gateway,
                    DialogLeg::Transfer => {
                        warn!(call_id = %call_id, "invalid transfer-to-transfer dialog linkage");
                        return datagrams;
                    }
                };
                let target_addr = outbound::target_addr_for(&outbound_uri);
                let transfer_dialog = TransferDialogState {
                    dialog: DialogLegState {
                        call_id: transfer_call_id.clone(),
                        local_uri: transferee_dialog.remote_uri.clone(),
                        remote_uri: target_uri.clone(),
                        local_tag: format!("vosrs-t-{}", uuid::Uuid::new_v4().simple()),
                        remote_tag: None,
                        local_cseq: 1,
                        remote_cseq: None,
                        route_set: Vec::new(),
                        remote_target: outbound_uri.clone(),
                        peer: Some(target_addr.clone()),
                    },
                    transferee_leg,
                };

                let refer_sub = crate::edge_state::ReferSubscription {
                    refer_to: target_uri.to_string(),
                    from_header: mutable_request
                        .headers
                        .get("from")
                        .map(|v| v.as_str().to_string())
                        .unwrap_or_default(),
                    to_header: mutable_request
                        .headers
                        .get("to")
                        .map(|v| v.as_str().to_string())
                        .unwrap_or_default(),
                    notify_cseq: local_cseq,
                    referrer_peer: peer.to_string(),
                    refer_cseq: mutable_request
                        .headers
                        .get("cseq")
                        .and_then(|v| crate::sip::dialog::cseq_number(v.as_str()))
                        .unwrap_or(1),
                    target_relay_port: Some(target_relay_rtp.port),
                    transferee_relay_port: transferee_relay_rtp.as_ref().map(|ep| ep.port),
                };

                {
                    if let Some(mut t_mut) = edge_state
                        .inbound_transactions
                        .get_mut(&transaction.session_id)
                    {
                        t_mut.refer_subscription = Some(refer_sub);
                        t_mut.transfer_dialog = Some(transfer_dialog.clone());
                    }
                }
                edge_state
                    .inbound_transactions
                    .index_dialog(&transaction.session_id, &transfer_call_id);

                let sdp_body = format!(
                    "v=0\r\no=- 0 0 IN IP4 {addr}\r\ns=-\r\nc=IN IP4 {addr}\r\nt=0 0\r\nm=audio {port} RTP/AVP 0 8 101\r\na=rtpmap:0 PCMU/8000\r\na=rtpmap:8 PCMA/8000\r\na=rtpmap:101 telephone-event/8000\r\na=fmtp:101 0-16\r\n",
                    addr = edge_config.advertised_addr,
                    port = target_relay_rtp.port,
                );

                let replaces_header_val = if let Some(refer_to_val) = refer_to_str {
                    if let Some(idx) = refer_to_val.find("?Replaces=") {
                        let part = &refer_to_val[idx + "?Replaces=".len()..];
                        let end_idx = part.find('>').unwrap_or(part.len());
                        let encoded = &part[..end_idx];
                        Some(percent_decode(encoded))
                    } else if let Some(idx) = refer_to_val.find("&Replaces=") {
                        let part = &refer_to_val[idx + "&Replaces=".len()..];
                        let end_idx = part.find('>').unwrap_or(part.len());
                        let encoded = &part[..end_idx];
                        Some(percent_decode(encoded))
                    } else {
                        None
                    }
                } else {
                    None
                };

                let invite_bytes = outbound::build_transfer_invite(
                    &transfer_dialog.dialog,
                    &edge_config.advertised_addr,
                    sdp_body.as_bytes(),
                    replaces_header_val.as_deref(),
                );

                datagrams.push(PendingDatagram::new(target_addr, invite_bytes));
                return datagrams;
            } else {
                warn!(
                    call_id = call_id.as_str(),
                    "missing or invalid Refer-To header in REFER"
                );
                let notify_400 = outbound::build_notify_sipfrag_with_state(
                    call_id.as_str(),
                    mutable_request
                        .headers
                        .get("from")
                        .map(|v| v.as_str())
                        .unwrap_or(""),
                    mutable_request
                        .headers
                        .get("to")
                        .map(|v| v.as_str())
                        .unwrap_or(""),
                    transaction.last_inbound_cseq.unwrap_or(1) + 50,
                    &edge_config.advertised_addr,
                    "SIP/2.0 400 Bad Request\r\n",
                    "terminated;reason=noresource",
                );
                datagrams.push(PendingDatagram::new(peer.to_string(), notify_400));
                return datagrams;
            }
        }
        _ => {}
    }

    let target_dialog = {
        let Some(mut current) = edge_state
            .inbound_transactions
            .get_mut(transaction.session_id.as_str())
        else {
            let error = call_error_for_unknown_request(&mutable_request);
            datagrams.push(PendingDatagram::new(
                peer.to_string(),
                response::error_for_call_error(&mutable_request, &error),
            ));
            return datagrams;
        };
        let target_leg = match source_leg {
            DialogLeg::Caller
                if current
                    .transfer_dialog
                    .as_ref()
                    .is_some_and(|transfer| transfer.transferee_leg == DialogLeg::Caller) =>
            {
                DialogLeg::Transfer
            }
            DialogLeg::Gateway
                if current
                    .transfer_dialog
                    .as_ref()
                    .is_some_and(|transfer| transfer.transferee_leg == DialogLeg::Gateway) =>
            {
                DialogLeg::Transfer
            }
            DialogLeg::Caller => DialogLeg::Gateway,
            DialogLeg::Gateway => DialogLeg::Caller,
            DialogLeg::Transfer => current
                .transfer_dialog
                .as_ref()
                .map(|transfer| transfer.transferee_leg)
                .unwrap_or(DialogLeg::Caller),
        };
        let dialog = match target_leg {
            DialogLeg::Caller => Some(&mut current.dialogs.caller),
            DialogLeg::Gateway => Some(&mut current.dialogs.gateway),
            DialogLeg::Transfer => current
                .transfer_dialog
                .as_mut()
                .map(|transfer| &mut transfer.dialog),
        };
        let Some(dialog) = dialog else {
            let error = call_error_for_unknown_request(&mutable_request);
            datagrams.push(PendingDatagram::new(
                peer.to_string(),
                response::error_for_call_error(&mutable_request, &error),
            ));
            return datagrams;
        };
        if !matches!(&mutable_request.method, Method::Cancel) {
            dialog.local_cseq = dialog.local_cseq.saturating_add(1);
        }
        dialog.clone()
    };

    let request_uri = target_dialog.remote_target.clone();
    let target = target_dialog
        .route_set
        .first()
        .and_then(|route| parse_target_addr_from_route(route))
        .or_else(|| target_dialog.peer.clone())
        .unwrap_or_else(|| outbound::target_addr_for(&request_uri));
    let route_set = target_dialog.route_set.clone();

    let mut rewritten_sdp: Option<Vec<u8>> = None;
    let is_bridged = transaction.transfer_dialog.is_some();

    if !is_bridged && matches!(&mutable_request.method, Method::Invite | Method::Update) {
        {
            if let Some(mut t_mut) = edge_state.inbound_transactions.get_mut(call_id.as_str()) {
                t_mut.last_session_refresh = Some(Instant::now());
                debug!(
                    call_id = call_id.as_str(),
                    "session timer refreshed by Re-INVITE/UPDATE"
                );
            }
        }

        if media::is_sdp_body(&mutable_request.headers, &mutable_request.body) {
            let is_from_caller = peer.to_string() == transaction.peer;
            if is_from_caller {
                if let Some(gw_relay) = &transaction.gateway_relay_rtp {
                    // Single-pass: rewrite SDP + extract original endpoint
                    if let Ok((rewritten, remote_ep)) =
                        media::rewrite_sdp_and_extract_endpoint(&mutable_request.body, gw_relay)
                    {
                        rewritten_sdp = Some(rewritten);
                        register_relay_target(
                            &edge_state.media_relay,
                            gw_relay,
                            &remote_ep,
                            "mid-dialog caller target update",
                        );

                        if let Some(mut t_mut) =
                            edge_state.inbound_transactions.get_mut(call_id.as_str())
                        {
                            t_mut.caller_rtp = Some(remote_ep);
                            t_mut.original_request = Some(Arc::new(mutable_request.clone()));
                        }
                    }
                }
            } else if let Some(caller_relay) = &transaction.caller_relay_rtp {
                // Single-pass: rewrite SDP + extract original endpoint
                if let Ok((rewritten, remote_ep)) =
                    media::rewrite_sdp_and_extract_endpoint(&mutable_request.body, caller_relay)
                {
                    rewritten_sdp = Some(rewritten);
                    register_relay_target(
                        &edge_state.media_relay,
                        caller_relay,
                        &remote_ep,
                        "mid-dialog gateway target update",
                    );

                    if let Some(mut t_mut) =
                        edge_state.inbound_transactions.get_mut(call_id.as_str())
                    {
                        t_mut.gateway_rtp = Some(remote_ep);
                    }
                }
            }
        }
    }

    let outbound_body = rewritten_sdp
        .as_deref()
        .unwrap_or(mutable_request.body.as_ref());
    let bytes = outbound::build_b2bua_in_dialog_request(
        &mutable_request,
        &request_uri,
        &edge_config.advertised_addr,
        &route_set,
        &target_dialog.call_id,
        &target_dialog.local_uri,
        &target_dialog.local_tag,
        &target_dialog.remote_uri,
        target_dialog.remote_tag.as_deref(),
        target_dialog.local_cseq,
        outbound_body,
    );

    // BYE/CANCEL 转发后立即清理事务：更新并发计数并从 map 中删除
    if matches!(&mutable_request.method, Method::Bye | Method::Cancel) {
        let username: Option<String> = transaction
            .original_request
            .as_ref()
            .and_then(|req| crate::edge_state::EdgeState::username_from_request(req.as_ref()));
        if let Some(ref uname) = username {
            edge_state.decrement_user_concurrency(uname);
        }
        let duration_secs = transaction
            .established_at
            .map(|i| i.elapsed().as_secs())
            .unwrap_or(0);
        if edge_config.webhooks.control_mode == "http"
            || edge_config.webhooks.control_mode == "nats"
        {
            let edge_state_clone = edge_state
                .self_weak
                .get()
                .and_then(|w| w.upgrade())
                .unwrap();
            let edge_config_clone = edge_config.clone();
            let cid_clone = call_id.clone();
            tokio::spawn(async move {
                let event = call_core::WebhookEvent {
                    event_id: uuid::Uuid::new_v4().to_string(),
                    schema_version: "1.0".to_string(),
                    call_id: cid_clone,
                    sequence: 5,
                    occurred_at_ms: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis() as i64,
                    event: call_core::CallEvent::CallFinished {
                        duration_secs,
                        sip_status: Some(200),
                        q850_cause: Some(16),
                        reason: "Normal clearing".to_string(),
                        leg: "a_leg".to_string(),
                    },
                };
                let _ = crate::sip::handlers::interactive_control::post_webhook_event(
                    &edge_state_clone,
                    &edge_config_clone,
                    &event,
                )
                .await;
            });
        }
        edge_state.teardown_call_transaction(&transaction_call_id);
    }

    if matches!(&mutable_request.method, Method::Bye | Method::Cancel) {
        info!(
            call_id = call_id.as_str(),
            target = %target,
            source_leg = ?source_leg,
            "forwarding BYE/CANCEL to peer leg"
        );
    }
    datagrams.push(PendingDatagram::new(target, bytes));
    datagrams
}
