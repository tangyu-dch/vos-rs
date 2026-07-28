//! 通话监听端点：启动/停止媒体旁路监听。

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use std::net::SocketAddr;
use std::sync::Arc;

use crate::EdgeState;

#[derive(Deserialize)]
pub(super) struct MonitorCallRequest {
    supervisor_addr: String,
}

pub(super) async fn monitor_call(
    State(state): State<Arc<EdgeState>>,
    Path(call_id): Path<String>,
    Json(payload): Json<MonitorCallRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let Ok(supervisor_addr) = payload.supervisor_addr.parse::<SocketAddr>() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "status": "error",
                "message": "Invalid supervisor_addr format"
            })),
        );
    };

    let tx_opt = state.inbound_transactions.get(&call_id);
    let Some(tx) = tx_opt else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "status": "error",
                "message": "Call not found"
            })),
        );
    };

    let mut ports_monitored = Vec::new();
    if let Some(ref ep) = tx.caller_relay_rtp {
        state.media_relay.start_monitoring(ep.port, supervisor_addr);
        ports_monitored.push(ep.port);
    }
    if let Some(ref ep) = tx.gateway_relay_rtp {
        state.media_relay.start_monitoring(ep.port, supervisor_addr);
        ports_monitored.push(ep.port);
    }

    if ports_monitored.is_empty() {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "status": "error",
                "message": "Call is active but media ports are not allocated yet"
            })),
        );
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "success",
            "message": "Call monitoring started successfully",
            "call_id": call_id,
            "monitored_ports": ports_monitored,
            "supervisor_addr": supervisor_addr.to_string()
        })),
    )
}

pub(super) async fn stop_monitor_call(
    State(state): State<Arc<EdgeState>>,
    Path(call_id): Path<String>,
    Json(payload): Json<MonitorCallRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let Ok(supervisor_addr) = payload.supervisor_addr.parse::<SocketAddr>() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "status": "error",
                "message": "Invalid supervisor_addr format"
            })),
        );
    };

    let tx_opt = state.inbound_transactions.get(&call_id);
    let Some(tx) = tx_opt else {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "status": "error",
                "message": "Call not found"
            })),
        );
    };

    if let Some(ref ep) = tx.caller_relay_rtp {
        state.media_relay.stop_monitoring(ep.port, supervisor_addr);
    }
    if let Some(ref ep) = tx.gateway_relay_rtp {
        state.media_relay.stop_monitoring(ep.port, supervisor_addr);
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "success",
            "message": "Call monitoring stopped successfully",
            "call_id": call_id,
            "supervisor_addr": supervisor_addr.to_string()
        })),
    )
}
