use std::net::SocketAddr;
use std::sync::Arc;

use sip_core::{SipResponse, SipUri};
use tracing::{debug, warn};

mod call_manager_sync;
mod caller_response;
mod dialog_state;
mod failed_cleanup;
mod failover;
mod fork_cancel;
mod gateway_health;
mod media_negotiation;
mod message_cleanup;
mod prack;
mod pre_filter;
mod success_2xx;
mod transfer;

use crate::config::EdgeConfig;
use crate::edge_state::{DialogLegState, EdgeState, PendingDatagram};
use crate::sip::{outbound, transaction};

pub(super) async fn notify_invite_server_transaction(
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

pub(super) fn tagged_dialog_uri(uri: &SipUri, tag: Option<&str>) -> String {
    match tag {
        Some(tag) => format!("<{uri}>;tag={tag}"),
        None => format!("<{uri}>"),
    }
}

pub(super) fn build_gateway_success_ack(
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

pub(super) fn gateway_peer(dialog: &DialogLegState, response_peer: SocketAddr) -> String {
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

pub(super) fn dialog_target(dialog: &DialogLegState) -> String {
    dialog
        .route_set
        .first()
        .and_then(|route| crate::edge_state::parse_target_addr_from_route(route))
        .or_else(|| dialog.peer.clone())
        .unwrap_or_else(|| outbound::target_addr_for(&dialog.remote_target))
}

pub(super) fn build_dialog_bye(
    dialog: &mut DialogLegState,
    advertised_addr: &str,
) -> (String, Vec<u8>) {
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

    if let Some(datagrams) =
        pre_filter::try_handle_prefilter_response(&sip_response, edge_state, edge_config).await
    {
        return datagrams;
    }

    let call_id = sip_response
        .headers
        .get("call-id")
        .map(|call_id| call_id.as_str().to_string());

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

    let cancel_datagrams = fork_cancel::build_fork_cancel_datagrams(
        &sip_response,
        edge_state,
        edge_config,
        session_key,
        call_id.as_deref(),
        &gateway_call_id,
    );

    if is_transfer_response {
        return transfer::handle_transfer_response(
            &sip_response,
            peer,
            edge_state,
            edge_config,
            session_key,
            is_invite,
            response_cseq,
        )
        .await;
    }

    dialog_state::update_gateway_dialog_state(
        &sip_response,
        peer,
        edge_state,
        edge_config,
        session_key,
        call_id.as_deref(),
        is_invite,
    );
    let transaction = session_key.and_then(|session_id| {
        edge_state
            .inbound_transactions
            .get(session_id)
            .map(|r| r.clone())
    });

    let blf_datagrams = success_2xx::handle_2xx_success(
        &sip_response,
        edge_state,
        edge_config,
        session_key,
        call_id.as_deref(),
    );

    message_cleanup::cleanup_message_transaction(&sip_response, edge_state, session_key);

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

    let mut outbound_response_outcome = match call_manager_sync::sync_to_call_manager(
        &sip_response,
        edge_state,
        call_id.as_deref(),
        is_invite,
        is_reinvite_response,
    )
    .await
    {
        Some(outcome) => outcome,
        None => return Vec::new(),
    };

    gateway_health::update_gateway_health(
        &sip_response,
        edge_state,
        &outbound_response_outcome,
        is_invite,
        is_reinvite_response,
    );

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

    failed_cleanup::handle_failed_state(
        &sip_response,
        edge_state,
        edge_config,
        transaction.as_ref(),
        call_id.as_deref(),
        session_key,
        &outbound_response_outcome,
        is_reinvite_response,
    );

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

    if let Some(datagrams) = prack::try_handle_prack_response(
        &sip_response,
        peer,
        edge_state,
        edge_config,
        call_id.as_deref(),
        session_key,
        transaction.as_ref(),
        rewritten_sdp_bytes.as_deref(),
        invite_response_order.as_ref(),
        response_cseq,
        &gateway_call_id,
    )
    .await
    {
        return datagrams;
    }

    let mut datagrams = caller_response::build_caller_response(
        &sip_response,
        peer,
        edge_state,
        edge_config,
        transaction,
        call_id.as_deref(),
        session_key,
        is_invite,
        rewritten_sdp_bytes.as_deref(),
        invite_response_order.as_ref(),
        response_cseq,
        cancel_datagrams,
    )
    .await;
    // 将 BLF 通知数据报附加到响应后发送（仅 2xx INVITE 首次建立时非空）
    datagrams.extend(blf_datagrams);
    datagrams
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
