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

async fn internal_auth(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Result<axum::response::Response, StatusCode> {
    let secret = match std::env::var("VOS_RS_INTERNAL_SECRET") {
        Ok(val) => val,
        Err(_) => {
            tracing::error!("VOS_RS_INTERNAL_SECRET 未配置，管理接口拒绝所有请求");
            return Err(StatusCode::UNAUTHORIZED);
        }
    };

    let token = req
        .headers()
        .get("X-VOS-Token")
        .and_then(|h| h.to_str().ok());
    if let Some(t) = token {
        if t == secret {
            return Ok(next.run(req).await);
        }
    }
    Err(StatusCode::UNAUTHORIZED)
}

/// 启动管理 API（活跃呼叫查询 / 强制拆线）。
pub async fn serve(addr: String, state: Arc<EdgeState>) {
    let app = Router::new()
        .route("/manage/active-calls", get(active_calls))
        .route("/manage/calls/:call_id/terminate", post(terminate))
        .route("/manage/route-preview", get(route_preview))
        .route("/manage/media-metrics", get(media_metrics))
        .route_layer(axum::middleware::from_fn(internal_auth))
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
