use std::net::SocketAddr;
use std::str::FromStr;
use std::time::Instant;

use sdp_core::RtpEndpoint;
use sip_core::{parse_message, HeaderName, HeaderValue, Method, SipMessage, SipUri};
use tracing::{debug, info, warn};

use crate::config::EdgeConfig;
use crate::edge_state::{EdgeState, PendingDatagram};
use crate::media;
use crate::sip::handlers::handle_request;
use crate::sip::{outbound, response, transaction, ClientTransactionKey, RequestTransactionKey};

pub(crate) async fn handle_datagram(
    packet: &[u8],
    peer: SocketAddr,
    edge_state: &EdgeState,
    edge_config: &EdgeConfig,
) -> Vec<PendingDatagram> {
    if !edge_state.sbc_engine.is_allowed(peer.ip()) {
        warn!(%peer, "packet blocked by SBC IP ACL");
        return Vec::new();
    }

    if !edge_state.sbc_engine.check_rate(peer.ip()) {
        warn!(%peer, "packet blocked by SBC rate limit");
        if let Ok(SipMessage::Request(req)) = parse_message(packet) {
            return vec![PendingDatagram::new(
                peer.to_string(),
                response::build_response_with_owned_headers(
                    &req,
                    503,
                    "Service Unavailable - Rate Limit Exceeded",
                    &[("Retry-After".to_string(), "10".to_string())],
                    "",
                ),
            )];
        }
        return Vec::new();
    }

    match parse_message(packet) {
        Ok(SipMessage::Request(request)) => {
            info!(method = %request.method, uri = %request.uri, "received SIP request");

            let transaction_key = RequestTransactionKey::from_request(&request, peer);
            let has_socket = edge_state.get_socket().is_some();
            if !has_socket {
                if let Some(ref key) = transaction_key {
                    if let Some(cached) = edge_state.test_request_cache.get(key) {
                        debug!(%peer, method = %request.method, "replaying cached test response");
                        return cached.clone();
                    }
                }
            } else if let Some(ref key) = transaction_key {
                if let Some(tx) = edge_state.get_server_transaction(key) {
                    debug!(%peer, method = %request.method, "feeding duplicate request to active Server Transaction");
                    let _ = tx
                        .send(transaction::ServerTransactionEvent::Request(
                            request.clone(),
                        ))
                        .await;
                    return Vec::new();
                }
            }

            let is_ack = matches!(&request.method, Method::Ack);
            if is_ack {
                let ack_branch = request
                    .headers
                    .get("via")
                    .and_then(|v| transaction::branch_param(v.as_str()));
                let ack_call_id = request
                    .headers
                    .get("call-id")
                    .map(|v| v.as_str().to_string());
                let ack_cseq_num = request
                    .headers
                    .get("cseq")
                    .and_then(|v| v.as_str().split_whitespace().next().map(|s| s.to_string()));
                let invite_key = RequestTransactionKey::new_manual(
                    peer.to_string(),
                    "INVITE".to_string(),
                    ack_branch,
                    ack_call_id,
                    ack_cseq_num.map(|num| format!("{} INVITE", num)),
                );
                if let Some(tx) = edge_state.get_server_transaction(&invite_key) {
                    let _ = tx
                        .send(transaction::ServerTransactionEvent::Ack(request.clone()))
                        .await;
                }
            }

            let datagrams = handle_request(request.clone(), peer, edge_state, edge_config).await;

            if is_ack {
                return datagrams;
            }

            if !has_socket {
                if let Some(ref key) = transaction_key {
                    let peer_str = peer.to_string();
                    let peer_resps: Vec<PendingDatagram> = datagrams
                        .iter()
                        .filter(|d| d.target == peer_str)
                        .cloned()
                        .collect();
                    if !peer_resps.is_empty() {
                        edge_state
                            .test_request_cache
                            .insert(key.clone(), peer_resps);
                    }
                }
                return datagrams;
            }

            let has_socket = edge_state.get_socket().is_some();
            let mut final_datagrams = Vec::new();
            if let (true, Some(key)) = (has_socket, transaction_key) {
                let peer_str = peer.to_string();
                let mut peer_resps = Vec::new();

                for datagram in datagrams {
                    if datagram.target == peer_str {
                        peer_resps.push(datagram.bytes);
                    } else {
                        final_datagrams.push(datagram);
                    }
                }

                if !peer_resps.is_empty() {
                    let is_invite = request.method == Method::Invite;
                    if is_invite {
                        let has_2xx = peer_resps.iter().any(|resp| resp.starts_with(b"SIP/2.0 2"));
                        if has_2xx {
                            for resp in peer_resps {
                                final_datagrams.push(PendingDatagram::new(peer_str.clone(), resp));
                            }
                        } else {
                            let (tx, rx) = tokio::sync::mpsc::channel(4);
                            edge_state.register_server_transaction(key.clone(), tx.clone());
                            transaction::spawn_invite_server_transaction(
                                key,
                                request,
                                peer,
                                edge_state.get_socket(),
                                rx,
                            );
                            for resp in peer_resps {
                                let _ = tx
                                    .send(transaction::ServerTransactionEvent::Response(resp))
                                    .await;
                            }
                        }
                    } else {
                        let (tx, rx) = tokio::sync::mpsc::channel(16);
                        edge_state.register_server_transaction(key.clone(), tx.clone());
                        transaction::spawn_non_invite_server_transaction(
                            key,
                            request,
                            peer,
                            edge_state.get_socket(),
                            rx,
                        );
                        for resp in peer_resps {
                            let _ = tx
                                .send(transaction::ServerTransactionEvent::Response(resp))
                                .await;
                        }
                    }
                }
            } else {
                final_datagrams = datagrams;
            }

            final_datagrams
        }
        Ok(SipMessage::Response(mut sip_response)) => {
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

            // Topology Hiding: translate external Call-ID (seen by gateway) back to internal
            // Call-ID (used by inbound_transactions map and the caller-facing leg).
            // We also patch sip_response.headers in-place so all downstream code (including
            // call_manager.handle_outbound_response) sees the internal Call-ID.
            let call_id = if let Some(ref cid) = call_id {
                if let Some(internal_cid) = edge_state.get_internal_call_id(cid) {
                    debug!(external_call_id = %cid, internal_call_id = %internal_cid, "topology hiding: translated gateway Call-ID to internal");
                    // Patch the raw response so downstream code never sees the external Call-ID.
                    if let Ok(name) = HeaderName::new("call-id") {
                        sip_response
                            .headers
                            .replace(name, HeaderValue::new(&internal_cid));
                    }
                    Some(internal_cid)
                } else {
                    call_id.clone()
                }
            } else {
                call_id.clone()
            };

            let original_call_id = if let Some(ref cid) = call_id {
                edge_state.refer_transfers.get(cid).map(|r| r.clone())
            } else {
                None
            };

            if let Some(orig_cid) = original_call_id {
                if let Some((_, mut t)) = edge_state.inbound_transactions.remove(&orig_cid) {
                    // 盲转完成：递减原始通话的用户并发计数
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
                        let mut datagrams =
                            vec![PendingDatagram::new(sub.referrer_peer.clone(), notify)];

                        if (200..300).contains(&status_code) {
                            // Successful transfer!
                            // Send BYE to the referrer to terminate the old session.
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

                            datagrams
                                .push(PendingDatagram::new(sub.referrer_peer.clone(), bye_bytes));

                            // Bridge the media! Update transferee target to point to C's media endpoint.
                            if let (Some(target_port), Some(transferee_port)) =
                                (sub.target_relay_port, sub.transferee_relay_port)
                            {
                                if let Ok(c_media_rtp) =
                                    media::parse_sdp_rtp_endpoint(&sip_response.body)
                                {
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

                                    // ALSO set target destination of C's relay port to the transferee's remote media endpoint!
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
                                        let _ = edge_state
                                            .media_relay
                                            .set_target(&target_relay_ep, &dest);
                                    }
                                }
                            }

                            // Setup bridged transfer routing fields in InboundTransaction
                            let from_header_val = sip_response
                                .headers
                                .get("from")
                                .map(|v| v.as_str().to_string());
                            let to_header_val = sip_response
                                .headers
                                .get("to")
                                .map(|v| v.as_str().to_string());
                            let contact_val = sip_response.headers.get("contact").and_then(|v| {
                                crate::edge_state::extract_uri_from_contact(v.as_str())
                            });

                            t.transfer_from_header = from_header_val;
                            t.transfer_to_header = to_header_val;
                            t.transfer_call_id = call_id.clone();
                            t.transfer_contact = contact_val;
                            t.transfer_peer = Some(peer.to_string());
                            t.transferee_is_caller = sub.transferee_relay_port.is_some()
                                && t.caller_relay_rtp.as_ref().map(|ep| ep.port)
                                    == sub.transferee_relay_port;

                            // Insert bridged transfers mapping
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

                            // Transfer failed! Restore/Rollback the original media session between referrer and transferee
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
                                // 1. Re-pair the original ports
                                edge_state
                                    .media_relay
                                    .pair_ports(transferee_relay.port, referrer_relay.port);

                                // 2. Restore remote destinations (targets)
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
                            // Final response: clean up refer transfers map and refer subscription from transaction
                            if let Some(ref cid) = call_id {
                                edge_state.refer_transfers.remove(cid);
                            }
                            t.refer_subscription = None;
                        }

                        edge_state.inbound_transactions.insert(orig_cid, t);
                        return datagrams;
                    }
                }
                return Vec::new();
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
                }
            }
            let transaction = call_id.as_deref().and_then(|call_id| {
                edge_state
                    .inbound_transactions
                    .get(call_id)
                    .map(|r| r.clone())
            });

            // Parse Session-Expires from 200 OK and store on the transaction
            if sip_response.status_code >= 200 && sip_response.status_code < 300 {
                if let Some(cid) = call_id.as_deref() {
                    let se_header = sip_response
                        .headers
                        .get("session-expires")
                        .or_else(|| sip_response.headers.get("x"))
                        .map(|v| v.as_str().to_string());
                    if let Some(se_val) = se_header {
                        // "600;refresher=uac" → parse seconds and optional refresher
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

            // Check if this is a Re-INVITE response (call already established - has caller_relay_rtp set)
            let is_reinvite_response = is_invite
                && transaction
                    .as_ref()
                    .map(|t| t.caller_relay_rtp.is_some())
                    .unwrap_or(false);

            let outbound_response_outcome = if is_invite && !is_reinvite_response {
                match edge_state
                    .call_manager
                    .handle_outbound_response(&sip_response)
                {
                    Ok(outcome) => outcome,
                    Err(error) => {
                        warn!(%error, "failed to apply outbound SIP response");
                        return Vec::new();
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
                }
            };

            // Record gateway health based on the outbound response outcome.
            if is_invite && !is_reinvite_response {
                let gateway_id = outbound_response_outcome.gateway_id.clone();
                if !gateway_id.is_empty() {
                    let mut health = edge_state
                        .gateway_health
                        .lock()
                        .unwrap_or_else(|e| e.into_inner());
                    if sip_response.status_code >= 200 && sip_response.status_code <= 299 {
                        health.record_success(&gateway_id);
                    } else if sip_response.status_code >= 400 {
                        health.record_failure(&gateway_id);
                    }

                    if let (Some(db), Some((open, failures, state_str, last_failure_at, half_open_successes, active_calls))) = (
                        edge_state.db_store.clone(),
                        health.get_gateway_status(&gateway_id),
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

            if let Some(next_uri) = outbound_response_outcome.failover_uri {
                info!(
                    call_id = ?call_id,
                    status = sip_response.status_code,
                    %next_uri,
                    "triggering gateway failover"
                );

                // Decrement old gateway, increment new gateway.
                let old_gw = &outbound_response_outcome.gateway_id;
                if !old_gw.is_empty() {
                    edge_state
                        .gateway_health
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .decrement_active(old_gw);
                }
                let new_gw = next_uri.host.clone();
                edge_state
                    .gateway_health
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .increment_active(&new_gw);

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
                    // Topology Hiding: generate a fresh external Call-ID for the failover leg.
                    let failover_internal_cid = req
                        .headers
                        .get("call-id")
                        .map(|v| v.as_str().to_string())
                        .unwrap_or_default();
                    let failover_external_cid = edge_state
                        .get_external_call_id(&failover_internal_cid)
                        .unwrap_or_else(|| failover_internal_cid.clone());
                    let bytes = outbound::build_outbound_invite_with_body_and_call_id(
                        req,
                        &next_uri,
                        &edge_config.advertised_addr,
                        sdp.body.as_slice(),
                        &failover_external_cid,
                    );
                    return vec![PendingDatagram::new(target, bytes)];
                } else {
                    warn!("could not perform failover because original request or rewritten sdp is missing");
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
                // 初始 INVITE 被网关拒绝（非 Re-INVITE）：清理事务并递减并发计数
                if !is_reinvite_response {
                    if let Some(cid) = call_id.as_deref() {
                        let username = edge_state.inbound_transactions.get(cid).and_then(|tx| {
                            tx.original_request.as_ref().and_then(|req| {
                                crate::edge_state::EdgeState::username_from_request(req)
                            })
                        });
                        if let Some(ref uname) = username {
                            edge_state.decrement_user_concurrency(uname);
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
                            // Single-pass: rewrite SDP + extract original endpoint
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
                                    if let Some(mut t_mut) =
                                        edge_state.inbound_transactions.get_mut(cid)
                                    {
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
                match crate::sip::handlers::prepare_rewritten_sdp(
                    &sip_response.headers,
                    &sip_response.body,
                    &edge_state.media_relay,
                    &edge_config.media,
                    "outbound response answer",
                ) {
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

                            if let Some(pt) = media::parse_sdp_dtmf_payload_type(&sip_response.body)
                            {
                                edge_state.media_relay.register_port_dtmf_tracking(
                                    call_id,
                                    sdp.relay_endpoint.port,
                                    pt,
                                );
                            }

                            if let Some(t) = &transaction {
                                if let Some(original_req) = &t.original_request {
                                    if let Some(pt) =
                                        media::parse_sdp_dtmf_payload_type(&original_req.body)
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

            // ── RFC 3262: 100rel intercept ────────────────────────────────────
            // If a provisional response carries `Require: 100rel` and `RSeq`,
            // sip-edge must:
            //   1. Send PRACK toward the gateway on behalf of the caller
            //   2. Rewrite RSeq with our own outbound sequence counter
            //   3. Forward the (rewritten) provisional response to the caller
            let is_100rel = sip_response.status_code >= 180
                && sip_response.status_code < 200
                && sip_response
                    .headers
                    .get("require")
                    .map(|v| v.as_str().contains("100rel"))
                    .unwrap_or(false);

            if is_100rel {
                if let Some(cid) = call_id.as_deref() {
                    // Extract RSeq from gateway response
                    let gw_rseq = sip_response
                        .headers
                        .get("rseq")
                        .and_then(|v| v.as_str().trim().parse::<u32>().ok())
                        .unwrap_or(1);

                    // Determine our own PRACK sequence number and increment the counter
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
                                    .unwrap_or_else(|| {
                                        SipUri::from_str("sip:unknown@127.0.0.1").unwrap()
                                    }),
                            )
                        }
                    };

                    // "RAck: <rseq> <cseq-number> <cseq-method>"
                    // cseq from the gateway's 1xx, rseq from the gateway's 1xx
                    let gw_cseq_num = sip_response
                        .headers
                        .get("cseq")
                        .and_then(|v| v.as_str().split_whitespace().next()?.parse::<u32>().ok())
                        .unwrap_or(1);
                    let rack_value = format!("{gw_rseq} {gw_cseq_num} INVITE");

                    // 1. Send PRACK toward the gateway
                    // Topology Hiding: gateway expects its external Call-ID, not our internal one.
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

                    // 2. Forward the 1xx to the caller with our own RSeq
                    if let Some(t) = transaction.as_ref() {
                        // Replace RSeq with our outbound counter, keep Require: 100rel
                        // Topology Hiding: override Call-ID with the internal one so caller sees its original Call-ID.
                        let mut rewritten_response =
                            response::forward_response_to_inbound_with_body_and_call_id(
                                &sip_response,
                                &t.vias,
                                &t.inbound_route_set,
                                rewritten_sdp_bytes
                                    .as_deref()
                                    .unwrap_or(sip_response.body.as_slice()),
                                call_id.as_deref(),
                            );
                        // Patch the RSeq header in the raw response bytes
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
                            if let Some(key) =
                                RequestTransactionKey::from_request(orig_req, peer_addr)
                            {
                                if let Some(tx) = edge_state.get_server_transaction(&key) {
                                    let _ = tx.send(transaction::ServerTransactionEvent::UpdateLastProvisional(rewritten_response.clone())).await;
                                }
                            }
                        }

                        datagrams.push(PendingDatagram::new(t.peer.clone(), rewritten_response));
                    }
                    return datagrams;
                }
            }

            match transaction {
                Some(transaction) => {
                    let forwarded_bytes =
                        response::forward_response_to_inbound_with_body_and_call_id(
                            &sip_response,
                            &transaction.vias,
                            &transaction.inbound_route_set,
                            rewritten_sdp_bytes
                                .as_deref()
                                .unwrap_or(sip_response.body.as_slice()),
                            call_id.as_deref(),
                        );

                    if sip_response.status_code >= 100 && sip_response.status_code < 200 {
                        if let (Some(ref orig_req), Ok(peer_addr)) = (
                            &transaction.original_request,
                            transaction.peer.parse::<SocketAddr>(),
                        ) {
                            if let Some(key) =
                                RequestTransactionKey::from_request(orig_req, peer_addr)
                            {
                                if let Some(tx) = edge_state.get_server_transaction(&key) {
                                    let _ = tx.send(transaction::ServerTransactionEvent::UpdateLastProvisional(forwarded_bytes.clone())).await;
                                }
                            }
                        }
                    }

                    vec![PendingDatagram::new(transaction.peer, forwarded_bytes)]
                }
                None => {
                    warn!("received outbound SIP response without inbound transaction");
                    Vec::new()
                }
            }
        }
        Err(error) => {
            warn!(%error, "failed to parse SIP datagram");
            Vec::new()
        }
    }
}
