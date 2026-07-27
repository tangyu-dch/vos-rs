use std::net::SocketAddr;
use std::sync::Arc;

use sip_core::SipResponse;
use tokio::sync::Mutex;
use tracing::warn;

use crate::config::EdgeConfig;
use crate::edge_state::{EdgeState, InboundTransaction, InviteResponseOrder, PendingDatagram};
use crate::sip::{response, RequestTransactionKey};

use super::{
    build_gateway_non_2xx_ack, build_gateway_success_ack, gateway_peer,
    notify_invite_server_transaction,
};

/// 构建转发给 caller 的响应数据报，处理 ACK、BYE、媒体绑定。
///
/// 该函数消费 `transaction` 与 `cancel_datagrams`，返回最终需要发送的数据报列表。
#[allow(clippy::too_many_arguments)]
pub(super) async fn build_caller_response(
    sip_response: &SipResponse,
    peer: SocketAddr,
    edge_state: &EdgeState,
    edge_config: &EdgeConfig,
    transaction: Option<InboundTransaction>,
    call_id: Option<&str>,
    session_key: Option<&str>,
    is_invite: bool,
    rewritten_sdp_bytes: Option<&[u8]>,
    invite_response_order: Option<&Arc<Mutex<InviteResponseOrder>>>,
    response_cseq: Option<u32>,
    cancel_datagrams: Vec<PendingDatagram>,
) -> Vec<PendingDatagram> {
    match transaction {
        Some(transaction) => {
            if transaction.dialogs.caller.peer.as_deref() == Some("local-originate") {
                // Originated call response: register media target and ACK 200 OK.
                if let Some(ep) = transaction.caller_relay_rtp.as_ref() {
                    let sdp_bytes = rewritten_sdp_bytes.unwrap_or(sip_response.body.as_ref());
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
                        sip_response,
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
                        let cid_str = call_id.unwrap_or("").to_string();
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
                    sip_response,
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
                sip_response,
                inbound_request,
                &edge_config.advertised_addr,
                &transaction.dialogs.caller.local_tag,
                rewritten_sdp_bytes.unwrap_or(sip_response.body.as_ref()),
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
                    build_gateway_non_2xx_ack(sip_response, &transaction.dialogs.gateway);
                datagrams.push(PendingDatagram::new(
                    gateway_peer(&transaction.dialogs.gateway, peer),
                    ack_bytes,
                ));
            }

            let caller_response = PendingDatagram::new(caller_peer, forwarded_bytes);
            let caller_response = match invite_response_order {
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
