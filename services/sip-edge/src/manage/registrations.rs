//! 出站网关注册状态查询 API。
//!
//! 暴露 `OutboundRegState` 的关键信息，用于运维监控网关注册健康度。
//! 所有端点受 `X-VOS-Token` 内部认证保护。

use axum::{extract::State, response::IntoResponse, Json};
use serde::Serialize;
use std::sync::Arc;

use crate::EdgeState;

/// 出站注册状态摘要（管理 API 返回结构）。
#[derive(Debug, Serialize)]
pub struct OutboundRegistrationItem {
    pub gateway_id: String,
    pub host: String,
    pub port: Option<u16>,
    pub transport: String,
    pub username: String,
    pub call_id: String,
    pub cseq: u32,
    pub expires: u32,
    pub last_reg_sent: Option<u64>,
    pub last_reg_success: Option<u64>,
    pub authenticated: bool,
}

/// `GET /manage/outbound-registrations`：返回所有出站网关注册状态。
pub async fn list_outbound_registrations(State(edge): State<Arc<EdgeState>>) -> impl IntoResponse {
    let items: Vec<OutboundRegistrationItem> = edge
        .outbound_registrations
        .iter()
        .map(|entry| {
            let reg = entry.value();
            OutboundRegistrationItem {
                gateway_id: reg.gateway_id.clone(),
                host: reg.host.clone(),
                port: reg.port,
                transport: reg.transport.clone(),
                username: reg.username.clone(),
                call_id: reg.call_id.clone(),
                cseq: reg.cseq,
                expires: reg.expires,
                last_reg_sent: reg.last_reg_sent.map(|t| t.elapsed().as_secs()),
                last_reg_success: reg.last_reg_success.map(|t| t.elapsed().as_secs()),
                // challenge 存在表示正在进行 Digest 认证握手
                authenticated: reg.challenge.is_none(),
            }
        })
        .collect();

    Json(serde_json::json!({
        "code": 0,
        "message": "success",
        "data": items,
        "total": items.len(),
    }))
}
