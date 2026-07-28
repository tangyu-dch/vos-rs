//! VCI 媒体类命令处理：Play / Gather / Stream。
//!
//! 三者共享"parked 呼叫应答 + 编解码协商 + 端口注册"的初始化逻辑，
//! 仅在交互行为上分化：
//! - Play：单向放音
//! - Gather：放音 + DTMF 收集 + NATS 回传
//! - Stream：WebSocket 双向音频流

use std::str::FromStr;
use std::sync::Arc;

use tracing::{info, warn};

use crate::config::EdgeConfig;
use crate::edge_state::{EdgeState, PendingDatagram};
use crate::sip::response;

use super::commands::{GatherParams, PlayParams, StreamParams};

/// 处理 Play 命令：应答 parked 呼叫并启动单向放音。
pub(super) async fn handle_play(
    call_id: &str,
    params: PlayParams,
    edge_state: &Arc<EdgeState>,
    edge_config: &EdgeConfig,
    socket: &Arc<tokio::net::UdpSocket>,
) {
    info!(call_id, "VCI Play command execution started");

    let parked = match edge_state.parked_calls.get(call_id) {
        Some(p) => p.value().clone(),
        None => {
            warn!(call_id, "parked call not found for play command");
            return;
        }
    };

    let codec = crate::media::sdp::negotiated_audio_codec(&parked.invite_request.body)
        .unwrap_or(rtp_core::AudioCodec::Pcma);
    let pt = codec.static_payload_type().unwrap_or(8);
    let codec_name = match codec {
        rtp_core::AudioCodec::Pcmu => "PCMU",
        _ => "PCMA",
    };

    let sdp_answer = format!(
        "v=0\r\n\
         o=vos-rs 123456 123456 IN IP4 {addr}\r\n\
         s=vos-rs-playback\r\n\
         c=IN IP4 {addr}\r\n\
         t=0 0\r\n\
         m=audio {port} RTP/AVP {pt}\r\n\
         a=rtpmap:{pt} {codec_name}/8000\r\n\
         a=sendrecv\r\n",
        addr = edge_config.media.advertised_addr,
        port = parked.caller_relay_port,
    );

    let resp = response::build_response_with_owned_headers(
        &parked.invite_request,
        200,
        "OK",
        &[
            ("Content-Type".to_string(), "application/sdp".to_string()),
            (
                "Contact".to_string(),
                format!("<sip:vosrs-playback@{}>", edge_config.advertised_addr),
            ),
        ],
        &sdp_answer,
    );

    let dg = PendingDatagram::new(parked.peer_addr.to_string(), resp);
    let _ = edge_state.send_sip_datagram(dg, socket, edge_config).await;

    edge_state.parked_calls.remove(call_id);

    edge_state
        .media_relay
        .register_port_codec(parked.caller_relay_port, codec);

    let file_path = params.url.clone();
    let loop_playback = params.loop_count.unwrap_or(1) > 1;
    let _ = edge_state
        .media_relay
        .start_playback(
            parked.caller_relay_port,
            std::path::PathBuf::from(file_path),
            crate::media::relay::PlaybackMode::Exclusive,
            loop_playback,
        )
        .await;
}

/// 处理 Gather 命令：放音 + 收集 DTMF，收集完成后通过 NATS 上报。
pub(super) async fn handle_gather(
    call_id: &str,
    params: GatherParams,
    edge_state: &Arc<EdgeState>,
    edge_config: &EdgeConfig,
    socket: &Arc<tokio::net::UdpSocket>,
) {
    info!(call_id, "VCI Gather command execution started");

    let parked = match edge_state.parked_calls.get(call_id) {
        Some(p) => p.value().clone(),
        None => {
            warn!(call_id, "parked call not found for gather command");
            return;
        }
    };

    let codec = crate::media::sdp::negotiated_audio_codec(&parked.invite_request.body)
        .unwrap_or(rtp_core::AudioCodec::Pcma);
    let pt = codec.static_payload_type().unwrap_or(8);
    let codec_name = match codec {
        rtp_core::AudioCodec::Pcmu => "PCMU",
        _ => "PCMA",
    };

    let sdp_answer = format!(
        "v=0\r\n\
         o=vos-rs 123456 123456 IN IP4 {addr}\r\n\
         s=vos-rs-gather\r\n\
         c=IN IP4 {addr}\r\n\
         t=0 0\r\n\
         m=audio {port} RTP/AVP {pt} 101\r\n\
         a=rtpmap:{pt} {codec_name}/8000\r\n\
         a=rtpmap:101 telephone-event/8000\r\n\
         a=fmtp:101 0-15\r\n\
         a=sendrecv\r\n",
        addr = edge_config.media.advertised_addr,
        port = parked.caller_relay_port,
    );

    let resp = response::build_response_with_owned_headers(
        &parked.invite_request,
        200,
        "OK",
        &[
            ("Content-Type".to_string(), "application/sdp".to_string()),
            (
                "Contact".to_string(),
                format!("<sip:vosrs-gather@{}>", edge_config.advertised_addr),
            ),
        ],
        &sdp_answer,
    );

    let dg = PendingDatagram::new(parked.peer_addr.to_string(), resp);
    let _ = edge_state.send_sip_datagram(dg, socket, edge_config).await;

    edge_state.parked_calls.remove(call_id);

    edge_state
        .media_relay
        .register_port_codec(parked.caller_relay_port, codec);

    edge_state.media_relay.register_port_dtmf_tracking(
        &parked.session_id,
        parked.caller_relay_port,
        101,
    );

    if let Some(ref play_url) = params.play_url {
        let _ = edge_state
            .media_relay
            .start_playback(
                parked.caller_relay_port,
                std::path::PathBuf::from(play_url.clone()),
                crate::media::relay::PlaybackMode::Exclusive,
                false,
            )
            .await;
    }

    let edge_state_clone = edge_state.clone();
    let caller_relay_port = parked.caller_relay_port;
    let max_digits = params.max_digits;
    let timeout_ms = params.timeout_ms;
    let dtmf_subject = edge_config.webhooks.control_dtmf_subject.clone();
    let media_session_id = parked.session_id.clone();
    let call_id_owned = call_id.to_string();

    tokio::spawn(async move {
        let start = std::time::Instant::now();
        let interval = std::time::Duration::from_millis(100);
        let mut gathered = String::new();

        while start.elapsed().as_millis() < timeout_ms as u128 {
            tokio::time::sleep(interval).await;
            if let Some(digits) = edge_state_clone
                .media_relay
                .get_dtmf_digits(&media_session_id)
            {
                gathered = digits.clone();
                if gathered.len() >= max_digits {
                    break;
                }
            }
        }

        edge_state_clone
            .media_relay
            .stop_playback(caller_relay_port);

        if let Some(nats) = edge_state_clone.nats_connection() {
            let dtmf_event = serde_json::json!({
                "call_id": call_id_owned,
                "digits": gathered,
                "status": "success"
            });
            let payload = serde_json::to_vec(&dtmf_event).unwrap_or_default();
            if let Err(e) = nats.publish(dtmf_subject, payload.into()).await {
                warn!("failed to publish DTMF digits back to NATS: {:?}", e);
            }
        }
    });
}

/// 处理 Stream 命令：建立 WebSocket 双向音频流。
///
/// 兼容两种入口：
/// - parked 呼叫：返回 200 OK 应答并注册事务，便于后续 BYE 拆线
/// - 已建立通话：复用现有 caller_relay_rtp 端口
pub(super) async fn handle_stream(
    call_id: &str,
    params: StreamParams,
    edge_state: &Arc<EdgeState>,
    edge_config: &EdgeConfig,
    socket: &Arc<tokio::net::UdpSocket>,
) {
    info!(call_id, "VCI Stream command execution started");

    // 1. Check if the call is parked (first command after INVITE)
    let port = if let Some(parked) = edge_state
        .parked_calls
        .get(call_id)
        .map(|p| p.value().clone())
    {
        let codec = crate::media::sdp::negotiated_audio_codec(&parked.invite_request.body)
            .unwrap_or(rtp_core::AudioCodec::Pcma);
        let pt = codec.static_payload_type().unwrap_or(8);
        let codec_name = match codec {
            rtp_core::AudioCodec::Pcmu => "PCMU",
            _ => "PCMA",
        };

        let sdp_answer = format!(
            "v=0\r\n\
             o=vos-rs 123456 123456 IN IP4 {addr}\r\n\
             s=vos-rs-stream\r\n\
             c=IN IP4 {addr}\r\n\
             t=0 0\r\n\
             m=audio {port} RTP/AVP {pt}\r\n\
             a=rtpmap:{pt} {codec_name}/8000\r\n\
             a=sendrecv\r\n",
            addr = edge_config.media.advertised_addr,
            port = parked.caller_relay_port,
        );

        let resp = response::build_response_with_owned_headers(
            &parked.invite_request,
            200,
            "OK",
            &[
                ("Content-Type".to_string(), "application/sdp".to_string()),
                (
                    "Contact".to_string(),
                    format!("<sip:vosrs-stream@{}>", edge_config.advertised_addr),
                ),
            ],
            &sdp_answer,
        );

        let dg = PendingDatagram::new(parked.peer_addr.to_string(), resp);
        let _ = edge_state.send_sip_datagram(dg, socket, edge_config).await;

        edge_state.parked_calls.remove(call_id);
        edge_state
            .media_relay
            .register_port_codec(parked.caller_relay_port, codec);

        // Register transaction so BYEs work
        let client_ep = crate::media::sdp::parse_sdp_rtp_endpoint(&parked.invite_request.body).ok();
        let local_ep = sdp_core::RtpEndpoint {
            address: edge_config.media.advertised_addr.clone(),
            port: parked.caller_relay_port,
        };
        edge_state.remember_inbound_invite(
            parked.session_id,
            &parked.invite_request,
            parked.peer_addr,
            sip_core::SipUri::from_str(&format!(
                "sip:vosrs-stream@{}",
                edge_config.advertised_addr
            ))
            .unwrap(),
            client_ep,
            Some(local_ep.clone()),
            None,
        );
        if let Some(mut tx) = edge_state.inbound_transactions.get_mut(call_id) {
            tx.caller_relay_rtp = Some(local_ep);
            tx.dialogs.caller.local_tag = "vosrs-stream-tag".to_string();
        }

        parked.caller_relay_port
    } else if let Some(tx) = edge_state.inbound_transactions.get(call_id) {
        // If the call is already active, find its media port
        if let Some(ref local_ep) = tx.caller_relay_rtp {
            local_ep.port
        } else {
            warn!(
                call_id,
                "active call has no caller relay port, cannot start stream"
            );
            return;
        }
    } else {
        warn!(call_id, "call not found for stream command");
        return;
    };

    // 2. Start WebSocket audio stream
    let _ = edge_state
        .media_relay
        .start_stream(port, params.websocket_url, params.format, params.barge_in)
        .await;
}
