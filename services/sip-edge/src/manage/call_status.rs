//! 通话状态查询端点：获取指定通话的媒体与控制状态。

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use std::sync::Arc;

use crate::EdgeState;

/// 获取指定通话的媒体与控制状态
pub(super) async fn call_status(
    State(state): State<Arc<EdgeState>>,
    Path(call_id): Path<String>,
) -> (StatusCode, Json<serde_json::Value>) {
    // 获取活跃呼叫事务
    let tx = match state.inbound_transactions.get(&call_id) {
        Some(t) => t,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "call not found"})),
            )
        }
    };

    let mut caller_playback = serde_json::json!(null);
    let mut callee_playback = serde_json::json!(null);
    let mut caller_muted = false;
    let mut callee_muted = false;
    let mut caller_talking = false;
    let mut callee_talking = false;
    let mut caller_metrics = serde_json::json!(null);
    let mut callee_metrics = serde_json::json!(null);

    // 获取主叫端状态
    if let Some(ref rtp) = tx.caller_relay_rtp {
        caller_muted = state.media_relay.muted_ports.contains(&rtp.port);
        caller_talking = state
            .media_relay
            .talking_status
            .get(&rtp.port)
            .map(|v| *v)
            .unwrap_or(false);
        caller_playback = serialize_playback(&state, rtp.port);
        caller_metrics = serialize_metrics(&state, rtp.port);
    }

    // 获取被叫端状态
    if let Some(ref rtp) = tx.gateway_relay_rtp {
        callee_muted = state.media_relay.muted_ports.contains(&rtp.port);
        callee_talking = state
            .media_relay
            .talking_status
            .get(&rtp.port)
            .map(|v| *v)
            .unwrap_or(false);
        callee_playback = serialize_playback(&state, rtp.port);
        callee_metrics = serialize_metrics(&state, rtp.port);
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "session_id": tx.session_id,
            "caller_call_id": tx.dialogs.caller.call_id,
            "gateway_call_id": tx.dialogs.gateway.call_id,
            "caller": {
                "muted": caller_muted,
                "is_talking": caller_talking,
                "playback": caller_playback,
                "metrics": caller_metrics,
            },
            "callee": {
                "muted": callee_muted,
                "is_talking": callee_talking,
                "playback": callee_playback,
                "metrics": callee_metrics,
            }
        })),
    )
}

/// 序列化指定端口的播放状态为 JSON。
fn serialize_playback(state: &EdgeState, port: u16) -> serde_json::Value {
    if let Some(playback) = state.media_relay.playbacks.get(&port) {
        if let Ok(st) = playback.lock() {
            return serde_json::json!({
                "file_path": st.file_path.to_string_lossy(),
                "mode": format!("{:?}", st.mode).to_lowercase(),
                "loop_playback": st.loop_playback,
                "progress_percentage": if st.samples.is_empty() { 0.0 } else { (st.current_sample_idx as f64 / st.samples.len() as f64) * 100.0 },
            });
        }
    }
    serde_json::json!(null)
}

/// 序列化指定端口的 RTCP/WebRTC 指标为 JSON。
fn serialize_metrics(state: &EdgeState, port: u16) -> serde_json::Value {
    if let Some(metrics) = state.media_relay.metrics.get(&port) {
        let win = metrics.rtcp_window;
        return serde_json::json!({
            "received_packets": metrics.received_packets,
            "dropped_packets": metrics.dropped_invalid_packets,
            "jitter_ms": win.average_jitter.map(|j| j as f64 / 8.0).unwrap_or(0.0),
            "loss_percent": win.average_fraction_lost.map(|l| l as f64 * 100.0 / 255.0).unwrap_or(0.0),
            "rtt_ms": win.average_rtt_ms.unwrap_or(0),
            "mos": win.mos_x100.map(|m| m as f64 / 100.0).unwrap_or(0.0),
            "webrtc": {
                "ice_connected": metrics.webrtc_ice_connected,
                "dtls_connected": metrics.webrtc_dtls_connected,
                "dtls_failed": metrics.webrtc_dtls_failed,
            }
        });
    }
    serde_json::json!(null)
}
