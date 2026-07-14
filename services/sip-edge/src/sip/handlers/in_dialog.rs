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
    extract_uri_from_contact, parse_target_addr_from_route, sip_uri_from_peer, DialogLeg,
    EdgeState, PendingDatagram,
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

    let mut mutable_request = request;

    let (transaction, source_leg, is_target) = {
        let lookup_cid = edge_state
            .bridged_transfers
            .get(call_id.as_str())
            .map(|r| r.clone());
        let actual_cid = lookup_cid.as_deref().unwrap_or(call_id.as_str());

        let Some(mut t) = edge_state.inbound_transactions.get_mut(actual_cid) else {
            if matches!(&mutable_request.method, Method::Ack) {
                return Vec::new();
            }

            let error = call_error_for_unknown_request(&mutable_request);
            return vec![PendingDatagram::new(
                peer.to_string(),
                response::error_for_call_error(&mutable_request, &error),
            )];
        };

        let is_target = t.transfer_call_id.as_deref() == Some(call_id.as_str());

        let source_leg = if is_target {
            let leg = if t.transferee_is_caller {
                DialogLeg::Gateway
            } else {
                DialogLeg::Caller
            };

            // Rewrite Call-ID, From, To for original leg B
            mutable_request.headers.replace(
                HeaderName::new("call-id").unwrap(),
                HeaderValue::new(actual_cid),
            );
            if let Some(orig_req) = &t.original_request {
                let (from_val, to_val) = if t.transferee_is_caller {
                    (
                        orig_req.headers.get("to").cloned(),
                        orig_req.headers.get("from").cloned(),
                    )
                } else {
                    (
                        orig_req.headers.get("from").cloned(),
                        orig_req.headers.get("to").cloned(),
                    )
                };
                if let Some(f) = from_val {
                    mutable_request.headers.replace(
                        HeaderName::new("from").unwrap(),
                        HeaderValue::new(f.as_str()),
                    );
                }
                if let Some(o) = to_val {
                    mutable_request
                        .headers
                        .replace(HeaderName::new("to").unwrap(), HeaderValue::new(o.as_str()));
                }
            }

            leg
        } else {
            let is_bridged = t.transfer_call_id.is_some();

            let (leg, cseq_update) = match t.validate_in_dialog_request(&mutable_request, peer) {
                Ok(result) => result,
                Err(error) => {
                    return vec![PendingDatagram::new(
                        peer.to_string(),
                        response_for_dialog_validation_error(&mutable_request, &error),
                    )];
                }
            };

            // Update cseq outside of validation
            if let Some(cseq) = cseq_update {
                match leg {
                    DialogLeg::Caller => t.last_inbound_cseq = Some(cseq),
                    DialogLeg::Gateway => t.last_outbound_cseq = Some(cseq),
                }
            }

            if is_bridged {
                // Rewrite Call-ID, From, To for target leg C
                if let Some(ref tf_cid) = t.transfer_call_id {
                    mutable_request.headers.insert(
                        HeaderName::new("call-id").unwrap(),
                        HeaderValue::new(tf_cid),
                    );
                }
                if let Some(ref tf_from) = t.transfer_from_header {
                    mutable_request
                        .headers
                        .insert(HeaderName::new("from").unwrap(), HeaderValue::new(tf_from));
                }
                if let Some(ref tf_to) = t.transfer_to_header {
                    mutable_request
                        .headers
                        .insert(HeaderName::new("to").unwrap(), HeaderValue::new(tf_to));
                }
            }

            leg
        };

        (t.clone(), source_leg, is_target)
    };

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

            let cid = mutable_request
                .headers
                .get("call-id")
                .map(|val| val.as_str())
                .unwrap_or("missing-call-id");
            let dtmf_digits = edge_state.media_relay.get_dtmf_digits(cid);
            if let Some(digits) = &dtmf_digits {
                info!(call_id = cid, digits = %digits, "collected DTMF digits for call");
            }
            edge_state.media_relay.clear_dtmf_digits(cid);

            // Collect DTMF audit events for persistence to the detail table.
            let dtmf_events = edge_state.media_relay.take_dtmf_events(cid);
            if !dtmf_events.is_empty() {
                info!(
                    call_id = cid,
                    count = dtmf_events.len(),
                    "collected DTMF audit events for call"
                );
                if let Some(ref db) = edge_state.db_store {
                    if let Err(error) = db.insert_dtmf_events_batch(&dtmf_events).await {
                        warn!(%error, call_id = cid, "failed to persist DTMF audit events");
                    }
                }
            } else {
                edge_state.media_relay.clear_dtmf_events(cid);
            }

            // Clean up bridged mappings
            if transaction.transfer_call_id.is_some() {
                if let Some(ref tf_cid) = transaction.transfer_call_id {
                    edge_state.bridged_transfers.remove(tf_cid);
                }
                let lookup_cid = edge_state
                    .bridged_transfers
                    .get(call_id.as_str())
                    .map(|r| r.clone());
                let actual_cid = lookup_cid.as_deref().unwrap_or(call_id.as_str());
                edge_state.bridged_transfers.remove(actual_cid);
            }

            match edge_state.call_manager.handle_inbound_termination(
                &mutable_request,
                metrics,
                dtmf_digits,
            ) {
                Ok(outcome) => {
                    // Decrement active call count for the gateway.
                    let call_id_str = mutable_request
                        .headers
                        .get("call-id")
                        .map(|v| v.as_str())
                        .unwrap_or("");
                    if let Some(gw_id) = edge_state.call_manager.current_gateway_id(call_id_str) {
                        let status = {
                            let mut health = edge_state
                                .gateway_health
                                .lock()
                                .unwrap_or_else(|e| e.into_inner());
                            health.decrement_active(&gw_id);
                            health.get_gateway_status(&gw_id)
                        };
                        crate::timers::persist_gateway_health(edge_state, gw_id.clone(), status);
                    }

                    // Real-time billing: settle the call.
                    if let (true, Some(db)) = (
                        edge_state.billing_settlement_enabled,
                        edge_state.db_store.as_ref(),
                    ) {
                        let call = edge_state.call_manager.get(&outcome.call_id);
                        if let Some(call) = call {
                            let caller_user = call.caller.as_deref().and_then(|s| {
                                // Extract user from "sip:user@host" or "<sip:user@host>"
                                let idx = s.find("sip:")?;
                                let rest = &s[idx + 4..];
                                let end = rest.find(['@', ';', '>']).unwrap_or(rest.len());
                                if end == 0 {
                                    None
                                } else {
                                    Some(&rest[..end])
                                }
                            });
                            let callee = call.inbound.remote_uri.user.as_deref().unwrap_or("");
                            let duration_ms = call
                                .ended_at
                                .and_then(|e| e.duration_since(call.started_at).ok())
                                .map(|d| d.as_millis() as i64)
                                .unwrap_or(0);
                            if let Some(user) = caller_user {
                                let cid = outcome.call_id.as_str().to_string();
                                let db = db.clone();
                                let user = user.to_string();
                                let callee = callee.to_string();
                                tokio::spawn(async move {
                                    match db.settle_call(&cid, &user, &callee, duration_ms).await {
                                        Ok(Some(new_bal)) => {
                                            tracing::info!(call_id = %cid, user = %user, new_bal, "实时计费结算完成");
                                        }
                                        Ok(None) => {}
                                        Err(e) => {
                                            tracing::warn!(call_id = %cid, error = %e, "实时计费结算失败");
                                        }
                                    }
                                });
                            }
                        }
                    }

                    edge_state.clear_media_targets(&transaction);

                    // 如果是会议呼叫（单腿 UAS 呼叫），直接在本地终结并返回 200 OK，不转发给其他任何节点
                    let out_user = transaction.outbound_uri.user.as_deref().unwrap_or("");
                    if out_user.starts_with("conf_") || out_user.starts_with("room_") {
                        let username = transaction.original_request.as_ref().and_then(|req| {
                            crate::edge_state::EdgeState::username_from_request(req.as_ref())
                        });
                        if let Some(ref uname) = username {
                            edge_state.decrement_user_concurrency(uname);
                        }
                        edge_state.inbound_transactions.remove(call_id.as_str());

                        datagrams.push(PendingDatagram::new(
                            peer.to_string(),
                            response::ok_for_request(&mutable_request),
                        ));
                        return datagrams;
                    }

                    if transaction.transfer_call_id.is_some() {
                        let transferee_port = if transaction.transferee_is_caller {
                            transaction.caller_relay_rtp.as_ref().map(|ep| ep.port)
                        } else {
                            transaction.gateway_relay_rtp.as_ref().map(|ep| ep.port)
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
                    datagrams.push(PendingDatagram::new(
                        peer.to_string(),
                        response::error_for_call_error(&mutable_request, &error),
                    ));
                    return datagrams;
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
                let cid = mutable_request
                    .headers
                    .get("call-id")
                    .map(|v| v.as_str())
                    .unwrap_or("");
                if !cid.is_empty() {
                    edge_state.media_relay.register_info_dtmf_digit(cid, digit);
                }
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
                    .allocate_endpoint(&edge_config.media)
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

                let transferee_relay_rtp = match source_leg {
                    DialogLeg::Caller => transaction.gateway_relay_rtp.clone(),
                    DialogLeg::Gateway => transaction.caller_relay_rtp.clone(),
                };

                if let Some(transferee_relay) = &transferee_relay_rtp {
                    edge_state
                        .media_relay
                        .pair_ports(target_relay_rtp.port, transferee_relay.port);
                }

                let transfer_call_id = format!("transfer-{}-{}", call_id.as_str(), local_cseq);

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
                    transfer_call_id: transfer_call_id.clone(),
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
                    if let Some(mut t_mut) =
                        edge_state.inbound_transactions.get_mut(call_id.as_str())
                    {
                        t_mut.refer_subscription = Some(refer_sub);
                    }
                }

                edge_state
                    .refer_transfers
                    .insert(transfer_call_id.clone(), call_id.as_str().to_string());

                let from_header = match source_leg {
                    DialogLeg::Caller => mutable_request
                        .headers
                        .get("to")
                        .map(|v| v.as_str())
                        .unwrap_or(""),
                    DialogLeg::Gateway => mutable_request
                        .headers
                        .get("from")
                        .map(|v| v.as_str())
                        .unwrap_or(""),
                };
                let to_header = refer_to_str.unwrap_or("");

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
                    &transfer_call_id,
                    from_header,
                    to_header,
                    1,
                    &edge_config.advertised_addr,
                    &outbound_uri,
                    sdp_body.as_bytes(),
                    replaces_header_val.as_deref(),
                );

                let target_addr = outbound::target_addr_for(&outbound_uri);
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

    let (request_uri, route_set, target) =
        if transaction.transfer_call_id.is_some() {
            if is_target {
                if transaction.transferee_is_caller {
                    let uri = transaction
                        .caller_contact
                        .clone()
                        .unwrap_or_else(|| sip_uri_from_peer(&transaction.peer));
                    (uri, Vec::new(), transaction.peer.clone())
                } else {
                    let uri = transaction
                        .callee_contact
                        .clone()
                        .unwrap_or_else(|| transaction.outbound_uri.clone());
                    let target = if !transaction.outbound_route_set.is_empty() {
                        parse_target_addr_from_route(&transaction.outbound_route_set[0])
                            .unwrap_or_else(|| {
                                if transaction.callee_behind_nat {
                                    transaction.outbound_peer.clone().unwrap_or_else(|| {
                                        outbound::target_addr_for(&transaction.outbound_uri)
                                    })
                                } else {
                                    outbound::target_addr_for(&transaction.outbound_uri)
                                }
                            })
                    } else if transaction.callee_behind_nat {
                        transaction
                            .outbound_peer
                            .clone()
                            .unwrap_or_else(|| outbound::target_addr_for(&uri))
                    } else {
                        outbound::target_addr_for(&uri)
                    };
                    (uri, transaction.outbound_route_set.clone(), target)
                }
            } else {
                let uri = transaction
                    .transfer_contact
                    .clone()
                    .unwrap_or_else(|| transaction.outbound_uri.clone());
                let target = transaction
                    .transfer_peer
                    .clone()
                    .unwrap_or_else(|| outbound::target_addr_for(&uri));
                (uri, Vec::new(), target)
            }
        } else {
            match source_leg {
                DialogLeg::Caller => {
                    let request_uri = transaction
                        .callee_contact
                        .clone()
                        .unwrap_or_else(|| transaction.outbound_uri.clone());
                    let target = if !transaction.outbound_route_set.is_empty() {
                        parse_target_addr_from_route(&transaction.outbound_route_set[0])
                            .unwrap_or_else(|| {
                                if transaction.callee_behind_nat {
                                    transaction.outbound_peer.clone().unwrap_or_else(|| {
                                        outbound::target_addr_for(&transaction.outbound_uri)
                                    })
                                } else {
                                    outbound::target_addr_for(&transaction.outbound_uri)
                                }
                            })
                    } else if transaction.callee_behind_nat {
                        transaction
                            .outbound_peer
                            .clone()
                            .unwrap_or_else(|| outbound::target_addr_for(&request_uri))
                    } else {
                        outbound::target_addr_for(&request_uri)
                    };
                    (request_uri, transaction.outbound_route_set.clone(), target)
                }
                DialogLeg::Gateway => {
                    let request_uri = transaction
                        .caller_contact
                        .clone()
                        .unwrap_or_else(|| sip_uri_from_peer(&transaction.peer));
                    let target = if !transaction.inbound_route_set.is_empty() {
                        parse_target_addr_from_route(&transaction.inbound_route_set[0])
                            .unwrap_or_else(|| transaction.peer.clone())
                    } else {
                        transaction.peer.clone()
                    };
                    (request_uri, transaction.inbound_route_set.clone(), target)
                }
            }
        };

    let mut rewritten_sdp: Option<Vec<u8>> = None;
    let is_bridged = transaction.transfer_call_id.is_some();

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

    // Topology Hiding: when forwarding a request from the caller toward the gateway,
    // replace the inbound (internal) Call-ID with the external Call-ID the gateway knows.
    // When forwarding from gateway toward caller, no rewrite is needed — the caller sees
    // the internal Call-ID.
    if matches!(source_leg, DialogLeg::Caller) {
        let internal_cid = mutable_request
            .headers
            .get("call-id")
            .map(|v| v.as_str().to_string())
            .unwrap_or_default();
        if let Some(external_cid) = edge_state.get_external_call_id(&internal_cid) {
            mutable_request.headers.replace(
                HeaderName::new("call-id").unwrap(),
                HeaderValue::new(&external_cid),
            );
            debug!(
                internal_cid,
                external_cid, "topology hiding: rewrote Call-ID for in-dialog request to gateway"
            );
        }
    }

    let bytes = if let Some(body) = &rewritten_sdp {
        outbound::build_outbound_in_dialog_request_with_body(
            &mutable_request,
            &request_uri,
            &edge_config.advertised_addr,
            &route_set,
            body,
        )
    } else {
        outbound::build_outbound_in_dialog_request(
            &mutable_request,
            &request_uri,
            &edge_config.advertised_addr,
            &route_set,
        )
    };

    // BYE/CANCEL 转发后立即清理事务：更新并发计数并从 map 中删除
    if matches!(&mutable_request.method, Method::Bye | Method::Cancel) {
        let username: Option<String> = transaction
            .original_request
            .as_ref()
            .and_then(|req| crate::edge_state::EdgeState::username_from_request(req.as_ref()));
        if let Some(ref uname) = username {
            edge_state.decrement_user_concurrency(uname);
        }
        edge_state.inbound_transactions.remove(call_id.as_str());
    }

    datagrams.push(PendingDatagram::new(target, bytes));
    datagrams
}
