use axum::{
    extract::{Query, State},
    Json,
};
use serde::Deserialize;

use crate::{normalize_page, ApiError, AppState, PageQuery, PaginatedResponse};

#[derive(Debug, Deserialize)]
pub struct RegistrationQuery {
    pub page: Option<i64>,
    pub page_size: Option<i64>,
    pub keyword: Option<String>,
    pub export: Option<bool>,
}

pub async fn list_registrations(
    State(state): State<AppState>,
    Query(query): Query<RegistrationQuery>,
) -> Result<axum::response::Response, ApiError> {
    let page_query = PageQuery {
        page: query.page,
        page_size: query.page_size,
        gateway_type: None,
        role: None,
        export: query.export,
    };
    let (page, page_size, offset) = normalize_page(&page_query);
    let (items, total) = tokio::try_join!(
        state
            .store
            .list_registrations_page(query.keyword.as_deref(), page_size, offset),
        state.store.count_registrations(query.keyword.as_deref()),
    )
    .map_err(|e| ApiError {
        error: e.to_string(),
    })?;

    if query.export.unwrap_or(false) {
        let headers = vec!["AOR(用户标识)", "联系地址", "接收地址", "过期时间", "更新时间"];
        let mut rows = Vec::new();
        for item in items {
            rows.push(vec![
                item.aor.clone(),
                item.contact_uri.clone(),
                item.received_from.clone(),
                item.expires_at.to_string(),
                item.updated_at.map(|t| t.to_string()).unwrap_or_default(),
            ]);
        }
        return Ok(crate::utils::to_csv_response("registrations.csv", &headers, &rows));
    }

    use axum::response::IntoResponse;
    Ok(Json(PaginatedResponse {
        items,
        total,
        page,
        page_size,
    }).into_response())
}
