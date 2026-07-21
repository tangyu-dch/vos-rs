use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Instant;

use sdp_core::RtpEndpoint;
use sip_core::{HeaderName, HeaderValue, SipResponse, SipUri};
use tracing::{debug, info, warn};

use crate::config::EdgeConfig;
use crate::edge_state::{EdgeState, PendingDatagram};
use crate::media;
use crate::sip::{outbound, response, transaction, ClientTransactionKey, RequestTransactionKey};

pub(crate) async fn dispatch_response(
    mut sip_response: SipResponse,
    peer: SocketAddr,
    edge_state: &EdgeState,
    edge_config: &EdgeConfig,
) -> Vec<PendingDatagram> {
    let is_self_refresh = sip_response
        .headers
        .get_all("via")
        .any(|v| v.as_str().contains("branch=z9hG4bK-refresh-"));

    if is_self_refresh {
        let call_id = sip_response
            .headers
            .get("call-id")
            .map(|v| v.as_str().to_string());
        if sip_response.status_code >= 200 && sip_response.status_code < 300 {
            if let Some(ref cid) = call_id {
                if let Some(mut t_mut) = edge_state.inbound_transactions.get_mut(cid) {
                    t_mut.last_session_refresh = Some(std::time::Instant::now());
                    debug!(call_id = %cid, "received 200 OK for self-generated session refresh");
                }
            }
        } else if sip_response.status_code >= 300 {
            warn!(
                call_id = ?call_id,
                status = sip_response.status_code,
                "self-generated session refresh request failed"
            );
        }
        return Vec::new();
    }

    if let Some(key) = ClientTransactionKey::from_response(&sip_response) {
        edge_state.cancel_client_transaction(&key);
    }
    let call_id = sip_response
        .headers
        .get("call-id")
        .map(|call_id| call_id.as_str().to_string());

    if let Some(ref probe_call_id) = call_id {
        if sip_response.status_code >= 200 {
            if let Some((_, gateway_id)) = edge_state.gateway_probes.remove(probe_call_id) {
                if sip_response.status_code < 300 {
                    crate::timers::record_probe_success(edge_state, &gateway_id);
                } else {
                    crate::timers::record_probe_failure(
                        edge_state,
                        &gateway_id,
                        format!("OPTIONS returned {}", sip_response.status_code),
                    );
                }
                return Vec::new();
            }
        }
    }

    if let Some(ref reg_call_id) = call_id {
        let is_outbound_reg = edge_state
            .outbound_registrations
            .iter()
            .any(|entry| entry.value().call_id == *reg_call_id);
        if is_outbound_reg {
            return crate::sip::outbound_reg::handle_outbound_register_response(
                edge_state,
                edge_config,
                &sip_response,
                reg_call_id,
            );
        }
    }

    let raw_external_call_id = sip_response
        .headers
        .get("call-id")
        .map(|call_id| call_id.as_str().to_string())
        .unwrap_or_default();

    let call_id = if let Some(ref cid) = call_id {
        if let Some(internal_cid) = edge_state.get_internal_call_id(cid) {
            debug!(external_call_id = %cid, internal_call_id = %internal_cid, "topology hiding: translated gateway Call-ID to internal");
            if let Ok(name) = HeaderName::new("call-id") {
                sip_response
                    .headers
                    .replace(name, HeaderValue::new_owned(internal_cid.clone()));
            }
            Some(internal_cid)
        } else {
            call_id.clone()
        }
    } else {
        call_id.clone()
    };

    let is_invite = sip_response
        .headers
        .get("cseq")
        .map(|cseq| cseq.as_str().contains("INVITE"))
        .unwrap_or(false);
    let response_cseq = sip_response
        .headers
        .get("cseq")
        .and_then(|value| crate::sip::dialog::cseq_number(value.as_str()));

    // UDP worker tasks may finish out of order under load. Keep response ordering local to
    // each dialog so a delayed 1xx can never be emitted after that INVITE's final response.
    let invite_response_order = if is_invite {
        call_id.as_deref().and_then(|cid| {
            edge_state
                .inbound_transactions
                .get(cid)
                .map(|transaction| Arc::clone(&transaction.invite_response_order))
        })
    } else {
        None
    };
    let mut invite_response_guard = match invite_response_order.as_ref() {
        Some(order) => Some(order.lock().await),
        None => None,
    };
    if let Some(order) = invite_response_guard.as_mut() {
        if order.cseq != response_cseq {
            order.cseq = response_cseq;
            order.final_response_seen = false;
            order.final_response_send_started = false;
        }
        if sip_response.status_code < 200 && order.final_response_seen {
            debug!(
                call_id = ?call_id,
                status = sip_response.status_code,
                "dropping late provisional INVITE response after final response"
            );
            return Vec::new();
        }
        if sip_response.status_code >= 200 {
            order.final_response_seen = true;
        }
    }

    let mut cancel_datagrams = Vec::new();

    if is_invite {
        if let Some(ref cid) = call_id {
            if (200..300).contains(&sip_response.status_code) {
                let mut forks_to_cancel = Vec::new();
                let mut request_user = None;
                let mut from_header = String::new();
                let mut to_header = String::new();
                let mut invite_cseq = 1;

                if let Some(mut t_mut) = edge_state.inbound_transactions.get_mut(cid) {
                    if !t_mut.active_forks.is_empty() {
                        for (fork_cid, fork_gw) in t_mut.active_forks.iter() {
                            if fork_cid != &raw_external_call_id {
                                forks_to_cancel.push((fork_cid.clone(), fork_gw.clone()));
                            }
                        }
                        t_mut.active_forks.clear();
                    }
                    if let Some(ref orig_req) = t_mut.original_request {
                        from_header = orig_req
                            .headers
                            .get("from")
                            .map(|v| v.as_str().to_string())
                            .unwrap_or_default();
                        to_header = orig_req
                            .headers
                            .get("to")
                            .map(|v| v.as_str().to_string())
                            .unwrap_or_default();
                        invite_cseq = orig_req
                            .headers
                            .get("cseq")
                            .and_then(|v| crate::sip::dialog::cseq_number(v.as_str()))
                            .unwrap_or(1);
                        request_user = orig_req.uri.user.clone();
                    }
                }

                for (fork_cid, fork_gw) in forks_to_cancel {
                    if !fork_gw.is_empty() {
                        edge_state.gateway_health.decrement_active(&fork_gw);
                        let status = edge_state.gateway_health.get_gateway_status(&fork_gw);
                        crate::timers::persist_gateway_health(edge_state, fork_gw.clone(), status);
                    }

                    if let Some(ref user) = request_user {
                        let routes = edge_state.call_manager.routes();
                        let gateway_target = routes
                            .routes()
                            .iter()
                            .find(|r| r.target.gateway_id.as_str() == fork_gw)
                            .map(|r| r.target.clone());
                        if let Some(target) = gateway_target {
                            let outbound_uri = sip_core::SipUri {
                                secure: false,
                                user: Some(user.clone()),
                                host: target.host.clone().into(),
                                port: target.port,
                                params: Vec::new(),
                            };
                            let target_addr = outbound::target_addr_for(&outbound_uri);
                            let branch = format!("z9hG4bK-cancel-{}", fork_cid);
                            let cancel_bytes = format!(
                                "CANCEL {uri} SIP/2.0\r\n\
                                 Via: SIP/2.0/UDP {addr};branch={branch}\r\n\
                                 Max-Forwards: 70\r\n\
                                 From: {from}\r\n\
                                 To: {to}\r\n\
                                 Call-ID: {fork_cid}\r\n\
                                 CSeq: {cseq} CANCEL\r\n\
                                 Content-Length: 0\r\n\r\n",
                                uri = outbound_uri,
                                addr = edge_config.advertised_addr,
                                branch = branch,
                                from = from_header,
                                to = to_header,
                                fork_cid = fork_cid,
                                cseq = invite_cseq
                            )
                            .into_bytes();
                            cancel_datagrams.push(PendingDatagram::new(target_addr, cancel_bytes));
                        }
                    }
                }
            } else if sip_response.status_code >= 300 {
                let mut fork_gw_to_decrement = None;
                if let Some(mut t_mut) = edge_state.inbound_transactions.get_mut(cid) {
                    if let Some(pos) = t_mut
                        .active_forks
                        .iter()
                        .position(|(f_cid, _)| f_cid == &raw_external_call_id)
                    {
                        let (_, gw) = t_mut.active_forks.remove(pos);
                        fork_gw_to_decrement = Some(gw);
                    }
                }
                if let Some(gw_id) = fork_gw_to_decrement {
                    if !gw_id.is_empty() {
                        edge_state.gateway_health.decrement_active(&gw_id);
                        let status = edge_state.gateway_health.get_gateway_status(&gw_id);
                        crate::timers::persist_gateway_health(edge_state, gw_id.clone(), status);
                    }
                }
            }
        }
    }

    let original_call_id = if let Some(ref cid) = call_id {
        edge_state.refer_transfers.get(cid).map(|r| r.clone())
    } else {
        None
    };

    if let Some(orig_cid) = original_call_id {
        if let Some((_, mut t)) = edge_state.inbound_transactions.remove(&orig_cid) {
            if let Some(ref original_request) = t.original_request {
                if let Some(uname) =
                    crate::edge_state::EdgeState::username_from_request(original_request)
                {
                    edge_state.decrement_user_concurrency(&uname);
                }
            }
            if let Some(ref mut sub) = t.refer_subscription {
                sub.notify_cseq += 1;
                let status_code = sip_response.status_code;
                let notify_body =
                    format!("SIP/2.0 {} {}\r\n", status_code, sip_response.reason_phrase);

                let sub_state = if status_code >= 200 {
                    "terminated;reason=noresource"
                } else {
                    "active;expires=60"
                };

                let notify = outbound::build_notify_sipfrag_with_state(
                    &orig_cid,
                    &sub.from_header,
                    &sub.to_header,
                    sub.notify_cseq,
                    &edge_config.advertised_addr,
                    &notify_body,
                    sub_state,
                );
                let mut datagrams = vec![PendingDatagram::new(sub.referrer_peer.clone(), notify)];

                if (200..300).contains(&status_code) {
                    let bye_cseq = t.last_outbound_cseq.unwrap_or(100) + 1;
                    t.last_outbound_cseq = Some(bye_cseq);

                    let to_tag_str = t
                        .inbound_to_tag
                        .as_ref()
                        .map(|s| format!(";tag={}", s))
                        .unwrap_or_default();
                    let from_tag_str = t
                        .inbound_from_tag
                        .as_ref()
                        .map(|s| format!(";tag={}", s))
                        .unwrap_or_default();
                    let from_hdr = format!(
                        "{};tag={}",
                        sub.to_header.split(';').next().unwrap_or(""),
                        to_tag_str
                    );
                    let to_hdr = format!(
                        "{};tag={}",
                        sub.from_header.split(';').next().unwrap_or(""),
                        from_tag_str
                    );

                    let req_uri = t
                        .caller_contact
                        .clone()
                        .unwrap_or_else(|| crate::edge_state::sip_uri_from_peer(&t.peer));

                    let bye_branch = format!("z9hG4bK-bye-{}-{}", orig_cid, bye_cseq);
                    let bye_bytes = format!(
                        "BYE {req_uri} SIP/2.0\r\n\
                         Via: SIP/2.0/UDP {addr};branch={bye_branch}\r\n\
                         Max-Forwards: 70\r\n\
                         From: {from_hdr}\r\n\
                         To: {to_hdr}\r\n\
                         Call-ID: {orig_cid}\r\n\
                         CSeq: {bye_cseq} BYE\r\n\
                         Content-Length: 0\r\n\r\n",
                        req_uri = req_uri,
                        addr = edge_config.advertised_addr,
                        bye_branch = bye_branch,
                        from_hdr = from_hdr,
                        to_hdr = to_hdr,
                        orig_cid = orig_cid,
                        bye_cseq = bye_cseq
                    )
                    .into_bytes();

                    datagrams.push(PendingDatagram::new(sub.referrer_peer.clone(), bye_bytes));

                    if let (Some(target_port), Some(transferee_port)) =
                        (sub.target_relay_port, sub.transferee_relay_port)
                    {
                        if let Ok(c_media_rtp) = media::parse_sdp_rtp_endpoint(&sip_response.body) {
                            let target_ep = RtpEndpoint {
                                address: c_media_rtp.address.clone(),
                                port: c_media_rtp.port,
                            };
                            let transferee_relay_ep = RtpEndpoint {
                                address: edge_config
                                    .advertised_addr
                                    .split(':')
                                    .next()
                                    .unwrap_or("127.0.0.1")
                                    .to_string(),
                                port: transferee_port,
                            };
                            let _ = edge_state
                                .media_relay
                                .set_target(&transferee_relay_ep, &target_ep);

                            let transferee_is_caller = sub.transferee_relay_port.is_some()
                                && t.caller_relay_rtp.as_ref().map(|ep| ep.port)
                                    == sub.transferee_relay_port;
                            let transferee_dest = if transferee_is_caller {
                                t.caller_rtp.clone()
                            } else {
                                t.gateway_rtp.clone()
                            };
                            if let Some(dest) = transferee_dest {
                                let target_relay_ep = RtpEndpoint {
                                    address: edge_config
                                        .advertised_addr
                                        .split(':')
                                        .next()
                                        .unwrap_or("127.0.0.1")
                                        .to_string(),
                                    port: target_port,
                                };
                                let _ = edge_state.media_relay.set_target(&target_relay_ep, &dest);
                            }
                        }
                    }

                    let from_header_val = sip_response
                        .headers
                        .get("from")
                        .map(|v| v.as_str().to_string());
                    let to_header_val = sip_response
                        .headers
                        .get("to")
                        .map(|v| v.as_str().to_string());
                    let contact_val = sip_response
                        .headers
                        .get("contact")
                        .and_then(|v| crate::edge_state::extract_uri_from_contact(v.as_str()));

                    t.transfer_from_header = from_header_val;
                    t.transfer_to_header = to_header_val;
                    t.transfer_call_id = call_id.clone();
                    t.transfer_contact = contact_val;
                    t.transfer_peer = Some(peer.to_string());
                    t.transferee_is_caller = sub.transferee_relay_port.is_some()
                        && t.caller_relay_rtp.as_ref().map(|ep| ep.port)
                            == sub.transferee_relay_port;

                    if let Some(ref cid) = call_id {
                        edge_state
                            .bridged_transfers
                            .insert(cid.clone(), orig_cid.clone());
                        edge_state
                            .bridged_transfers
                            .insert(orig_cid.clone(), cid.clone());
                    }
                }

                if status_code >= 300 {
                    if let Some(target_port) = sub.target_relay_port {
                        edge_state.media_relay.clear_target(target_port);
                    }

                    let transferee_is_caller = sub.transferee_relay_port.is_some()
                        && t.caller_relay_rtp.as_ref().map(|ep| ep.port)
                            == sub.transferee_relay_port;
                    let transferee_relay = if transferee_is_caller {
                        t.caller_relay_rtp.clone()
                    } else {
                        t.gateway_relay_rtp.clone()
                    };
                    let referrer_relay = if transferee_is_caller {
                        t.gateway_relay_rtp.clone()
                    } else {
                        t.caller_relay_rtp.clone()
                    };
                    if let (Some(transferee_relay), Some(referrer_relay)) =
                        (transferee_relay, referrer_relay)
                    {
                        edge_state
                            .media_relay
                            .pair_ports(transferee_relay.port, referrer_relay.port);

                        let ref_dest = if transferee_is_caller {
                            t.gateway_rtp.clone()
                        } else {
                            t.caller_rtp.clone()
                        };
                        if let Some(ref_dest) = ref_dest {
                            let _ = edge_state
                                .media_relay
                                .set_target(&transferee_relay, &ref_dest);
                        }
                        let trans_dest = if transferee_is_caller {
                            t.caller_rtp.clone()
                        } else {
                            t.gateway_rtp.clone()
                        };
                        if let Some(trans_dest) = trans_dest {
                            let _ = edge_state
                                .media_relay
                                .set_target(&referrer_relay, &trans_dest);
                        }
                        debug!(orig_cid = ?orig_cid, "restored original media session after transfer failure");
                    }
                }

                if status_code >= 200 {
                    if let Some(ref cid) = call_id {
                        edge_state.refer_transfers.remove(cid);
                    }
                    t.refer_subscription = None;
                }

                edge_state.inbound_transactions.insert(orig_cid, t);
                return datagrams;
            }
        }
    }

    if let Some(call_id) = call_id.as_deref() {
        edge_state.remember_inbound_to_tag(call_id, &sip_response);
        {
            if let Some(mut t_mut) = edge_state.inbound_transactions.get_mut(call_id) {
                t_mut.outbound_peer = Some(peer.to_string());
            }
        }
        if sip_response.status_code >= 180 && sip_response.status_code < 300 {
            if let Some(mut t_mut) = edge_state.inbound_transactions.get_mut(call_id) {
                t_mut.outbound_route_set = sip_response
                    .headers
                    .get_all("record-route")
                    .map(|value| value.as_str().to_string())
                    .collect::<Vec<_>>();
                if let Some(contact_val) = sip_response.headers.get("contact") {
                    if let Some(mut uri) =
                        crate::edge_state::extract_uri_from_contact(contact_val.as_str())
                    {
                        if uri.port.is_none() {
                            uri.port = t_mut.outbound_uri.port;
                        }
                        t_mut.callee_contact = Some(uri);
                    }
                }
            }
            if sip_response.status_code < 200
                && (edge_config.webhooks.control_mode == "http"
                    || edge_config.webhooks.control_mode == "nats")
            {
                let edge_state_clone = edge_state
                    .self_weak
                    .get()
                    .and_then(|w| w.upgrade())
                    .unwrap();
                let edge_config_clone = edge_config.clone();
                let cid_clone = call_id.to_string();
                let status = sip_response.status_code;
                tokio::spawn(async move {
                    let event = call_core::WebhookEvent {
                        event_id: uuid::Uuid::new_v4().to_string(),
                        schema_version: "1.0".to_string(),
                        call_id: cid_clone,
                        sequence: 2,
                        occurred_at_ms: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as i64,
                        event: call_core::CallEvent::CallRinging {
                            sip_status: status,
                            leg: "b_leg".to_string(),
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
        }
    }
    let transaction = call_id.as_deref().and_then(|call_id| {
        edge_state
            .inbound_transactions
            .get(call_id)
            .map(|r| r.clone())
    });

    if sip_response.status_code >= 200 && sip_response.status_code < 300 {
        if let Some(cid) = call_id.as_deref() {
            let is_invite_local = sip_response
                .headers
                .get("cseq")
                .map(|c| c.as_str().contains("INVITE"))
                .unwrap_or(false);
            if is_invite_local
                && (edge_config.webhooks.control_mode == "http"
                    || edge_config.webhooks.control_mode == "nats")
            {
                let edge_state_clone = edge_state
                    .self_weak
                    .get()
                    .and_then(|w| w.upgrade())
                    .unwrap();
                let edge_config_clone = edge_config.clone();
                let cid_clone = cid.to_string();
                let status = sip_response.status_code;
                tokio::spawn(async move {
                    let event = call_core::WebhookEvent {
                        event_id: uuid::Uuid::new_v4().to_string(),
                        schema_version: "1.0".to_string(),
                        call_id: cid_clone.clone(),
                        sequence: 3,
                        occurred_at_ms: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as i64,
                        event: call_core::CallEvent::CallAnswered {
                            sip_status: status,
                            leg: "b_leg".to_string(),
                        },
                    };
                    if let Some(next_inst) =
                        crate::sip::handlers::interactive_control::post_webhook_event(
                            &edge_state_clone,
                            &edge_config_clone,
                            &event,
                        )
                        .await
                    {
                        crate::sip::handlers::interactive_control::execute_instruction(
                            next_inst,
                            cid_clone,
                            edge_state_clone,
                            edge_config_clone,
                        )
                        .await;
                    }
                });
            }
            if let Some(mut t_mut) = edge_state.inbound_transactions.get_mut(cid) {
                if t_mut.established_at.is_none() {
                    t_mut.established_at = Some(std::time::Instant::now());
                }
            }
            let se_header = sip_response
                .headers
                .get("session-expires")
                .or_else(|| sip_response.headers.get("x"))
                .map(|v| v.as_str().to_string());
            if let Some(se_val) = se_header {
                let mut parts = se_val.splitn(2, ';');
                let secs: Option<u32> = parts.next().and_then(|s| s.trim().parse().ok());
                let refresher = parts
                    .next()
                    .and_then(|p| p.split('=').nth(1).map(|r| r.trim().to_string()))
                    .unwrap_or_else(|| "uac".to_string());
                if let Some(secs) = secs {
                    if let Some(mut t_mut) = edge_state.inbound_transactions.get_mut(cid) {
                        t_mut.session_expires = Some(secs);
                        t_mut.session_refresher = Some(refresher);
                        t_mut.last_session_refresh = Some(Instant::now());
                        debug!(
                            call_id = cid,
                            session_expires = secs,
                            "stored Session-Expires from 200 OK"
                        );
                    }
                }
            }
        }
    }

    let is_message = sip_response
        .headers
        .get("cseq")
        .map(|cseq| cseq.as_str().contains("MESSAGE"))
        .unwrap_or(false);
    if is_message && sip_response.status_code >= 200 {
        if let Some(cid) = call_id.as_deref() {
            edge_state.inbound_transactions.remove(cid);
            debug!(call_id = %cid, "cleaned up temporary MESSAGE transaction");
        }
    }

    let is_invite = sip_response
        .headers
        .get("cseq")
        .map(|cseq| cseq.as_str().contains("INVITE"))
        .unwrap_or(false);

    let is_reinvite_response = is_invite
        && transaction
            .as_ref()
            .map(|t| t.established_at.is_some())
            .unwrap_or(false);

    let mut outbound_response_outcome = if is_invite && !is_reinvite_response {
        match edge_state
            .call_manager
            .handle_outbound_response(&sip_response)
        {
            Ok(outcome) => outcome,
            Err(error) => {
                if sip_response.status_code >= 200 && sip_response.status_code < 300 {
                    debug!(%error, "200 OK arrived after call terminated, forwarding anyway");
                } else {
                    warn!(%error, "failed to apply outbound SIP response");
                    return Vec::new();
                }
                call_core::OutboundResponseOutcome {
                    call_id: call_core::CallId::new(
                        sip_response
                            .headers
                            .get("call-id")
                            .map(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                    ),
                    state: call_core::CallState::Established,
                    failover_uri: None,
                    gateway_id: String::new(),
                    failover_gateway_id: None,
                    caller_identity: None,
                }
            }
        }
    } else {
        call_core::OutboundResponseOutcome {
            call_id: call_core::CallId::new(
                sip_response
                    .headers
                    .get("call-id")
                    .map(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
            ),
            state: call_core::CallState::Established,
            failover_uri: None,
            gateway_id: String::new(),
            failover_gateway_id: None,
            caller_identity: None,
        }
    };

    if is_invite && !is_reinvite_response {
        let gateway_id = outbound_response_outcome.gateway_id.clone();
        if !gateway_id.is_empty() {
            if sip_response.status_code >= 200 && sip_response.status_code <= 299 {
                edge_state.gateway_health.record_success(&gateway_id);
            } else if sip_response.status_code >= 400 {
                edge_state.gateway_health.record_failure(&gateway_id);
            }

            if let (
                true,
                Some(db),
                Some((
                    open,
                    failures,
                    state_str,
                    last_failure_at,
                    half_open_successes,
                    active_calls,
                )),
            ) = (
                edge_state.gateway_health_persistence_enabled,
                edge_state.db_store.clone(),
                edge_state.gateway_health.get_gateway_status(&gateway_id),
            ) {
                let gw = gateway_id.clone();
                let last_failure_at = last_failure_at.map(|st| {
                    let secs = st
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs() as i64;
                    time::OffsetDateTime::from_unix_timestamp(secs)
                        .unwrap_or(time::OffsetDateTime::UNIX_EPOCH)
                });
                tokio::spawn(async move {
                    if let Err(e) = db
                        .save_gateway_health(
                            &gw,
                            open,
                            failures,
                            &state_str,
                            last_failure_at,
                            half_open_successes,
                            None,
                            active_calls,
                        )
                        .await
                    {
                        tracing::warn!(gateway = %gw, error = %e, "无法异步持久化网关健康状态");
                    }
                });
            }
        }
    }

    if outbound_response_outcome.failover_uri.is_some() {
        if let Err(error) = crate::resource_lease::migrate_to_current(
            edge_state,
            &outbound_response_outcome.call_id,
            &outbound_response_outcome.gateway_id,
        )
        .await
        {
            warn!(
                call_id = %outbound_response_outcome.call_id.as_str(),
                %error,
                "gateway failover rejected because resource lease migration failed"
            );
            edge_state.call_manager.terminate_call_with_reason(
                outbound_response_outcome.call_id.as_str(),
                &error.to_string(),
            );
            outbound_response_outcome.failover_uri = None;
            outbound_response_outcome.failover_gateway_id = None;
            outbound_response_outcome.state = call_core::CallState::Failed;
        }
    }

    if let Some(next_uri) = outbound_response_outcome.failover_uri {
        info!(
            call_id = ?call_id,
            status = sip_response.status_code,
            %next_uri,
            "triggering gateway failover"
        );

        let old_gw = &outbound_response_outcome.gateway_id;
        if !old_gw.is_empty() {
            edge_state.gateway_health.decrement_active(old_gw);
        }
        if let Some(new_gateway_id) = outbound_response_outcome.failover_gateway_id.as_deref() {
            edge_state.gateway_health.increment_active(new_gateway_id);
        }

        if let Some(transaction) = transaction.as_ref() {
            edge_state.clear_media_targets(transaction);
        }

        let original_request = transaction
            .as_ref()
            .and_then(|t| t.original_request.as_ref());
        let rewritten_sdp = if let Some(req) = original_request {
            match crate::sip::handlers::prepare_rewritten_sdp(
                &req.headers,
                &req.body,
                &edge_state.media_relay,
                &edge_config.media,
                "failover INVITE offer",
                call_id.as_deref().unwrap_or(""),
            ) {
                Ok(rewritten_sdp) => rewritten_sdp,
                Err(error) => {
                    warn!(%error, "failed to prepare media for failover INVITE");
                    None
                }
            }
        } else {
            None
        };

        if let (Some(call_id), Some(sdp)) = (call_id.as_deref(), rewritten_sdp.as_ref()) {
            if let Some(caller_rtp) = &sdp.original_endpoint {
                crate::sip::handlers::register_relay_target(
                    &edge_state.media_relay,
                    &sdp.relay_endpoint,
                    caller_rtp,
                    "gateway-to-caller RTP (failover)",
                );
            }

            if let Some(mut t_mut) = edge_state.inbound_transactions.get_mut(call_id) {
                t_mut.outbound_uri = next_uri.clone();
                t_mut.gateway_relay_rtp = Some(sdp.relay_endpoint.clone());
                t_mut.caller_rtp = sdp.original_endpoint.clone();
                t_mut.gateway_rtp = None;
                t_mut.caller_relay_rtp = None;
            }
        }

        if let (Some(req), Some(sdp)) = (original_request, rewritten_sdp) {
            let target = outbound::target_addr_for(&next_uri);
            let failover_internal_cid = req
                .headers
                .get("call-id")
                .map(|v| v.as_str().to_string())
                .unwrap_or_default();
            let failover_external_cid = edge_state
                .get_external_call_id(&failover_internal_cid)
                .unwrap_or_else(|| failover_internal_cid.clone());
            let bytes = outbound::build_outbound_invite_with_body_call_id_and_caller(
                req,
                &next_uri,
                &edge_config.advertised_addr,
                sdp.body.as_slice(),
                &failover_external_cid,
                outbound_response_outcome.caller_identity.as_ref(),
            );
            return vec![PendingDatagram::new(target, bytes)];
        } else {
            warn!(
                "could not perform failover because original request or rewritten sdp is missing"
            );
            return Vec::new();
        }
    }

    if matches!(
        outbound_response_outcome.state,
        call_core::CallState::Failed
    ) {
        if let Some(transaction) = transaction.as_ref() {
            edge_state.clear_media_targets(transaction);
        }
        if !is_reinvite_response {
            if let Some(cid) = call_id.as_deref() {
                let username = edge_state.inbound_transactions.get(cid).and_then(|tx| {
                    tx.original_request
                        .as_ref()
                        .and_then(|req| crate::edge_state::EdgeState::username_from_request(req))
                });
                if let Some(ref uname) = username {
                    edge_state.decrement_user_concurrency(uname);
                }
                if !outbound_response_outcome.gateway_id.is_empty() {
                    edge_state
                        .gateway_health
                        .decrement_active(&outbound_response_outcome.gateway_id);
                }
                crate::resource_lease::release(
                    edge_state,
                    &call_core::CallId::new(cid.to_string()),
                );
                if edge_config.webhooks.control_mode == "http"
                    || edge_config.webhooks.control_mode == "nats"
                {
                    let edge_state_clone = edge_state
                        .self_weak
                        .get()
                        .and_then(|w| w.upgrade())
                        .unwrap();
                    let edge_config_clone = edge_config.clone();
                    let cid_clone = cid.to_string();
                    let status = sip_response.status_code;
                    tokio::spawn(async move {
                        let event = call_core::WebhookEvent {
                            event_id: uuid::Uuid::new_v4().to_string(),
                            schema_version: "1.0".to_string(),
                            call_id: cid_clone,
                            sequence: 4,
                            occurred_at_ms: std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_millis() as i64,
                            event: call_core::CallEvent::CallFinished {
                                duration_secs: 0,
                                sip_status: Some(status),
                                q850_cause: Some(16),
                                reason: "Call setup failed".to_string(),
                                leg: "b_leg".to_string(),
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
                edge_state.inbound_transactions.remove(cid);
            }
        }
    }

    let mut rewritten_sdp_body = None;
    let mut mid_dialog_rewritten = false;

    if let Some(t) = &transaction {
        if t.gateway_relay_rtp.is_some() && t.caller_relay_rtp.is_some() {
            mid_dialog_rewritten = true;
            if media::is_sdp_body(&sip_response.headers, &sip_response.body) {
                let is_to_caller = peer.to_string() != t.peer;
                let relay_ep = if is_to_caller {
                    t.caller_relay_rtp.as_ref()
                } else {
                    t.gateway_relay_rtp.as_ref()
                };

                if let Some(ep) = relay_ep {
                    if let Ok((rewritten, remote_ep)) =
                        media::rewrite_sdp_and_extract_endpoint(&sip_response.body, ep)
                    {
                        rewritten_sdp_body = Some(rewritten);
                        crate::sip::handlers::register_relay_target(
                            &edge_state.media_relay,
                            ep,
                            &remote_ep,
                            "mid-dialog response target update",
                        );

                        if let Some(cid) = call_id.as_deref() {
                            if let Some(mut t_mut) = edge_state.inbound_transactions.get_mut(cid) {
                                if is_to_caller {
                                    t_mut.gateway_rtp = Some(remote_ep);
                                } else {
                                    t_mut.caller_rtp = Some(remote_ep);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    let rewritten_sdp_bytes = if mid_dialog_rewritten {
        rewritten_sdp_body
    } else {
        let caller_is_webrtc = transaction
            .as_ref()
            .and_then(|value| value.original_request.as_ref())
            .is_some_and(|request| media::is_webrtc_sdp(&request.body));
        let prepared =
            if caller_is_webrtc && media::is_sdp_body(&sip_response.headers, &sip_response.body) {
                crate::sip::handlers::prepare_webrtc_answer(
                    &sip_response.body,
                    &edge_state.media_relay,
                    &edge_config.media,
                    call_id.as_deref().unwrap_or(""),
                )
                .map(Some)
            } else {
                crate::sip::handlers::prepare_rewritten_sdp(
                    &sip_response.headers,
                    &sip_response.body,
                    &edge_state.media_relay,
                    &edge_config.media,
                    "outbound response answer",
                    call_id.as_deref().unwrap_or(""),
                )
            };
        match prepared {
            Ok(Some(sdp)) => {
                if let (Some(call_id), Some(gateway_rtp)) =
                    (call_id.as_deref(), &sdp.original_endpoint)
                {
                    crate::sip::handlers::register_relay_target(
                        &edge_state.media_relay,
                        &sdp.relay_endpoint,
                        gateway_rtp,
                        "caller-to-gateway RTP",
                    );

                    if let Some(pt) = media::parse_sdp_dtmf_payload_type(&sip_response.body) {
                        edge_state.media_relay.register_port_dtmf_tracking(
                            call_id,
                            sdp.relay_endpoint.port,
                            pt,
                        );
                    }

                    if let Some(t) = &transaction {
                        if let Some(original_req) = &t.original_request {
                            if let Some(pt) = media::parse_sdp_dtmf_payload_type(&original_req.body)
                            {
                                if let Some(gateway_relay) = &t.gateway_relay_rtp {
                                    edge_state.media_relay.register_port_dtmf_tracking(
                                        call_id,
                                        gateway_relay.port,
                                        pt,
                                    );
                                }
                            }
                        }
                    }

                    edge_state.remember_gateway_media(
                        call_id,
                        sdp.original_endpoint.clone(),
                        sdp.relay_endpoint.clone(),
                        &edge_config.media,
                    );
                }
                Some(sdp.body)
            }
            _ => None,
        }
    };

    let cseq_method = sip_response
        .headers
        .get("cseq")
        .map(|cseq| cseq.as_str())
        .unwrap_or("");
    let is_renegotiation_response =
        cseq_method.contains("INVITE") || cseq_method.contains("UPDATE");
    let is_message_response = cseq_method.contains("MESSAGE");
    if !is_renegotiation_response && !is_message_response {
        return Vec::new();
    }

    let is_100rel = sip_response.status_code >= 180
        && sip_response.status_code < 200
        && sip_response
            .headers
            .get("require")
            .map(|v| v.as_str().contains("100rel"))
            .unwrap_or(false);

    if is_100rel {
        if let Some(cid) = call_id.as_deref() {
            let gw_rseq = sip_response
                .headers
                .get("rseq")
                .and_then(|v| v.as_str().trim().parse::<u32>().ok())
                .unwrap_or(1);

            let (our_rseq, prack_cseq, from_val, to_val, outbound_uri) = {
                if let Some(mut t_mut) = edge_state.inbound_transactions.get_mut(cid) {
                    t_mut.prack_rseq += 1;
                    t_mut.gateway_100rel = true;
                    let our_rseq = t_mut.prack_rseq;
                    let prack_cseq = t_mut.last_inbound_cseq.unwrap_or(1) + 100 + our_rseq;
                    let from_val = sip_response
                        .headers
                        .get("from")
                        .map(|v| v.as_str().to_string())
                        .unwrap_or_default();
                    let to_val = sip_response
                        .headers
                        .get("to")
                        .map(|v| v.as_str().to_string())
                        .unwrap_or_default();
                    (
                        our_rseq,
                        prack_cseq,
                        from_val,
                        to_val,
                        t_mut.outbound_uri.clone(),
                    )
                } else {
                    (
                        1,
                        1,
                        String::new(),
                        String::new(),
                        transaction
                            .as_ref()
                            .map(|t| t.outbound_uri.clone())
                            .unwrap_or_else(|| SipUri::from_str("sip:unknown@127.0.0.1").unwrap()),
                    )
                }
            };

            let gw_cseq_num = sip_response
                .headers
                .get("cseq")
                .and_then(|v| v.as_str().split_whitespace().next()?.parse::<u32>().ok())
                .unwrap_or(1);
            let rack_value = format!("{gw_rseq} {gw_cseq_num} INVITE");

            let prack_call_id = edge_state
                .get_external_call_id(cid)
                .unwrap_or_else(|| cid.to_string());
            let prack_bytes = outbound::build_outbound_prack(
                &prack_call_id,
                &from_val,
                &to_val,
                prack_cseq,
                &rack_value,
                &edge_config.advertised_addr,
                &outbound_uri,
            );
            let gw_target = outbound::target_addr_for(&outbound_uri);
            let mut datagrams: Vec<PendingDatagram> =
                vec![PendingDatagram::new(gw_target, prack_bytes)];

            if let Some(t) = transaction.as_ref() {
                let mut rewritten_response =
                    response::forward_response_to_inbound_with_body_and_call_id(
                        &sip_response,
                        &t.vias,
                        &t.inbound_route_set,
                        rewritten_sdp_bytes
                            .as_deref()
                            .unwrap_or(sip_response.body.as_ref()),
                        call_id.as_deref(),
                    );
                let raw_str = String::from_utf8_lossy(&rewritten_response);
                let patched = crate::sip::handlers::replace_header_value(
                    &raw_str,
                    "RSeq",
                    &our_rseq.to_string(),
                );
                rewritten_response = patched.into_bytes();

                if let (Some(ref orig_req), Ok(peer_addr)) =
                    (&t.original_request, t.peer.parse::<SocketAddr>())
                {
                    if let Some(key) = RequestTransactionKey::from_request(orig_req, peer_addr) {
                        if let Some(tx) = edge_state.get_server_transaction(&key) {
                            let _ = tx
                                .send(transaction::ServerTransactionEvent::UpdateLastProvisional(
                                    rewritten_response.clone(),
                                ))
                                .await;
                        }
                    }
                }

                let caller_response = PendingDatagram::new(t.peer.clone(), rewritten_response);
                let caller_response = match invite_response_order.as_ref() {
                    Some(order) => caller_response.with_invite_response_order(
                        Arc::clone(order),
                        response_cseq,
                        sip_response.status_code,
                    ),
                    None => caller_response,
                };
                datagrams.push(caller_response);
            }
            return datagrams;
        }
    }

    match transaction {
        Some(transaction) => {
            if transaction.peer == "local-originate" {
                // Originated call response: register media target and ACK 200 OK.
                if let Some(ep) = transaction.caller_relay_rtp.as_ref() {
                    let sdp_bytes = rewritten_sdp_bytes
                        .as_deref()
                        .unwrap_or(sip_response.body.as_ref());
                    if let Ok(remote_ep) = crate::media::sdp::parse_sdp_rtp_endpoint(sdp_bytes) {
                        if let Err(e) = edge_state.media_relay.set_target(ep, &remote_ep) {
                            tracing::warn!(error = %e, "originate: failed to set relay target");
                        }
                        if let Some(cid) = call_id.as_deref() {
                            if let Some(mut t_mut) = edge_state.inbound_transactions.get_mut(cid) {
                                t_mut.caller_rtp = Some(remote_ep);
                            }
                        }
                    }
                }
                let mut datagrams = Vec::new();
                if is_invite && (200..300).contains(&sip_response.status_code) {
                    let request_uri = transaction
                        .callee_contact
                        .as_ref()
                        .unwrap_or(&transaction.outbound_uri);
                    let ack_bytes = outbound::build_success_response_ack(
                        &sip_response,
                        request_uri,
                        &edge_config.advertised_addr,
                        call_id.as_deref().unwrap_or(""),
                        &transaction.outbound_route_set,
                    );
                    datagrams.push(PendingDatagram::new(peer.to_string(), ack_bytes));
                    // Emit CallAnswered event for the originated leg
                    if let Some(edge_arc) = edge_state.self_weak.get().and_then(|w| w.upgrade()) {
                        let cfg = edge_config.clone();
                        let cid_str = call_id.as_deref().unwrap_or("").to_string();
                        tokio::spawn(async move {
                            use call_core::{CallEvent, WebhookEvent, WEBHOOK_SCHEMA_VERSION};
                            use std::time::{SystemTime, UNIX_EPOCH};
                            let event = WebhookEvent {
                                event_id: uuid::Uuid::new_v4().to_string(),
                                schema_version: WEBHOOK_SCHEMA_VERSION.to_string(),
                                call_id: cid_str,
                                sequence: 3,
                                occurred_at_ms: SystemTime::now()
                                    .duration_since(UNIX_EPOCH)
                                    .unwrap_or_default()
                                    .as_millis()
                                    as i64,
                                event: CallEvent::CallAnswered {
                                    sip_status: 200,
                                    leg: "b_leg".to_string(),
                                },
                            };
                            crate::sip::handlers::interactive_control::post_webhook_event(
                                &edge_arc, &cfg, &event,
                            )
                            .await;
                        });
                    }
                }
                return datagrams;
            }

            let gateway_success_ack = if is_invite
                && (200..300).contains(&sip_response.status_code)
                && !raw_external_call_id.is_empty()
            {
                let request_uri = transaction
                    .callee_contact
                    .as_ref()
                    .unwrap_or(&transaction.outbound_uri);
                Some(PendingDatagram::new(
                    peer.to_string(),
                    outbound::build_success_response_ack(
                        &sip_response,
                        request_uri,
                        &edge_config.advertised_addr,
                        &raw_external_call_id,
                        &transaction.outbound_route_set,
                    ),
                ))
            } else {
                None
            };
            let forwarded_bytes = response::forward_response_to_inbound_with_body_and_call_id(
                &sip_response,
                &transaction.vias,
                &transaction.inbound_route_set,
                rewritten_sdp_bytes
                    .as_deref()
                    .unwrap_or(sip_response.body.as_ref()),
                call_id.as_deref(),
            );

            let mut delivered_by_server_transaction = false;
            if is_invite {
                if let (Some(ref orig_req), Ok(peer_addr)) = (
                    &transaction.original_request,
                    transaction.peer.parse::<SocketAddr>(),
                ) {
                    if let Some(key) = RequestTransactionKey::from_request(orig_req, peer_addr) {
                        if let Some(tx) = edge_state.get_server_transaction(&key) {
                            let event = if sip_response.status_code < 200 {
                                transaction::ServerTransactionEvent::UpdateLastProvisional(
                                    forwarded_bytes.clone(),
                                )
                            } else {
                                transaction::ServerTransactionEvent::Response(
                                    forwarded_bytes.clone(),
                                )
                            };
                            delivered_by_server_transaction =
                                sip_response.status_code >= 200 && tx.send(event).await.is_ok();
                        }
                    }
                }
            }

            let mut datagrams = gateway_success_ack.into_iter().collect::<Vec<_>>();
            if !delivered_by_server_transaction {
                let caller_response = PendingDatagram::new(transaction.peer, forwarded_bytes);
                let caller_response = match invite_response_order.as_ref() {
                    Some(order) => caller_response.with_invite_response_order(
                        Arc::clone(order),
                        response_cseq,
                        sip_response.status_code,
                    ),
                    None => caller_response,
                };
                datagrams.push(caller_response);
            }
            datagrams.extend(cancel_datagrams);
            datagrams
        }
        None => {
            warn!("received outbound SIP response without inbound transaction");
            Vec::new()
        }
    }
}
