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

use crate::EdgeState;

/// 启动管理 API（活跃呼叫查询 / 强制拆线）。
pub async fn serve(addr: String, state: Arc<EdgeState>) {
    let app = Router::new()
        .route("/manage/active-calls", get(active_calls))
        .route("/manage/calls/:call_id/terminate", post(terminate))
        .route("/manage/route-preview", get(route_preview))
        .with_state(state)
        .layer(CorsLayer::new().allow_origin(Any).allow_methods(Any).allow_headers(Any));

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

async fn terminate(
    State(state): State<Arc<EdgeState>>,
    Path(call_id): Path<String>,
) -> StatusCode {
    state.call_manager.terminate_call(&call_id);
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
