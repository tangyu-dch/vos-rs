//! 呼叫管理端点：活跃呼叫查询、强制拆线、路由试算。

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use call_core::ActiveCall;
use serde::Deserialize;
use sip_core::SipUri;
use std::sync::Arc;

use crate::EdgeState;

use super::AdvertisedAddr;

pub(super) async fn active_calls(State(state): State<Arc<EdgeState>>) -> Json<Vec<ActiveCall>> {
    Json(state.call_manager.active_calls())
}

pub(super) async fn active_calls_count(State(state): State<Arc<EdgeState>>) -> Json<usize> {
    Json(state.call_manager.active_calls_count())
}

pub(super) async fn terminate(
    State(state): State<Arc<EdgeState>>,
    State(AdvertisedAddr(advertised_addr)): State<AdvertisedAddr>,
    Path(call_id): Path<String>,
) -> StatusCode {
    // 管理接口接受任意一条腿的 Call-ID，但业务和媒体分别使用 A-leg ID 与 session_id。
    let Some((session_id, caller_call_id, username)) =
        state.inbound_transactions.get(&call_id).map(|transaction| {
            let username = transaction
                .original_request
                .as_ref()
                .and_then(|request| crate::edge_state::EdgeState::username_from_request(request));
            (
                transaction.session_id.clone(),
                transaction.dialogs.caller.call_id.clone(),
                username,
            )
        })
    else {
        return StatusCode::NOT_FOUND;
    };
    if let Some(ref uname) = username {
        state.decrement_user_concurrency(uname);
    }
    // Decrement active call count for the gateway before terminating.
    if let Some(gw_id) = state.call_manager.current_gateway_id(&caller_call_id) {
        state.gateway_health.decrement_active(&gw_id);
    }
    let Some(mut transaction) = state.teardown_call_transaction(&session_id) else {
        return StatusCode::NOT_FOUND;
    };
    let byes = crate::sip::dialog_request::build_session_byes(&mut transaction, &advertised_addr);
    if let Some(socket) = state.get_socket() {
        for bye in byes {
            if let Err(error) = socket.send_to(&bye.bytes, &bye.target).await {
                tracing::warn!(%error, target = %bye.target, session_id, "管理拆线发送 BYE 失败");
            }
        }
    }
    state.call_manager.terminate_call(&caller_call_id);

    crate::billing::settle_completed_call(&state, &call_core::CallId::new(caller_call_id));

    StatusCode::OK
}

#[derive(Deserialize)]
pub(super) struct RoutePreviewQuery {
    destination: String,
}

/// 选路试算：返回某被叫号码的候选路由序列（failover 顺序）。
pub(super) async fn route_preview(
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
