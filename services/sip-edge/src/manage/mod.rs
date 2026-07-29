//! sip-edge 内置管理 API 服务。
//!
//! 提供活跃呼叫管理、媒体控制、会议、监控、集群运维等 HTTP 端点。
//! 所有端点（除 `/health` 外）通过 `X-VOS-Token` 头进行内部认证。

mod call_status;
mod calls;
mod cluster;
mod conference;
mod config;
mod media_control;
mod metrics;
mod monitor;
mod registrations;
mod sbc;
mod tenants;

use axum::{
    extract::State,
    http::StatusCode,
    routing::{get, post},
    Router,
};
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};

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

#[derive(Clone)]
pub(super) struct AdvertisedAddr(String);

#[derive(Clone)]
pub(super) struct ManageState {
    edge: Arc<EdgeState>,
    advertised_addr: AdvertisedAddr,
}

impl axum::extract::FromRef<ManageState> for Arc<EdgeState> {
    fn from_ref(state: &ManageState) -> Self {
        Arc::clone(&state.edge)
    }
}

impl axum::extract::FromRef<ManageState> for AdvertisedAddr {
    fn from_ref(state: &ManageState) -> Self {
        state.advertised_addr.clone()
    }
}

/// 启动管理 API（活跃呼叫查询 / 强制拆线 / 媒体控制 / 集群运维）。
pub async fn serve(
    addr: String,
    state: Arc<EdgeState>,
    internal_secret: String,
    advertised_addr: String,
) {
    let protected = Router::new()
        .route("/manage/active-calls", get(calls::active_calls))
        .route("/manage/active-calls/count", get(calls::active_calls_count))
        .route("/manage/cluster/status", get(cluster::cluster_status))
        .route("/manage/cluster/drain", post(cluster::cluster_drain))
        .route("/manage/cluster/resume", post(cluster::cluster_resume))
        .route("/manage/calls/:call_id/terminate", post(calls::terminate))
        .route("/manage/calls/:call_id/transfer", post(calls::transfer))
        .route("/manage/route-preview", get(calls::route_preview))
        .route("/manage/media-metrics", get(metrics::media_metrics))
        .route("/manage/cdr-metrics", get(metrics::cdr_metrics))
        .route(
            "/manage/config/recording",
            get(config::recording_config).put(config::reload_recording_config),
        )
        .route("/manage/calls/:call_id/play", post(media_control::play))
        .route(
            "/manage/calls/:call_id/stop-play",
            post(media_control::stop_play),
        )
        .route("/manage/calls/:call_id/mute", post(media_control::mute))
        .route("/manage/calls/:call_id/unmute", post(media_control::unmute))
        .route(
            "/manage/calls/:call_id/barge-in",
            post(media_control::barge_in),
        )
        .route("/manage/calls/:call_id/stream", post(media_control::stream))
        .route(
            "/manage/calls/:call_id/status",
            get(call_status::call_status),
        )
        .route(
            "/manage/calls/:call_id/monitor",
            post(monitor::monitor_call),
        )
        .route(
            "/manage/calls/:call_id/stop-monitor",
            post(monitor::stop_monitor_call),
        )
        .route(
            "/manage/conferences/join",
            post(conference::join_conference),
        )
        .route(
            "/manage/conferences/leave",
            post(conference::leave_conference),
        )
        .route(
            "/manage/conferences/status",
            get(conference::conference_status),
        )
        .route(
            "/manage/conferences/mute-participant",
            post(conference::mute_conference_participant),
        )
        .route("/manage/sbc/rules", post(sbc::update_sbc_rules))
        .route("/manage/tenants", get(tenants::list_tenants))
        .route("/manage/tenants/count", get(tenants::tenant_count))
        .route(
            "/manage/outbound-registrations",
            get(registrations::list_outbound_registrations),
        )
        .route("/manage/turn/status", get(metrics::turn_status))
        .route_layer(axum::middleware::from_fn_with_state(
            ManageAuthSecret(internal_secret),
            internal_auth,
        ))
        .with_state(ManageState {
            edge: state,
            advertised_addr: AdvertisedAddr(advertised_addr),
        });
    let app = Router::new()
        .route("/health", get(health))
        .merge(protected)
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

async fn health() -> &'static str {
    "ok"
}
