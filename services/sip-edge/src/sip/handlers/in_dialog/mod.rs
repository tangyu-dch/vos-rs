use std::net::SocketAddr;

use call_core::CallError;
use sip_core::{Method, SipRequest};
use tracing::info;

use super::{call_error_for_unknown_request, response_for_dialog_validation_error};
use crate::config::EdgeConfig;
use crate::edge_state::{parse_target_addr_from_route, DialogLeg, EdgeState, PendingDatagram};
use crate::sip::{outbound, response};

mod bye;
mod info;
mod refer;
mod reinvite;

/// Entry point for in-dialog SIP requests (BYE / CANCEL / INFO / PRACK / REFER /
/// re-INVITE / UPDATE / NOTIFY / MESSAGE-with-To-tag).
///
/// The function validates the request against the matched B2BUA transaction,
/// dispatches method-specific handling, then (for forwardable methods) resolves the
/// peer-leg dialog, rewrites SDP if needed, builds the outbound B2BUA request, and
/// performs post-forward cleanup for teardown methods.
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
            if bye::handle_bye_cancel_pre_forward(
                &mutable_request,
                peer,
                edge_state,
                edge_config,
                &transaction,
                &transaction_call_id,
                call_id.as_str(),
                source_leg,
                &mut datagrams,
            )
            .await
            {
                return datagrams;
            }
        }
        Method::Info => {
            info::handle_info(
                &mutable_request,
                peer,
                edge_state,
                &transaction,
                &mut datagrams,
            );
        }
        Method::Prack => {
            info::handle_prack(&mutable_request, peer, &mut datagrams);
            return datagrams;
        }
        Method::Refer => {
            return refer::handle_refer(
                &mutable_request,
                peer,
                edge_state,
                edge_config,
                &transaction,
                call_id.as_str(),
                source_leg,
            )
            .await;
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

    let is_bridged = transaction.transfer_dialog.is_some();
    let rewritten_sdp = reinvite::refresh_session_renegotiation(
        &mutable_request,
        peer,
        edge_state,
        &transaction,
        call_id.as_str(),
        is_bridged,
    );

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
        bye::forward_bye_cancel_cleanup(
            edge_state,
            edge_config,
            &transaction,
            &transaction_call_id,
            call_id.as_str(),
        );
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
