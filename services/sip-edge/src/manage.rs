use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};

use call_core::ActiveCall;
use serde::Deserialize;
use sip_core::SipUri;

use crate::media::relay::MediaRelayMetrics;
use crate::EdgeState;

#[derive(Clone)]
struct ManageAuthSecret(String);

async fn internal_auth(
    State(secret): State<ManageAuthSecret>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Result<axum::response::Response, StatusCode> {
    let token = req
        .headers()
        .get("X-VOS-Token")
        .and_then(|h| h.to_str().ok());
    if let Some(t) = token {
        if t == secret.0 {
            return Ok(next.run(req).await);
        }
    }
    Err(StatusCode::UNAUTHORIZED)
}

/// 启动管理 API（活跃呼叫查询 / 强制拆线）。
pub async fn serve(addr: String, state: Arc<EdgeState>, internal_secret: String) {
    let app = Router::new()
        .route("/manage/active-calls", get(active_calls))
        .route("/manage/calls/:call_id/terminate", post(terminate))
        .route("/manage/route-preview", get(route_preview))
        .route("/manage/media-metrics", get(media_metrics))
        .route("/manage/calls/:call_id/play", post(play))
        .route("/manage/calls/:call_id/stop-play", post(stop_play))
        .route("/manage/calls/:call_id/mute", post(mute))
        .route("/manage/calls/:call_id/unmute", post(unmute))
        .route("/manage/calls/:call_id/status", get(call_status))
        .route_layer(axum::middleware::from_fn_with_state(
            ManageAuthSecret(internal_secret),
            internal_auth,
        ))
        .with_state(state)
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        );

    match tokio::net::TcpListener::bind(&addr).await {
        Ok(listener) => {
            tracing::info!(%addr, "manage API listening");
            if let Err(e) = axum::serve(listener, app).await {
                tracing::warn!(error = %e, "manage API stopped");
            }
        }
        Err(e) => {
            tracing::warn!(%addr, error = %e, "failed to bind manage API port");
        }
    }
}

async fn active_calls(State(state): State<Arc<EdgeState>>) -> Json<Vec<ActiveCall>> {
    Json(state.call_manager.active_calls())
}

/// RTP/录音聚合指标，供 API Server、压测脚本和运维面板读取。
async fn media_metrics(State(state): State<Arc<EdgeState>>) -> Json<MediaRelayMetrics> {
    Json(state.media_relay.metrics_totals())
}

async fn terminate(State(state): State<Arc<EdgeState>>, Path(call_id): Path<String>) -> StatusCode {
    // 强制挂断：同步清理并发计数和事务记录
    let username = state.inbound_transactions.get(&call_id).and_then(|tx| {
        tx.original_request
            .as_ref()
            .and_then(|req| crate::edge_state::EdgeState::username_from_request(req))
    });
    if let Some(ref uname) = username {
        state.decrement_user_concurrency(uname);
    }
    // Decrement active call count for the gateway before terminating.
    if let Some(gw_id) = state.call_manager.current_gateway_id(&call_id) {
        state
            .gateway_health
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .decrement_active(&gw_id);
    }
    state.inbound_transactions.remove(&call_id);
    state.call_manager.terminate_call(&call_id);

    // Real-time billing: settle the call on force terminate.
    if let Some(ref db) = state.db_store {
        if let Some(call) = state
            .call_manager
            .get(&call_core::CallId::new(call_id.clone()))
        {
            let caller_user = call.caller.as_deref().and_then(|s| {
                let idx = s.find("sip:")?;
                let rest = &s[idx + 4..];
                let end = rest.find(['@', ';', '>']).unwrap_or(rest.len());
                if end == 0 {
                    None
                } else {
                    Some(&rest[..end])
                }
            });
            let callee = call.inbound.remote_uri.user.as_deref().unwrap_or("");
            let duration_ms = call
                .ended_at
                .and_then(|e| e.duration_since(call.started_at).ok())
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);
            if let Some(user) = caller_user {
                let db = db.clone();
                let user = user.to_string();
                let callee = callee.to_string();
                let cid = call_id.clone();
                tokio::spawn(async move {
                    if let Err(e) = db.settle_call(&cid, &user, &callee, duration_ms).await {
                        tracing::warn!(call_id = %cid, error = %e, "force-terminate settlement failed");
                    }
                });
            }
        }
    }

    StatusCode::OK
}

#[derive(Deserialize)]
struct RoutePreviewQuery {
    destination: String,
}

/// 选路试算：返回某被叫号码的候选路由序列（failover 顺序）。
async fn route_preview(
    State(state): State<Arc<EdgeState>>,
    Query(q): Query<RoutePreviewQuery>,
) -> Json<serde_json::Value> {
    let cm = &state.call_manager;
    let routes = cm.routes();
    let uri_str = format!("sip:{}@preview.local", q.destination);
    let uri: SipUri = match uri_str.parse() {
        Ok(u) => u,
        Err(_) => {
            return Json(serde_json::json!({
                "destination": q.destination,
                "candidates": [],
                "error": "invalid destination"
            }));
        }
    };
    match routes.select_candidates(&uri) {
        Ok(candidates) => Json(serde_json::json!({
            "destination": q.destination,
            "candidates": candidates.iter().map(|c| serde_json::json!({
                "route_id": c.route_id,
                "gateway_id": c.target.gateway_id.as_str(),
                "host": c.target.host,
                "port": c.target.port,
            })).collect::<Vec<_>>()
        })),
        Err(_) => Json(serde_json::json!({
            "destination": q.destination,
            "candidates": [],
            "error": "no matching route"
        })),
    }
}

/// 播放音频请求负载
#[derive(Deserialize)]
struct PlayRequest {
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
struct ControlRequest {
    /// 目标分支: "caller" (主叫), "callee" (被叫), "both" (双方)
    leg: String,
}

/// 向指定通话分支播放音频接口
async fn play(
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
            if let Err(e) = state.media_relay.start_playback(
                rtp.port,
                file_path.clone(),
                mode,
                payload.loop_playback,
            ) {
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
            if let Err(e) =
                state
                    .media_relay
                    .start_playback(rtp.port, file_path, mode, payload.loop_playback)
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
async fn stop_play(
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

    let mut stop_caller = false;
    let mut stop_callee = false;
    match payload.leg.as_str() {
        "caller" => stop_caller = true,
        "callee" => stop_callee = true,
        "both" => {
            stop_caller = true;
            stop_callee = true;
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
async fn mute(
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

    let mut mute_caller = false;
    let mut mute_callee = false;
    match payload.leg.as_str() {
        "caller" => mute_caller = true,
        "callee" => mute_callee = true,
        "both" => {
            mute_caller = true;
            mute_callee = true;
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

    if mute_caller {
        if let Some(ref rtp) = tx.caller_relay_rtp {
            state.media_relay.muted_ports.insert(rtp.port);
        }
    }

    if mute_callee {
        if let Some(ref rtp) = tx.gateway_relay_rtp {
            state.media_relay.muted_ports.insert(rtp.port);
        }
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({"status": "success"})),
    )
}

/// 取消静音接口：恢复指定分支的声音传输
async fn unmute(
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

    let mut unmute_caller = false;
    let mut unmute_callee = false;
    match payload.leg.as_str() {
        "caller" => unmute_caller = true,
        "callee" => unmute_callee = true,
        "both" => {
            unmute_caller = true;
            unmute_callee = true;
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

    if unmute_caller {
        if let Some(ref rtp) = tx.caller_relay_rtp {
            state.media_relay.muted_ports.remove(&rtp.port);
        }
    }

    if unmute_callee {
        if let Some(ref rtp) = tx.gateway_relay_rtp {
            state.media_relay.muted_ports.remove(&rtp.port);
        }
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({"status": "success"})),
    )
}

/// 获取指定通话的媒体与控制状态
async fn call_status(
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

    // 获取主叫端状态
    if let Some(ref rtp) = tx.caller_relay_rtp {
        caller_muted = state.media_relay.muted_ports.contains(&rtp.port);
        if let Some(playback) = state.media_relay.playbacks.get(&rtp.port) {
            if let Ok(st) = playback.lock() {
                caller_playback = serde_json::json!({
                    "file_path": st.file_path.to_string_lossy(),
                    "mode": format!("{:?}", st.mode).to_lowercase(),
                    "loop_playback": st.loop_playback,
                    "progress_percentage": if st.samples.is_empty() { 0.0 } else { (st.current_sample_idx as f64 / st.samples.len() as f64) * 100.0 },
                });
            }
        }
    }

    // 获取被叫端状态
    if let Some(ref rtp) = tx.gateway_relay_rtp {
        callee_muted = state.media_relay.muted_ports.contains(&rtp.port);
        if let Some(playback) = state.media_relay.playbacks.get(&rtp.port) {
            if let Ok(st) = playback.lock() {
                callee_playback = serde_json::json!({
                    "file_path": st.file_path.to_string_lossy(),
                    "mode": format!("{:?}", st.mode).to_lowercase(),
                    "loop_playback": st.loop_playback,
                    "progress_percentage": if st.samples.is_empty() { 0.0 } else { (st.current_sample_idx as f64 / st.samples.len() as f64) * 100.0 },
                });
            }
        }
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "call_id": call_id,
            "caller": {
                "muted": caller_muted,
                "playback": caller_playback,
            },
            "callee": {
                "muted": callee_muted,
                "playback": callee_playback,
            }
        })),
    )
}
