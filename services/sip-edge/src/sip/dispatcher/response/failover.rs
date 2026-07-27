use tracing::{info, warn};

use crate::config::EdgeConfig;
use crate::edge_state::{EdgeState, PendingDatagram};
use crate::sip::outbound;
use sip_core::SipResponse;

/// 处理呼叫 Failover 逻辑：当从某个网关收到非 2xx 响应时，如果配置了重试网关，重新生成 INVITE 请求发往下一个网关
pub(crate) async fn handle_gateway_failover(
    edge_state: &EdgeState,
    edge_config: &EdgeConfig,
    sip_response: &SipResponse,
    session_id: Option<&str>,
    outbound_response_outcome: &mut call_core::OutboundResponseOutcome,
    transaction: Option<&crate::edge_state::InboundTransaction>,
    peer: std::net::SocketAddr,
) -> Option<Vec<PendingDatagram>> {
    let next_uri = outbound_response_outcome.failover_uri.clone()?;

    info!(
        session_id,
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

    if let Some(transaction) = transaction {
        edge_state.clear_media_targets(transaction);
    }

    let original_request = transaction.and_then(|t| t.original_request.as_ref());
    let rewritten_sdp = if let Some(req) = original_request {
        match crate::sip::handlers::prepare_rewritten_sdp(
            &req.headers,
            &req.body,
            &edge_state.media_relay,
            &edge_config.media,
            "failover INVITE offer",
            session_id.unwrap_or(""),
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

    if let (Some(session_id), Some(sdp)) = (session_id, rewritten_sdp.as_ref()) {
        if let Some(caller_rtp) = &sdp.original_endpoint {
            crate::sip::handlers::register_relay_target(
                &edge_state.media_relay,
                &sdp.relay_endpoint,
                caller_rtp,
                "gateway-to-caller RTP (failover)",
            );
        }

        if let Some(mut t_mut) = edge_state.inbound_transactions.get_mut(session_id) {
            let target = outbound::target_addr_for(&next_uri);
            t_mut.dialogs.gateway.remote_uri = next_uri.clone();
            t_mut.dialogs.gateway.remote_target = next_uri.clone();
            t_mut.dialogs.gateway.remote_tag = None;
            t_mut.dialogs.gateway.route_set.clear();
            t_mut.dialogs.gateway.peer = Some(target.to_string());
            t_mut.gateway_relay_rtp = Some(sdp.relay_endpoint.clone());
            t_mut.caller_rtp = sdp.original_endpoint.clone();
            t_mut.gateway_rtp = None;
            t_mut.caller_relay_rtp = None;
        }
    }

    let mut datagrams = Vec::new();

    if let Some(t) = transaction {
        let ack_bytes = super::build_gateway_non_2xx_ack(sip_response, &t.dialogs.gateway);
        let target = t
            .dialogs
            .gateway
            .peer
            .clone()
            .unwrap_or_else(|| peer.to_string());
        datagrams.push(PendingDatagram::new(target, ack_bytes));
    }

    if let (Some(req), Some(sdp)) = (original_request, rewritten_sdp) {
        let target = outbound::target_addr_for(&next_uri);
        let (gateway_call_id, gateway_local_tag) = transaction
            .map(|transaction| {
                (
                    transaction.dialogs.gateway.call_id.clone(),
                    transaction.dialogs.gateway.local_tag.clone(),
                )
            })
            .unwrap_or_else(|| {
                (
                    uuid::Uuid::new_v4().to_string(),
                    format!("vosrs-b-{}", uuid::Uuid::new_v4().simple()),
                )
            });
        let bytes = outbound::build_b2bua_outbound_invite(
            req,
            &next_uri,
            &edge_config.advertised_addr,
            sdp.body.as_slice(),
            edge_config.session_expires_gateway,
            &[],
            &gateway_call_id,
            &gateway_local_tag,
            outbound_response_outcome.caller_identity.as_ref(),
        );
        datagrams.push(PendingDatagram::new(target, bytes));
    } else {
        warn!("could not perform failover because original request or rewritten sdp is missing");
    }

    Some(datagrams)
}
