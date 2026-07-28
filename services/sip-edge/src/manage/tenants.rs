//! 租户管理 API：查询当前加载的租户列表与运行时策略。
//!
//! 数据源为 `TenantRegistry` 内存注册表（由后台任务周期从 PostgreSQL 同步）。
//! 所有端点受 `X-VOS-Token` 内部认证保护。

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use serde::Serialize;

use crate::EdgeState;

/// 租户列表条目（管理 API 返回结构）。
///
/// 与 `tenant::TenantSummary` 字段保持一致，但独立定义以解耦管理 API 与内部类型。
#[derive(Debug, Serialize)]
pub struct TenantListItem {
    pub id: String,
    pub name: String,
    pub domain: String,
    pub enabled: bool,
    pub max_concurrent_calls: u32,
    pub max_cps: u32,
    pub cross_tenant_policy: String,
    pub recording_enabled: Option<bool>,
    pub billing_account_id: Option<i64>,
}

/// 统一响应信封。
#[derive(Debug, Serialize)]
pub struct TenantListResponse {
    pub code: u16,
    pub message: &'static str,
    pub data: Vec<TenantListItem>,
    pub total: usize,
}

/// `GET /manage/tenants`：返回当前注册表中所有租户的简要信息。
///
/// 若多租户隔离未启用（`tenant_enabled=false`），返回空列表与提示信息。
pub async fn list_tenants(State(edge): State<std::sync::Arc<EdgeState>>) -> impl IntoResponse {
    let Some(registry) = edge.tenant_registry() else {
        return (
            StatusCode::OK,
            Json(TenantListResponse {
                code: 0,
                message: "multi-tenant isolation disabled",
                data: Vec::new(),
                total: 0,
            }),
        );
    };

    let summaries = registry.list_tenants().await;
    let total = summaries.len();
    let data = summaries
        .into_iter()
        .map(|s| TenantListItem {
            id: s.id,
            name: s.name,
            domain: s.domain,
            enabled: s.enabled,
            max_concurrent_calls: s.max_concurrent_calls,
            max_cps: s.max_cps,
            cross_tenant_policy: format!("{:?}", s.cross_tenant_policy),
            recording_enabled: s.recording_enabled,
            billing_account_id: s.billing_account_id,
        })
        .collect();

    (
        StatusCode::OK,
        Json(TenantListResponse {
            code: 0,
            message: "success",
            data,
            total,
        }),
    )
}

/// `GET /manage/tenants/count`：返回当前注册表中的租户数量。
pub async fn tenant_count(State(edge): State<std::sync::Arc<EdgeState>>) -> impl IntoResponse {
    let count = match edge.tenant_registry() {
        Some(registry) => registry.tenant_count().await,
        None => 0,
    };
    Json(serde_json::json!({ "code": 0, "count": count }))
}
