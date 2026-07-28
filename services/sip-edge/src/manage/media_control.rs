//! 媒体控制端点：音频播放/停止/静音/取消静音。

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
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
