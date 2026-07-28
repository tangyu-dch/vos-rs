//! 媒体控制端点：音频播放/停止/静音/取消静音。

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use std::net::SocketAddr;
use std::sync::Arc;

use crate::EdgeState;

/// 播放音频请求负载
#[derive(Deserialize)]
pub(super) struct PlayRequest {
    /// 目标分支: "caller" (主叫), "callee" (被叫), "both" (双方)
    leg: String,
    /// 音频文件本地路径 (支持 8000Hz 16-bit Mono WAV 格式)
    file_path: String,
    /// 播放模式: "exclusive" (独占，会静音对端原始声音), "background" (背景混音)
    mode: String,
    /// 是否循环播放
    #[serde(default)]
    loop_playback: bool,
}

/// 静音/取消静音/停止播放通用控制请求负载
#[derive(Deserialize)]
pub(super) struct ControlRequest {
    /// 目标分支: "caller" (主叫), "callee" (被叫), "both" (双方)
    leg: String,
}

/// 向指定通话分支播放音频接口
pub(super) async fn play(
    State(state): State<Arc<EdgeState>>,
    Path(call_id): Path<String>,
    Json(payload): Json<PlayRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    // 获取当前活跃通话的事务会话信息
    let tx = match state.inbound_transactions.get(&call_id) {
        Some(t) => t,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "call not found"})),
            )
        }
    };

    // 校验并解析播放模式
    let mode = match payload.mode.as_str() {
        "exclusive" => crate::media::relay::PlaybackMode::Exclusive,
        "background" => crate::media::relay::PlaybackMode::Background,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(
                    serde_json::json!({"error": "invalid mode, must be 'exclusive' or 'background'"}),
                ),
            )
        }
    };

    let file_path = std::path::PathBuf::from(&payload.file_path);
    if !file_path.exists() {
        return (
            StatusCode::BAD_REQUEST,
            Json(
                serde_json::json!({"error": format!("file does not exist: {}", payload.file_path)}),
            ),
        );
    }

    let mut play_caller = false;
    let mut play_callee = false;
    match payload.leg.as_str() {
        "caller" => play_caller = true,
        "callee" => play_callee = true,
        "both" => {
            play_caller = true;
            play_callee = true;
        }
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(
                    serde_json::json!({"error": "invalid leg, must be 'caller', 'callee', or 'both'"}),
                ),
            )
        }
    }

    // 向主叫分支注入音频 RTP 包
    if play_caller {
        if let Some(ref rtp) = tx.caller_relay_rtp {
            if let Err(e) = state
                .media_relay
                .start_playback(rtp.port, file_path.clone(), mode, payload.loop_playback)
                .await
            {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": format!("failed to play to caller: {}", e)})),
                );
            }
        }
    }

    // 向被叫分支注入音频 RTP 包
    if play_callee {
        if let Some(ref rtp) = tx.gateway_relay_rtp {
            if let Err(e) = state
                .media_relay
                .start_playback(rtp.port, file_path, mode, payload.loop_playback)
                .await
            {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({"error": format!("failed to play to callee: {}", e)})),
                );
            }
        }
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({"status": "success"})),
    )
}

/// 停止指定通话分支音频播放接口
pub(super) async fn stop_play(
    State(state): State<Arc<EdgeState>>,
    Path(call_id): Path<String>,
    Json(payload): Json<ControlRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let tx = match state.inbound_transactions.get(&call_id) {
        Some(t) => t,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "call not found"})),
            )
        }
    };

    let (stop_caller, stop_callee) = match parse_leg(&payload.leg) {
        Ok(flags) => flags,
        Err(resp) => return resp,
    };

    if stop_caller {
        if let Some(ref rtp) = tx.caller_relay_rtp {
            state.media_relay.stop_playback(rtp.port);
        }
    }

    if stop_callee {
        if let Some(ref rtp) = tx.gateway_relay_rtp {
            state.media_relay.stop_playback(rtp.port);
        }
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({"status": "success"})),
    )
}

/// 静音接口：将指定分支的声音拦截（不转发到对端）
pub(super) async fn mute(
    State(state): State<Arc<EdgeState>>,
    Path(call_id): Path<String>,
    Json(payload): Json<ControlRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let tx = match state.inbound_transactions.get(&call_id) {
        Some(t) => t,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "call not found"})),
            )
        }
    };

    let (mute_caller, mute_callee) = match parse_leg(&payload.leg) {
        Ok(flags) => flags,
        Err(resp) => return resp,
    };

    if mute_caller {
        if let Some(ref rtp) = tx.caller_relay_rtp {
            state.media_relay.muted_ports.insert(rtp.port);
            state.media_relay.mark_relay_features_changed(rtp.port);
        }
    }

    if mute_callee {
        if let Some(ref rtp) = tx.gateway_relay_rtp {
            state.media_relay.muted_ports.insert(rtp.port);
            state.media_relay.mark_relay_features_changed(rtp.port);
        }
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({"status": "success"})),
    )
}

/// 取消静音接口：恢复指定分支的声音传输
pub(super) async fn unmute(
    State(state): State<Arc<EdgeState>>,
    Path(call_id): Path<String>,
    Json(payload): Json<ControlRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let tx = match state.inbound_transactions.get(&call_id) {
        Some(t) => t,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "call not found"})),
            )
        }
    };

    let (unmute_caller, unmute_callee) = match parse_leg(&payload.leg) {
        Ok(flags) => flags,
        Err(resp) => return resp,
    };

    if unmute_caller {
        if let Some(ref rtp) = tx.caller_relay_rtp {
            state.media_relay.muted_ports.remove(&rtp.port);
            state.media_relay.mark_relay_features_changed(rtp.port);
        }
    }

    if unmute_callee {
        if let Some(ref rtp) = tx.gateway_relay_rtp {
            state.media_relay.muted_ports.remove(&rtp.port);
            state.media_relay.mark_relay_features_changed(rtp.port);
        }
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({"status": "success"})),
    )
}

/// 解析 leg 参数，返回 (caller, callee) 标志或错误响应。
type LegResult = Result<(bool, bool), (StatusCode, Json<serde_json::Value>)>;

fn parse_leg(leg: &str) -> LegResult {
    match leg {
        "caller" => Ok((true, false)),
        "callee" => Ok((false, true)),
        "both" => Ok((true, true)),
        _ => Err((
            StatusCode::BAD_REQUEST,
            Json(
                serde_json::json!({"error": "invalid leg, must be 'caller', 'callee', or 'both'"}),
            ),
        )),
    }
}

// ===== RWI 控制端点：barge-in / stream =====

/// 强插（barge-in）请求负载。
#[derive(Deserialize)]
pub(super) struct BargeInRequest {
    /// 强插模式: "listen_and_speak" | "speak_only" | "listen_only"
    mode: String,
    /// 目标分支: "a_leg" (主叫) | "b_leg" (被叫)
    target_leg: String,
    /// 监听模式下的 supervisor RTP 接收地址（IP:Port），listen 模式必填。
    #[serde(default)]
    supervisor_addr: Option<String>,
}

/// 强插端点：根据模式启动监听和/或播报。
///
/// - `listen_only`：调用 `start_monitoring` 将目标分支 RTP 流转发到 supervisor。
/// - `speak_only`：标记 speak 模式启用（实际音频注入由独立 play 端点完成）。
/// - `listen_and_speak`：同时执行监听与标记 speak。
pub(super) async fn barge_in(
    State(state): State<Arc<EdgeState>>,
    Path(call_id): Path<String>,
    Json(payload): Json<BargeInRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let listen = matches!(payload.mode.as_str(), "listen_only" | "listen_and_speak");
    let speak = matches!(payload.mode.as_str(), "speak_only" | "listen_and_speak");
    if !listen && !speak {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "invalid mode, must be 'listen_only', 'speak_only', or 'listen_and_speak'"
            })),
        );
    }

    let port = match resolve_target_port(&state, &call_id, &payload.target_leg) {
        Ok(p) => p,
        Err(resp) => return resp,
    };

    if listen {
        if let Err(resp) = start_listen(&state, port, &payload.supervisor_addr, &call_id) {
            return resp;
        }
    }
    if speak {
        tracing::info!(%call_id, %port, mode = %payload.mode, "barge-in speak 模式已启用");
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "call_id": call_id,
            "mode": payload.mode
        })),
    )
}

/// 媒体流转发请求负载。
#[derive(Deserialize)]
pub(super) struct StreamRequest {
    /// WebSocket 接收地址（ws://host:port/path 或 wss://host:port/path）。
    stream_url: String,
    /// 音频格式: "opus" | "pcmu" | "pcma"（当前简化实现仅用于日志记录）。
    #[serde(default)]
    format: Option<String>,
}

/// 媒体流转发端点：将通话双方 RTP 流复制到指定 WebSocket URL 对应的 supervisor 地址。
///
/// 简化实现：从 `stream_url` 解析出 host:port，通过 `start_monitoring` 将
/// caller 与 callee 双方的 RTP 流转发到该地址。
pub(super) async fn stream(
    State(state): State<Arc<EdgeState>>,
    Path(call_id): Path<String>,
    Json(payload): Json<StreamRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let supervisor_addr = match parse_ws_url_to_socket_addr(&payload.stream_url).await {
        Some(addr) => addr,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": "invalid stream_url, expected ws://host:port/path or wss://host:port/path"
                })),
            )
        }
    };

    let tx = match state.inbound_transactions.get(&call_id) {
        Some(t) => t,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "call not found", "call_id": call_id})),
            )
        }
    };

    let mut ports_streamed = Vec::new();
    if let Some(ref ep) = tx.caller_relay_rtp {
        state.media_relay.start_monitoring(ep.port, supervisor_addr);
        ports_streamed.push(ep.port);
    }
    if let Some(ref ep) = tx.gateway_relay_rtp {
        state.media_relay.start_monitoring(ep.port, supervisor_addr);
        ports_streamed.push(ep.port);
    }

    if ports_streamed.is_empty() {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "media port not allocated",
                "call_id": call_id
            })),
        );
    }

    tracing::info!(
        %call_id,
        %supervisor_addr,
        ports = ?ports_streamed,
        format = ?payload.format,
        "stream 转发已启动"
    );

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "call_id": call_id,
            "stream_url": payload.stream_url
        })),
    )
}

/// 根据目标分支解析通话对应的媒体中继端口。
fn resolve_target_port(
    state: &EdgeState,
    call_id: &str,
    target_leg: &str,
) -> Result<u16, (StatusCode, Json<serde_json::Value>)> {
    let tx = match state.inbound_transactions.get(call_id) {
        Some(t) => t,
        None => {
            return Err((
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "call not found", "call_id": call_id})),
            ))
        }
    };
    let port = match target_leg {
        "a_leg" => tx.caller_relay_rtp.as_ref().map(|ep| ep.port),
        "b_leg" => tx.gateway_relay_rtp.as_ref().map(|ep| ep.port),
        _ => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(
                    serde_json::json!({"error": "invalid target_leg, must be 'a_leg' or 'b_leg'"}),
                ),
            ))
        }
    };
    port.ok_or((
        StatusCode::CONFLICT,
        Json(serde_json::json!({"error": "media port not allocated", "call_id": call_id})),
    ))
}

/// 启动监听：解析 supervisor_addr 并调用 start_monitoring。
fn start_listen(
    state: &EdgeState,
    port: u16,
    supervisor_addr: &Option<String>,
    call_id: &str,
) -> Result<(), (StatusCode, Json<serde_json::Value>)> {
    let Some(addr_str) = supervisor_addr else {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "supervisor_addr is required for listen mode",
                "call_id": call_id
            })),
        ));
    };
    let Ok(supervisor) = addr_str.parse::<SocketAddr>() else {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "invalid supervisor_addr format, expected IP:Port",
                "call_id": call_id
            })),
        ));
    };
    state.media_relay.start_monitoring(port, supervisor);
    Ok(())
}

/// 从 WebSocket URL 解析出 supervisor 的 SocketAddr。
///
/// 支持格式：`ws://host:port/path`、`wss://host:port/path`。
/// 先尝试直接解析 IP:Port，失败则进行异步 DNS 解析。
async fn parse_ws_url_to_socket_addr(url: &str) -> Option<SocketAddr> {
    let rest = url
        .strip_prefix("ws://")
        .or_else(|| url.strip_prefix("wss://"))?;
    let authority = rest.split(['/', '?']).next()?;
    let host_port = if authority.contains(':') {
        authority.to_string()
    } else {
        format!("{authority}:80")
    };
    if let Ok(addr) = host_port.parse::<SocketAddr>() {
        return Some(addr);
    }
    match tokio::net::lookup_host(host_port).await {
        Ok(mut iter) => iter.next(),
        Err(_) => None,
    }
}
