use axum::{
    extract::{Query, State},
    Json,
};
use cdr_core::SipRegistration;
use serde::Deserialize;

use crate::{normalize_page, ApiError, AppState, PageQuery, PaginatedResponse};

#[derive(Debug, Deserialize)]
pub struct RegistrationQuery {
    pub page: Option<i64>,
    pub page_size: Option<i64>,
    pub keyword: Option<String>,
}

pub async fn list_registrations(
    State(state): State<AppState>,
    Query(query): Query<RegistrationQuery>,
) -> Result<Json<PaginatedResponse<SipRegistration>>, ApiError> {
    let page_query = PageQuery {
        page: query.page,
        page_size: query.page_size,
        gateway_type: None,
        role: None,
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
    Ok(Json(PaginatedResponse {
        items,
        total,
        page,
        page_size,
    }))
}
