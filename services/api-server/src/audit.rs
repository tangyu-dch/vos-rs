use axum::{
    extract::{Query, State},
    Json,
};
use cdr_core::AuditLog;
use serde::Deserialize;

use crate::{ApiError, AppState, PaginatedResponse};

#[derive(Debug, Deserialize)]
pub struct AuditLogQuery {
    pub page: Option<i64>,
    pub page_size: Option<i64>,
}

/// 查询管理 API 审计日志，仅管理员可访问。
pub async fn list_audit_logs(
    State(state): State<AppState>,
    Query(query): Query<AuditLogQuery>,
) -> Result<Json<PaginatedResponse<AuditLog>>, ApiError> {
    let page = query.page.unwrap_or(1).max(1);
    let page_size = query.page_size.unwrap_or(50).clamp(1, 200);
    let offset = (page - 1).saturating_mul(page_size);
    let (items, total) = tokio::try_join!(
        state.store.list_audit_logs(page_size, offset),
        state.store.count_audit_logs(),
    )
    .map_err(|e| ApiError {
        error: e.to_string(),
    })?;
    Ok(Json(PaginatedResponse {
        items,
        total,
        page,
        page_size,
    }))
}
