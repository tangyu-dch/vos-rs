use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Instant;

use sdp_core::RtpEndpoint;
use sip_core::{HeaderName, HeaderValue, SipResponse, SipUri};
use tracing::{debug, warn};

mod failover;
mod media_negotiation;

use crate::config::EdgeConfig;
use crate::edge_state::{DialogLeg, DialogLegState, EdgeState, PendingDatagram};
use crate::media;
use crate::sip::{outbound, response, transaction, RequestTransactionKey};

async fn notify_invite_server_transaction(
    tx: &tokio::sync::mpsc::Sender<transaction::ServerTransactionEvent>,
    status_code: u16,
    response_bytes: Vec<u8>,
) {
    let event = if status_code >= 200 {
        transaction::ServerTransactionEvent::observe_response(response_bytes)
    } else {
        transaction::ServerTransactionEvent::UpdateLastProvisional(response_bytes)
    };
    let _ = tx.send(event).await;
}

fn tagged_dialog_uri(uri: &SipUri, tag: Option<&str>) -> String {
    match tag {
        Some(tag) => format!("<{uri}>;tag={tag}"),
        None => format!("<{uri}>"),
    }
}

fn build_gateway_success_ack(
    response: &SipResponse,
    dialog: &DialogLegState,
    advertised_addr: &str,
) -> Vec<u8> {
    let branch = format!("z9hG4bK-ack-{}", uuid::Uuid::new_v4().simple());
    let mut ack = format!(
        "ACK {request_uri} SIP/2.0\r\n\
         Via: SIP/2.0/UDP {advertised_addr};branch={branch}\r\n\
         Max-Forwards: 70\r\n",
        request_uri = dialog.remote_target,
    );
    for route in &dialog.route_set {
        ack.push_str("Route: ");
        ack.push_str(route);
        ack.push_str("\r\n");
    }
    ack.push_str("From: ");
    ack.push_str(&tagged_dialog_uri(
        &dialog.local_uri,
        Some(&dialog.local_tag),
    ));
    ack.push_str("\r\nTo: ");
    ack.push_str(&tagged_dialog_uri(
        &dialog.remote_uri,
        dialog.remote_tag.as_deref(),
    ));
    ack.push_str("\r\nCall-ID: ");
    ack.push_str(&dialog.call_id);
    ack.push_str("\r\nCSeq: ");
    ack.push_str(&dialog.local_cseq.to_string());
    ack.push_str(" ACK\r\nContent-Length: 0\r\n\r\n");

    if response
        .headers
        .get("call-id")
        .is_some_and(|value| value.as_str() != dialog.call_id)
    {
        warn!(
            dialog_call_id = %dialog.call_id,
            "refusing to borrow gateway response identity while building ACK"
        );
    }
    ack.into_bytes()
}

pub(super) fn build_gateway_non_2xx_ack(
    response: &SipResponse,
    dialog: &DialogLegState,
) -> Vec<u8> {
    let via = response
        .headers
        .get("via")
        .map(|value| value.as_str())
        .unwrap_or_default();
    let mut ack = format!(
        "ACK {request_uri} SIP/2.0\r\n\
         Via: {via}\r\n\
         Max-Forwards: 70\r\n",
        request_uri = dialog.remote_uri,
    );
    ack.push_str("From: ");
    ack.push_str(&tagged_dialog_uri(
        &dialog.local_uri,
        Some(&dialog.local_tag),
    ));
    ack.push_str("\r\nTo: ");
    ack.push_str(&tagged_dialog_uri(
        &dialog.remote_uri,
        dialog.remote_tag.as_deref(),
    ));
    ack.push_str("\r\nCall-ID: ");
    ack.push_str(&dialog.call_id);
    ack.push_str("\r\nCSeq: ");
    ack.push_str(&dialog.local_cseq.to_string());
    ack.push_str(" ACK\r\nContent-Length: 0\r\n\r\n");
    ack.into_bytes()
}

fn gateway_peer(dialog: &DialogLegState, response_peer: SocketAddr) -> String {
    if let Some(route_peer) = dialog
        .route_set
        .first()
        .and_then(|route| crate::edge_state::parse_target_addr_from_route(route))
    {
        return route_peer.to_string();
    }
    dialog
        .peer
        .clone()
        .unwrap_or_else(|| response_peer.to_string())
}

fn dialog_target(dialog: &DialogLegState) -> String {
    dialog
        .route_set
        .first()
        .and_then(|route| crate::edge_state::parse_target_addr_from_route(route))
        .or_else(|| dialog.peer.clone())
        .unwrap_or_else(|| outbound::target_addr_for(&dialog.remote_target))
}

fn build_dialog_bye(dialog: &mut DialogLegState, advertised_addr: &str) -> (String, Vec<u8>) {
    dialog.local_cseq = dialog.local_cseq.saturating_add(1);
    let branch = format!("z9hG4bK-bye-{}-{}", dialog.call_id, dialog.local_cseq);
    let mut bye = format!(
        "BYE {request_uri} SIP/2.0\r\n\
         Via: SIP/2.0/UDP {advertised_addr};branch={branch}\r\n\
         Max-Forwards: 70\r\n",
        request_uri = dialog.remote_target,
    );
    for route in &dialog.route_set {
        bye.push_str("Route: ");
        bye.push_str(route);
        bye.push_str("\r\n");
    }
    bye.push_str("From: ");
    bye.push_str(&tagged_dialog_uri(
        &dialog.local_uri,
        Some(&dialog.local_tag),
    ));
    bye.push_str("\r\nTo: ");
    bye.push_str(&tagged_dialog_uri(
        &dialog.remote_uri,
        dialog.remote_tag.as_deref(),
    ));
    bye.push_str("\r\nCall-ID: ");
    bye.push_str(&dialog.call_id);
    bye.push_str("\r\nCSeq: ");
    bye.push_str(&dialog.local_cseq.to_string());
    bye.push_str(" BYE\r\nContent-Length: 0\r\n\r\n");
    (dialog_target(dialog), bye.into_bytes())
}

pub(crate) async fn dispatch_response(
    sip_response: SipResponse,
    peer: SocketAddr,
    edge_state: &EdgeState,
    edge_config: &EdgeConfig,
) -> Vec<PendingDatagram> {
    edge_state
        .client_transactions
        .observe_response(&sip_response);

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
                let session_id = edge_state
                    .inbound_transactions
                    .get(cid)
                    .map(|transaction| transaction.session_id.clone());
                if let Some(session_id) = session_id {
                    if let Some(mut transaction) =
                        edge_state.inbound_transactions.get_mut(&session_id)
                    {
                        transaction.last_session_refresh = Some(std::time::Instant::now());
                        debug!(
                            session_id,
                            "received 200 OK for self-generated session refresh"
                        );
                    }
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

    let gateway_call_id = sip_response
        .headers
        .get("call-id")
        .map(|call_id| call_id.as_str().to_string())
        .unwrap_or_default();

    let resolved_session = call_id.as_deref().and_then(|wire_call_id| {
        edge_state
            .inbound_transactions
            .get(wire_call_id)
            .map(|transaction| {
                (
                    transaction.session_id.clone(),
                    transaction.dialogs.caller.call_id.clone(),
                )
            })
    });
    let session_id = resolved_session
        .as_ref()
        .map(|(session_id, _)| session_id.clone());
    let call_id = resolved_session
        .map(|(_, caller_call_id)| caller_call_id)
        .or(call_id);
    let session_key = session_id.as_deref().or(call_id.as_deref());
    let (is_fork_response, is_transfer_response) = session_key
        .and_then(|session_id| {
            edge_state
                .inbound_transactions
                .get(session_id)
                .map(|transaction| {
                    (
                        transaction.fork_dialogs.contains_key(&gateway_call_id),
                        transaction
                            .transfer_dialog
                            .as_ref()
                            .is_some_and(|transfer| transfer.dialog.call_id == gateway_call_id),
                    )
                })
        })
        .unwrap_or((false, false));

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
    let invite_response_order = if is_invite && !is_fork_response && !is_transfer_response {
        session_key.and_then(|session_id| {
            edge_state
                .inbound_transactions
                .get(session_id)
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

                if let Some(mut t_mut) = edge_state
                    .inbound_transactions
                    .get_mut(session_key.unwrap_or(cid))
                {
                    if !t_mut.fork_dialogs.is_empty() {
                        for (fork_cid, fork) in t_mut.fork_dialogs.iter() {
                            if fork_cid != &gateway_call_id {
                                forks_to_cancel.push((fork_cid.clone(), fork.gateway_id.clone()));
                            }
                        }
                        t_mut.fork_dialogs.clear();
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
                if let Some(mut t_mut) = edge_state
                    .inbound_transactions
                    .get_mut(session_key.unwrap_or(cid))
                {
                    if let Some(fork) = t_mut.fork_dialogs.remove(&gateway_call_id) {
                        fork_gw_to_decrement = Some(fork.gateway_id);
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

    if is_transfer_response {
        let Some(session_id) = session_key else {
            return Vec::new();
        };
        let Some((_, mut transaction)) = edge_state.inbound_transactions.remove(session_id) else {
            return Vec::new();
        };
        let Some(mut subscription) = transaction.refer_subscription.take() else {
            edge_state
                .inbound_transactions
                .insert(transaction.session_id.clone(), transaction);
            return Vec::new();
        };

        let status = sip_response.status_code;
        let Some(transfer) = transaction.transfer_dialog.as_mut() else {
            edge_state
                .inbound_transactions
                .insert(transaction.session_id.clone(), transaction);
            return Vec::new();
        };
        if status >= 180 {
            transfer.dialog.remote_tag = sip_response
                .headers
                .get("to")
                .and_then(|value| crate::sip::dialog::tag_param(value.as_str()));
            if let Some(uri) = sip_response
                .headers
                .get("from")
                .and_then(|value| crate::edge_state::extract_uri_from_contact(value.as_str()))
            {
                transfer.dialog.local_uri = uri;
            }
            if let Some(uri) = sip_response
                .headers
                .get("to")
                .and_then(|value| crate::edge_state::extract_uri_from_contact(value.as_str()))
            {
                transfer.dialog.remote_uri = uri;
            }
            if let Some(uri) = sip_response
                .headers
                .get("contact")
                .and_then(|value| crate::edge_state::extract_uri_from_contact(value.as_str()))
            {
                transfer.dialog.remote_target = uri;
            }
            let mut route_set = sip_response
                .headers
                .get_all("record-route")
                .map(|value| value.as_str().to_string())
                .collect::<Vec<_>>();
            route_set.reverse();
            transfer.dialog.route_set = route_set;
            transfer.dialog.peer = Some(peer.to_string());
            if let Some(cseq) = response_cseq {
                transfer.dialog.local_cseq = cseq;
            }
        }
        let transfer_snapshot = transfer.clone();
        let transferee_leg = transfer.transferee_leg;

        let mut datagrams = Vec::new();
        if is_invite && (200..300).contains(&status) {
            let ack = build_gateway_success_ack(
                &sip_response,
                &transfer_snapshot.dialog,
                &edge_config.advertised_addr,
            );
            datagrams.push(PendingDatagram::new(
                gateway_peer(&transfer_snapshot.dialog, peer),
                ack,
            ));
        } else if is_invite && status >= 300 {
            let ack = build_gateway_non_2xx_ack(&sip_response, &transfer_snapshot.dialog);
            datagrams.push(PendingDatagram::new(
                gateway_peer(&transfer_snapshot.dialog, peer),
                ack,
            ));
        }

        subscription.notify_cseq = subscription.notify_cseq.saturating_add(1);
        let referrer_dialog = match transferee_leg {
            DialogLeg::Caller => &transaction.dialogs.gateway,
            DialogLeg::Gateway => &transaction.dialogs.caller,
            DialogLeg::Transfer => &transaction.dialogs.caller,
        };
        let notify = outbound::build_notify_sipfrag_with_state(
            &referrer_dialog.call_id,
            &subscription.from_header,
            &subscription.to_header,
            subscription.notify_cseq,
            &edge_config.advertised_addr,
            &format!("SIP/2.0 {} {}\r\n", status, sip_response.reason_phrase),
            if status >= 200 {
                "terminated;reason=noresource"
            } else {
                "active;expires=60"
            },
        );
        datagrams.push(PendingDatagram::new(
            subscription.referrer_peer.clone(),
            notify,
        ));

        if (200..300).contains(&status) {
            if let (Some(target_port), Ok(target_endpoint)) = (
                subscription.target_relay_port,
                media::parse_sdp_rtp_endpoint(&sip_response.body),
            ) {
                let transferee_relay = match transferee_leg {
                    DialogLeg::Caller => transaction.caller_relay_rtp.clone(),
                    DialogLeg::Gateway => transaction.gateway_relay_rtp.clone(),
                    DialogLeg::Transfer => None,
                };
                let transferee_destination = match transferee_leg {
                    DialogLeg::Caller => transaction.caller_rtp.clone(),
                    DialogLeg::Gateway => transaction.gateway_rtp.clone(),
                    DialogLeg::Transfer => None,
                };
                if let Some(transferee_relay) = transferee_relay {
                    let _ = edge_state
                        .media_relay
                        .set_target(&transferee_relay, &target_endpoint);
                }
                if let Some(destination) = transferee_destination {
                    let target_relay = RtpEndpoint {
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
                        .set_target(&target_relay, &destination);
                }
            }
            let referrer_dialog = match transferee_leg {
                DialogLeg::Caller => &mut transaction.dialogs.gateway,
                DialogLeg::Gateway => &mut transaction.dialogs.caller,
                DialogLeg::Transfer => &mut transaction.dialogs.caller,
            };
            let (target, bye) = build_dialog_bye(referrer_dialog, &edge_config.advertised_addr);
            datagrams.push(PendingDatagram::new(target, bye));
        } else if status >= 300 {
            if let Some(target_port) = subscription.target_relay_port {
                edge_state.media_relay.clear_target(target_port);
            }
            if let (Some(caller_relay), Some(gateway_relay)) = (
                transaction.caller_relay_rtp.clone(),
                transaction.gateway_relay_rtp.clone(),
            ) {
                edge_state
                    .media_relay
                    .pair_ports(caller_relay.port, gateway_relay.port);
                if let Some(gateway) = transaction.gateway_rtp.clone() {
                    let _ = edge_state.media_relay.set_target(&caller_relay, &gateway);
                }
                if let Some(caller) = transaction.caller_rtp.clone() {
                    let _ = edge_state.media_relay.set_target(&gateway_relay, &caller);
                }
            }
            transaction.transfer_dialog = None;
        }

        if status < 200 {
            transaction.refer_subscription = Some(subscription);
        }
        edge_state
            .inbound_transactions
            .insert(transaction.session_id.clone(), transaction);
        return datagrams;
    }

    if let Some(call_id) = call_id.as_deref() {
        if is_invite && session_key.is_some() {
            edge_state.remember_gateway_remote_tag(session_key.unwrap_or(call_id), &sip_response);
        }
        if let Some(mut t_mut) = edge_state
            .inbound_transactions
            .get_mut(session_key.unwrap_or(call_id))
        {
            t_mut.dialogs.gateway.peer = Some(peer.to_string());
        }
        if is_invite && sip_response.status_code >= 180 && sip_response.status_code < 300 {
            if let Some(mut t_mut) = edge_state
                .inbound_transactions
                .get_mut(session_key.unwrap_or(call_id))
            {
                let mut route_set = sip_response
                    .headers
                    .get_all("record-route")
                    .map(|value| value.as_str().to_string())
                    .collect::<Vec<_>>();
                route_set.reverse();
                t_mut.dialogs.gateway.route_set = route_set;
                if let Some(local_uri) = sip_response
                    .headers
                    .get("from")
                    .and_then(|value| crate::edge_state::extract_uri_from_contact(value.as_str()))
                {
                    t_mut.dialogs.gateway.local_uri = local_uri;
                }
                if let Some(remote_uri) = sip_response
                    .headers
                    .get("to")
                    .and_then(|value| crate::edge_state::extract_uri_from_contact(value.as_str()))
                {
                    t_mut.dialogs.gateway.remote_uri = remote_uri;
                }
                if let Some(invite_cseq) = sip_response
                    .headers
                    .get("cseq")
                    .and_then(|value| crate::sip::dialog::cseq_number(value.as_str()))
                {
                    t_mut.dialogs.gateway.local_cseq = invite_cseq;
                }
                if let Some(contact_val) = sip_response.headers.get("contact") {
                    if let Some(mut uri) =
                        crate::edge_state::extract_uri_from_contact(contact_val.as_str())
                    {
                        if uri.port.is_none() {
                            uri.port = t_mut.dialogs.gateway.remote_uri.port;
                        }
                        t_mut.dialogs.gateway.remote_target = uri;
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
    let transaction = session_key.and_then(|session_id| {
        edge_state
            .inbound_transactions
            .get(session_id)
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
            if let Some(mut t_mut) = edge_state
                .inbound_transactions
                .get_mut(session_key.unwrap_or(cid))
            {
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
                    if let Some(mut t_mut) = edge_state
                        .inbound_transactions
                        .get_mut(session_key.unwrap_or(cid))
                    {
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
            if let Some(mut t_mut) = edge_state
                .inbound_transactions
                .get_mut(session_key.unwrap_or(cid))
            {
                if t_mut.established_at.is_none() {
                    t_mut.established_at = Some(Instant::now());
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
        if let Some(session_id) = session_key {
            edge_state.inbound_transactions.remove(session_id);
            debug!(session_id, "cleaned up temporary MESSAGE transaction");
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

    // The call manager still owns the logical caller-side call state. Give it a private
    // projection of the response; never rewrite the wire B-leg response in place.
    let mut call_state_response = sip_response.clone();
    if let Some(caller_call_id) = call_id.as_deref() {
        if let Ok(name) = HeaderName::new("call-id") {
            call_state_response
                .headers
                .replace(name, HeaderValue::new_owned(caller_call_id.to_string()));
        }
    }

    let mut outbound_response_outcome = if is_invite && !is_reinvite_response {
        match edge_state
            .call_manager
            .handle_outbound_response(&call_state_response)
        {
            Ok(outcome) => outcome,
            Err(error) => {
                if sip_response.status_code >= 180 && sip_response.status_code < 300 {
                    debug!(%error, status = sip_response.status_code, "response arrived when call state machine not in active state, forwarding anyway");
                } else {
                    warn!(%error, "failed to apply outbound SIP response");
                    return Vec::new();
                }
                call_core::OutboundResponseOutcome {
                    call_id: call_core::CallId::new(
                        call_id.as_deref().unwrap_or_default().to_string(),
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
            call_id: call_core::CallId::new(call_id.as_deref().unwrap_or_default().to_string()),
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
            } else if sip_response.status_code == 408
                || (sip_response.status_code >= 500 && sip_response.status_code <= 599)
            {
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
        if let Some(datagrams) = failover::handle_gateway_failover(
            edge_state,
            edge_config,
            &sip_response,
            session_key,
            &mut outbound_response_outcome,
            transaction.as_ref(),
            peer,
        )
        .await
        {
            return datagrams;
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
                let username = edge_state
                    .inbound_transactions
                    .get(session_key.unwrap_or(cid))
                    .and_then(|tx| {
                        tx.original_request.as_ref().and_then(|req| {
                            crate::edge_state::EdgeState::username_from_request(req)
                        })
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
                if let Some(session_id) = session_key {
                    edge_state.inbound_transactions.remove(session_id);
                }
            }
        }
    }

    let (rewritten_sdp_bytes, _mid_dialog_rewritten) = media_negotiation::prepare_response_sdp(
        &sip_response,
        peer,
        transaction.as_ref(),
        call_id.as_deref(),
        edge_state,
        edge_config,
    );

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

            let (our_rseq, prack_cseq, from_val, to_val, gateway_call_id, target_uri, target_peer) = {
                if let Some(mut t_mut) = edge_state
                    .inbound_transactions
                    .get_mut(session_key.unwrap_or(cid))
                {
                    t_mut.prack_rseq += 1;
                    t_mut.gateway_100rel = true;
                    let our_rseq = t_mut.prack_rseq;
                    let gateway_dialog = &t_mut.dialogs.gateway;
                    let prack_cseq = gateway_dialog.local_cseq.saturating_add(our_rseq);
                    let from_val = tagged_dialog_uri(
                        &gateway_dialog.local_uri,
                        Some(&gateway_dialog.local_tag),
                    );
                    let to_val = tagged_dialog_uri(
                        &gateway_dialog.remote_uri,
                        gateway_dialog.remote_tag.as_deref(),
                    );
                    (
                        our_rseq,
                        prack_cseq,
                        from_val,
                        to_val,
                        gateway_dialog.call_id.clone(),
                        gateway_dialog.remote_target.clone(),
                        gateway_peer(gateway_dialog, peer),
                    )
                } else {
                    (
                        1,
                        1,
                        String::new(),
                        String::new(),
                        gateway_call_id.clone(),
                        transaction
                            .as_ref()
                            .map(|t| t.dialogs.gateway.remote_target.clone())
                            .unwrap_or_else(|| SipUri::from_str("sip:unknown@127.0.0.1").unwrap()),
                        peer.to_string(),
                    )
                }
            };

            let gw_cseq_num = sip_response
                .headers
                .get("cseq")
                .and_then(|v| v.as_str().split_whitespace().next()?.parse::<u32>().ok())
                .unwrap_or(1);
            let rack_value = format!("{gw_rseq} {gw_cseq_num} INVITE");

            let prack_bytes = outbound::build_outbound_prack(
                &gateway_call_id,
                &from_val,
                &to_val,
                prack_cseq,
                &rack_value,
                &edge_config.advertised_addr,
                &target_uri,
            );
            let mut datagrams: Vec<PendingDatagram> =
                vec![PendingDatagram::new(target_peer, prack_bytes)];

            if let Some(t) = transaction.as_ref() {
                let caller_peer = t.dialogs.caller.peer.clone().unwrap_or_default();
                let peer_addr = caller_peer.parse::<SocketAddr>().ok();
                let Some(inbound_request) = t.original_request.as_deref() else {
                    warn!(call_id = ?call_id, "cannot build caller response without inbound INVITE");
                    return datagrams;
                };
                let mut rewritten_response = response::build_inbound_leg_response(
                    &sip_response,
                    inbound_request,
                    &edge_config.advertised_addr,
                    &t.dialogs.caller.local_tag,
                    rewritten_sdp_bytes
                        .as_deref()
                        .unwrap_or(sip_response.body.as_ref()),
                    peer_addr,
                );
                let raw_str = String::from_utf8_lossy(&rewritten_response);
                let patched = crate::sip::handlers::replace_header_value(
                    &raw_str,
                    "RSeq",
                    &our_rseq.to_string(),
                );
                rewritten_response = patched.into_bytes();

                if let (Some(ref orig_req), Ok(peer_addr)) =
                    (&t.original_request, caller_peer.parse::<SocketAddr>())
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

                let caller_response = PendingDatagram::new(caller_peer, rewritten_response);
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
            if transaction.dialogs.caller.peer.as_deref() == Some("local-originate") {
                // Originated call response: register media target and ACK 200 OK.
                if let Some(ep) = transaction.caller_relay_rtp.as_ref() {
                    let sdp_bytes = rewritten_sdp_bytes
                        .as_deref()
                        .unwrap_or(sip_response.body.as_ref());
                    if let Ok(remote_ep) = crate::media::sdp::parse_sdp_rtp_endpoint(sdp_bytes) {
                        if let Err(e) = edge_state.media_relay.set_target(ep, &remote_ep) {
                            tracing::warn!(error = %e, "originate: failed to set relay target");
                        }
                        if let Some(session_id) = session_key {
                            if let Some(mut t_mut) =
                                edge_state.inbound_transactions.get_mut(session_id)
                            {
                                t_mut.caller_rtp = Some(remote_ep);
                            }
                        }
                    }
                }
                let mut datagrams = Vec::new();
                if is_invite && (200..300).contains(&sip_response.status_code) {
                    let ack_bytes = build_gateway_success_ack(
                        &sip_response,
                        &transaction.dialogs.gateway,
                        &edge_config.advertised_addr,
                    );
                    datagrams.push(PendingDatagram::new(
                        gateway_peer(&transaction.dialogs.gateway, peer),
                        ack_bytes,
                    ));
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

            let gateway_success_ack = if is_invite && (200..300).contains(&sip_response.status_code)
            {
                let ack_bytes = build_gateway_success_ack(
                    &sip_response,
                    &transaction.dialogs.gateway,
                    &edge_config.advertised_addr,
                );
                Some(PendingDatagram::new(
                    gateway_peer(&transaction.dialogs.gateway, peer),
                    ack_bytes,
                ))
            } else {
                None
            };
            let caller_peer = transaction.dialogs.caller.peer.clone().unwrap_or_default();
            let peer_addr = caller_peer.parse::<SocketAddr>().ok();
            let Some(inbound_request) = transaction.original_request.as_deref() else {
                warn!(call_id = ?call_id, "cannot build caller response without inbound INVITE");
                return gateway_success_ack.into_iter().collect();
            };
            let forwarded_bytes = response::build_inbound_leg_response(
                &sip_response,
                inbound_request,
                &edge_config.advertised_addr,
                &transaction.dialogs.caller.local_tag,
                rewritten_sdp_bytes
                    .as_deref()
                    .unwrap_or(sip_response.body.as_ref()),
                peer_addr,
            );

            if is_invite {
                if let (Some(ref orig_req), Ok(peer_addr)) = (
                    &transaction.original_request,
                    caller_peer.parse::<SocketAddr>(),
                ) {
                    if let Some(key) = RequestTransactionKey::from_request(orig_req, peer_addr) {
                        if let Some(tx) = edge_state.get_server_transaction(&key) {
                            notify_invite_server_transaction(
                                &tx,
                                sip_response.status_code,
                                forwarded_bytes.clone(),
                            )
                            .await;
                        }
                    }
                }
            }

            let mut datagrams = gateway_success_ack.into_iter().collect::<Vec<_>>();

            // RFC 3261 Section 17.1.1.3: 当收到被叫发来的非 2xx (300-699) INVITE 响应时，
            // 代理/B2BUA 必须立即向被叫发送 ACK 终止其 INVITE 服务端事务，防止被叫按 Timer G 疯狂重传非 2xx 响应！
            if is_invite && sip_response.status_code >= 300 {
                let ack_bytes =
                    build_gateway_non_2xx_ack(&sip_response, &transaction.dialogs.gateway);
                datagrams.push(PendingDatagram::new(
                    gateway_peer(&transaction.dialogs.gateway, peer),
                    ack_bytes,
                ));
            }

            let caller_response = PendingDatagram::new(caller_peer, forwarded_bytes);
            let caller_response = match invite_response_order.as_ref() {
                Some(order) => caller_response.with_invite_response_order(
                    Arc::clone(order),
                    response_cseq,
                    sip_response.status_code,
                ),
                None => caller_response,
            };
            datagrams.push(caller_response);
            datagrams.extend(cancel_datagrams);
            datagrams
        }
        None => {
            warn!("received outbound SIP response without inbound transaction");
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::notify_invite_server_transaction;
    use crate::sip::transaction::ServerTransactionEvent;

    #[tokio::test]
    async fn final_response_is_observed_without_a_second_immediate_send() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(2);

        notify_invite_server_transaction(&tx, 200, b"SIP/2.0 200 OK\r\n\r\n".to_vec()).await;
        assert!(matches!(
            rx.recv().await,
            Some(ServerTransactionEvent::Response {
                send_immediately: false,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn provisional_response_remains_owned_by_transport_dispatch() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);

        notify_invite_server_transaction(&tx, 180, b"SIP/2.0 180 Ringing\r\n\r\n".to_vec()).await;
        assert!(matches!(
            rx.recv().await,
            Some(ServerTransactionEvent::UpdateLastProvisional(_))
        ));
    }
}
