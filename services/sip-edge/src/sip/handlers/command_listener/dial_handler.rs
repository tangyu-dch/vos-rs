//! VCI Dial 命令处理。
//!
//! 唤醒 parked 呼叫：解析目标 URI、改写 SDP、绑定 gateway dialog，
//! 并通过 outbound 通道发送 INVITE，同时更新网关健康指标。

use std::str::FromStr;
use std::sync::Arc;

use sip_core::SipUri;
use tracing::{error, info, warn};

use crate::config::EdgeConfig;
use crate::edge_state::{EdgeState, PendingDatagram};
use crate::sip::handlers::{
    prepare_rewritten_sdp, register_relay_target, response_for_media_error,
};
use crate::sip::outbound;

use super::commands::DialParams;

/// 处理 Dial 命令：将 parked 呼叫转换为 B2BUA 出局 INVITE。
pub(super) async fn handle_dial(
    call_id: &str,
    params: DialParams,
    edge_state: &Arc<EdgeState>,
    edge_config: &EdgeConfig,
    socket: &Arc<tokio::net::UdpSocket>,
) {
    info!(call_id, "VCI Dial command execution started");

    let parked = match edge_state.parked_calls.remove(call_id) {
        Some((_, p)) => p,
        None => {
            warn!(call_id, "parked call not found for dial command");
            return;
        }
    };

    let callee = parked
        .invite_request
        .uri
        .user
        .as_deref()
        .unwrap_or("")
        .to_string();
    let outbound_uri = if let Some(ref uri_str) = params.target_uri {
        SipUri::from_str(uri_str).unwrap_or_else(|_| parked.invite_request.uri.clone())
    } else if let Some(ref gw_id) = params.target_gateway {
        let gw_addr = edge_state
            .gateway_target(gw_id)
            .unwrap_or_else(|| edge_config.default_gateway.clone());
        SipUri::from_str(&format!("sip:{}@{}", callee, gw_addr))
            .unwrap_or_else(|_| parked.invite_request.uri.clone())
    } else {
        parked.invite_request.uri.clone()
    };

    let rewritten_sdp = match prepare_rewritten_sdp(
        &parked.invite_request.headers,
        &parked.invite_request.body,
        &edge_state.media_relay,
        &edge_config.media,
        "inbound INVITE offer via VCI dial",
        &parked.session_id,
    ) {
        Ok(sdp) => sdp,
        Err(error) => {
            warn!(call_id, %error, "failed to rewrite SDP for VCI dial");
            let err_resp = response_for_media_error(&parked.invite_request, &error);
            let datagram = PendingDatagram::new(parked.peer_addr.to_string(), err_resp);
            let _ = edge_state
                .send_sip_datagram(datagram, socket, edge_config)
                .await;
            return;
        }
    };

    if let Some(ref rewritten_sdp) = rewritten_sdp {
        if let Some(caller_rtp) = &rewritten_sdp.original_endpoint {
            register_relay_target(
                &edge_state.media_relay,
                &rewritten_sdp.relay_endpoint,
                caller_rtp,
                "gateway-to-caller RTP via VCI",
            );
        }
    }

    let gateway_id = params.target_gateway.unwrap_or_default();
    let session_id = parked.session_id.clone();

    edge_state.remember_inbound_invite(
        session_id.clone(),
        &parked.invite_request,
        parked.peer_addr,
        outbound_uri.clone(),
        rewritten_sdp
            .as_ref()
            .and_then(|sdp| sdp.original_endpoint.clone()),
        rewritten_sdp.as_ref().map(|sdp| sdp.relay_endpoint.clone()),
        params.timeout_secs,
    );

    let external_call_id = uuid::Uuid::new_v4().to_string();
    edge_state.bind_gateway_dialog(&session_id, &external_call_id);

    let target_addr = if let Some(port) = outbound_uri.port {
        format!("{}:{}", outbound_uri.host, port)
    } else {
        format!("{}:5060", outbound_uri.host)
    };

    let caller_identity = params
        .caller_id
        .as_ref()
        .map(|num| call_core::CallerIdentity {
            original_number: num.clone(),
            presented_number: num.clone(),
            owner_gateway_id: call_core::GatewayId::new(gateway_id.clone()),
            mode: call_core::CallerIdentityMode::Fixed,
            max_concurrent: 0,
        });

    let Some(gateway_local_tag) = edge_state
        .inbound_transactions
        .get(&session_id)
        .map(|transaction| transaction.dialogs.gateway.local_tag.clone())
    else {
        warn!(
            call_id,
            "VCI Dial session disappeared before outbound INVITE"
        );
        return;
    };
    let outbound_invite_bytes = outbound::build_b2bua_outbound_invite(
        &parked.invite_request,
        &outbound_uri,
        &edge_config.advertised_addr,
        rewritten_sdp
            .as_ref()
            .map(|sdp| sdp.body.as_slice())
            .unwrap_or(parked.invite_request.body.as_ref()),
        edge_config.session_expires_gateway,
        &[],
        &external_call_id,
        &gateway_local_tag,
        caller_identity.as_ref(),
    );

    let datagram = PendingDatagram::new(target_addr, outbound_invite_bytes);
    if let Err(e) = edge_state
        .send_sip_datagram(datagram, socket, edge_config)
        .await
    {
        error!(
            call_id,
            error = %e,
            "failed to send outbound INVITE datagram for VCI Dial"
        );
    }

    if !gateway_id.is_empty() {
        edge_state.gateway_health.increment_active(&gateway_id);
        let status = edge_state.gateway_health.get_gateway_status(&gateway_id);
        crate::timers::persist_gateway_health(edge_state, gateway_id, status);
    }
}
