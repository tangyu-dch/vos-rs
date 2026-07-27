use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;

use sip_core::{SipResponse, SipUri};
use tokio::sync::Mutex;
use tracing::warn;

use crate::config::EdgeConfig;
use crate::edge_state::{EdgeState, InboundTransaction, InviteResponseOrder, PendingDatagram};
use crate::sip::{outbound, response, transaction, RequestTransactionKey};

use super::{gateway_peer, tagged_dialog_uri};

/// 处理 100rel 临时响应：构建 PRACK 请求并转发给 caller。
///
/// 返回 `Some(datagrams)` 表示已处理（调用方应提前返回）；
/// 返回 `None` 表示未命中 PRACK 分支（`is_100rel` 为假或 `call_id` 为空），调用方应继续后续流程。
#[allow(clippy::too_many_arguments)]
pub(super) async fn try_handle_prack_response(
    sip_response: &SipResponse,
    peer: SocketAddr,
    edge_state: &EdgeState,
    edge_config: &EdgeConfig,
    call_id: Option<&str>,
    session_key: Option<&str>,
    transaction: Option<&InboundTransaction>,
    rewritten_sdp_bytes: Option<&[u8]>,
    invite_response_order: Option<&Arc<Mutex<InviteResponseOrder>>>,
    response_cseq: Option<u32>,
    gateway_call_id: &str,
) -> Option<Vec<PendingDatagram>> {
    let is_100rel = sip_response.status_code >= 180
        && sip_response.status_code < 200
        && sip_response
            .headers
            .get("require")
            .map(|v| v.as_str().contains("100rel"))
            .unwrap_or(false);

    if !is_100rel {
        return None;
    }

    let cid = call_id?;

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
            let from_val =
                tagged_dialog_uri(&gateway_dialog.local_uri, Some(&gateway_dialog.local_tag));
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
                gateway_call_id.to_string(),
                transaction
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
    let mut datagrams: Vec<PendingDatagram> = vec![PendingDatagram::new(target_peer, prack_bytes)];

    if let Some(t) = transaction {
        let caller_peer = t.dialogs.caller.peer.clone().unwrap_or_default();
        let peer_addr = caller_peer.parse::<SocketAddr>().ok();
        let Some(inbound_request) = t.original_request.as_deref() else {
            warn!(call_id = ?call_id, "cannot build caller response without inbound INVITE");
            return Some(datagrams);
        };
        let mut rewritten_response = response::build_inbound_leg_response(
            sip_response,
            inbound_request,
            &edge_config.advertised_addr,
            &t.dialogs.caller.local_tag,
            rewritten_sdp_bytes.unwrap_or(sip_response.body.as_ref()),
            peer_addr,
        );
        let raw_str = String::from_utf8_lossy(&rewritten_response);
        let patched =
            crate::sip::handlers::replace_header_value(&raw_str, "RSeq", &our_rseq.to_string());
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
        let caller_response = match invite_response_order {
            Some(order) => caller_response.with_invite_response_order(
                Arc::clone(order),
                response_cseq,
                sip_response.status_code,
            ),
            None => caller_response,
        };
        datagrams.push(caller_response);
    }
    Some(datagrams)
}
