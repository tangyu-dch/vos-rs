use axum::{
    extract::{Query, State},
    Json,
};
use serde::Deserialize;

use crate::{ApiError, AppState, PaginatedResponse};

#[derive(Debug, Deserialize)]
pub struct AuditLogQuery {
    pub page: Option<i64>,
    pub page_size: Option<i64>,
    pub export: Option<bool>,
}

/// 查询管理 API 审计日志，仅管理员可访问。
pub async fn list_audit_logs(
    State(state): State<AppState>,
    Query(query): Query<AuditLogQuery>,
) -> Result<axum::response::Response, ApiError> {
    let (page, page_size, offset) = if query.export.unwrap_or(false) {
        (1, 100000, 0)
    } else {
        let page = query.page.unwrap_or(1).max(1);
        let page_size = query.page_size.unwrap_or(50).clamp(1, 200);
        let offset = (page - 1).saturating_mul(page_size);
        (page, page_size, offset)
    };
    let (items, total) = tokio::try_join!(
        state.store.list_audit_logs(page_size, offset),
        state.store.count_audit_logs(),
    )
    .map_err(|e| ApiError {
        error: e.to_string(),
    })?;

    if query.export.unwrap_or(false) {
        let headers = vec![
            "ID",
            "请求 ID",
            "操作员",
            "角色",
            "请求方法",
            "路径",
            "请求参数",
            "状态码",
            "源 IP",
            "操作时间",
        ];
        let mut rows = Vec::new();
        for item in items {
            rows.push(vec![
                item.id.to_string(),
                item.request_id.clone(),
                item.username.clone(),
                item.role.clone(),
                item.method.clone(),
                item.path.clone(),
                item.query_params.clone().unwrap_or_default(),
                item.status_code.to_string(),
                item.source_ip.clone().unwrap_or_default(),
                item.created_at.map(|t| t.to_string()).unwrap_or_default(),
            ]);
        }
        return Ok(crate::system::utils::to_csv_response(
            "audit_logs.csv",
            &headers,
            &rows,
        ));
    }

    use axum::response::IntoResponse;
    Ok(Json(PaginatedResponse {
        items,
        total,
        page,
        page_size,
    })
    .into_response())
}
