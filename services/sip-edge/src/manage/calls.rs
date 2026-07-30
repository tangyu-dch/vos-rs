//! 呼叫管理端点：活跃呼叫查询、强制拆线、路由试算。

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use call_core::ActiveCall;
use serde::Deserialize;
use sip_core::{HeaderName, HeaderValue, Method, SipUri};
use std::sync::Arc;

use crate::sip::dialog_request::build_dialog_request;
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
    let Some((session_id, caller_call_id, username, tenant_ctx)) =
        state.inbound_transactions.get(&call_id).map(|transaction| {
            let username = transaction
                .original_request
                .as_ref()
                .and_then(|request| crate::edge_state::EdgeState::username_from_request(request));
            (
                transaction.session_id.clone(),
                transaction.dialogs.caller.call_id.clone(),
                username,
                transaction.tenant.clone(),
            )
        })
    else {
        return StatusCode::NOT_FOUND;
    };
    if let Some(ref uname) = username {
        state.decrement_user_concurrency(uname);
    }
    state.decrement_tenant_concurrency(tenant_ctx.as_ref());
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
    access_trunk_id: Option<String>,
    source_type: Option<String>,
    source_id: Option<String>,
    caller_number: Option<String>,
}

/// 选路试算：返回某被叫号码的候选路由序列（failover 顺序），若指定呼叫来源（中继/分机/分机组）或主叫号码则联动试算主叫号码与落地。
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

    let mut selected_caller_number: Option<String> = None;
    let mut caller_pool_id: Option<String> = None;

    // 解析呼叫来源：优先取指定 source_type + source_id，其次兼容 access_trunk_id，再次根据 caller_number 反查分机/中继
    let source = if let (Some(ref stype), Some(ref sid)) = (&q.source_type, &q.source_id) {
        if !stype.trim().is_empty() && !sid.trim().is_empty() {
            Some(call_core::CallSource::new(stype.trim(), sid.trim()))
        } else {
            None
        }
    } else if let Some(ref access_id) = q.access_trunk_id {
        if !access_id.trim().is_empty() {
            Some(call_core::CallSource::trunk(access_id.trim()))
        } else {
            None
        }
    } else if let Some(ref caller) = q.caller_number {
        if !caller.trim().is_empty() {
            // 分机呼出：如果传入了分机号码（如 1001），对应 source_type = "extension", source_id = "1001"
            Some(call_core::CallSource::new("extension", caller.trim()))
        } else {
            None
        }
    } else {
        None
    };

    if let Some(ref src) = source {
        let directory = cm.outbound_policies();
        if let Some((pool_id, selected)) = directory.preview_caller_selection(src) {
            caller_pool_id = pool_id;
            selected_caller_number = Some(selected);
        }
    }

    match routes.select_candidates(&uri) {
        Ok(candidates) => Json(serde_json::json!({
            "destination": q.destination,
            "access_trunk_id": q.access_trunk_id,
            "selected_caller_number": selected_caller_number,
            "caller_pool_id": caller_pool_id,
            "candidates": candidates.iter().map(|c| serde_json::json!({
                "route_id": c.route_id,
                "gateway_id": c.target.gateway_id.as_str(),
                "host": c.target.host,
                "port": c.target.port,
            })).collect::<Vec<_>>()
        })),
        Err(_) => Json(serde_json::json!({
            "destination": q.destination,
            "access_trunk_id": q.access_trunk_id,
            "selected_caller_number": selected_caller_number,
            "caller_pool_id": caller_pool_id,
            "candidates": [],
            "error": "no matching route"
        })),
    }
}

// ===== RWI 控制端点：transfer =====

/// 呼叫转接请求负载。
#[derive(Deserialize)]
pub(super) struct TransferRequest {
    /// 转接目标 SIP URI，例如 "sip:1001@example.com"。
    target: String,
    /// 转接类型: "blind" | "attended"（attended 暂未实现，返回 501）。
    #[serde(default)]
    transfer_type: Option<String>,
}

/// 呼叫转接端点：对指定通话发起 SIP REFER 转接。
///
/// - blind transfer：构造 REFER 请求发送到 call_id 对应的 dialog leg，
///   Refer-To 头指向目标 URI。
/// - attended transfer：暂未实现，返回 501 Not Implemented。
pub(super) async fn transfer(
    State(state): State<Arc<EdgeState>>,
    State(AdvertisedAddr(advertised_addr)): State<AdvertisedAddr>,
    Path(call_id): Path<String>,
    Json(payload): Json<TransferRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let transfer_type = payload.transfer_type.as_deref().unwrap_or("blind");
    if transfer_type != "blind" {
        return (
            StatusCode::NOT_IMPLEMENTED,
            Json(serde_json::json!({
                "error": "attended transfer not implemented",
                "call_id": call_id
            })),
        );
    }

    let target_uri: SipUri = match payload.target.parse() {
        Ok(u) => u,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": "invalid target URI",
                    "target": payload.target
                })),
            )
        }
    };

    // 在 mutable 锁内构建 REFER 请求（需要递增 dialog.local_cseq），
    // 锁在块结束时释放，避免跨 await 持锁。
    let refer = {
        let mut tx = match state.inbound_transactions.get_mut(&call_id) {
            Some(t) => t,
            None => {
                return (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({"error": "call not found", "call_id": call_id})),
                )
            }
        };
        let Some(template) = tx.original_request.clone() else {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "internal error",
                    "detail": "missing original request template"
                })),
            );
        };
        let mut template_owned = (*template).clone();
        let Ok(header_name) = HeaderName::new("Refer-To") else {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(
                    serde_json::json!({"error": "internal error", "detail": "invalid header name"}),
                ),
            );
        };
        template_owned.headers.insert(
            header_name,
            HeaderValue::new_owned(format!("<{target_uri}>")),
        );
        // 根据 call_id 匹配选择目标 dialog leg；未匹配时默认使用 caller leg。
        let dialog = if tx.dialogs.gateway.call_id == call_id {
            &mut tx.dialogs.gateway
        } else {
            &mut tx.dialogs.caller
        };
        match build_dialog_request(
            &template_owned,
            dialog,
            Method::Refer,
            &advertised_addr,
            &[],
        ) {
            Some(d) => d,
            None => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "error": "internal error",
                        "detail": "failed to build REFER (dialog not established)"
                    })),
                )
            }
        }
    };

    if let Some(socket) = state.get_socket() {
        if let Err(error) = socket.send_to(&refer.bytes, &refer.target).await {
            tracing::warn!(%error, target = %refer.target, %call_id, "管理转接发送 REFER 失败");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "internal error",
                    "detail": error.to_string()
                })),
            );
        }
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "call_id": call_id,
            "target": payload.target
        })),
    )
}
