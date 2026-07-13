use axum::{
    extract::{Path, Query, State},
    Json,
};
use cdr_core::{CdrEvent, DtmfEventRecord};
use serde::Deserialize;

use crate::{parse_dt, ApiError, AppState, PaginatedResponse};

#[derive(Debug, Deserialize)]
pub struct ListCdrsQuery {
    pub page: Option<i64>,
    pub page_size: Option<i64>,
    pub status: Option<String>,
    pub call_id: Option<String>,
    pub caller: Option<String>,
    pub callee: Option<String>,
    pub start_time: Option<String>,
    pub end_time: Option<String>,
}

pub async fn list_cdrs(
    State(state): State<AppState>,
    Query(query): Query<ListCdrsQuery>,
) -> Result<Json<PaginatedResponse<CdrEvent>>, ApiError> {
    let page = query.page.unwrap_or(1);
    let page_size = query.page_size.unwrap_or(20).min(100);

    let start = query.start_time.as_deref().and_then(parse_dt);
    let end = query.end_time.as_deref().and_then(parse_dt);

    let (items, total) = state
        .store
        .list_cdrs(
            page,
            page_size,
            query.status.as_deref(),
            query.call_id.as_deref(),
            query.caller.as_deref(),
            query.callee.as_deref(),
            start,
            end,
        )
        .await
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

pub async fn get_cdr(
    State(state): State<AppState>,
    Path(call_id): Path<String>,
) -> Result<Json<Option<CdrEvent>>, ApiError> {
    state
        .store
        .get_cdr(&call_id)
        .await
        .map(Json)
        .map_err(|e| ApiError {
            error: e.to_string(),
        })
}

pub async fn get_dtmf_events(
    State(state): State<AppState>,
    Path(call_id): Path<String>,
) -> Result<Json<Vec<DtmfEventRecord>>, ApiError> {
    state
        .store
        .get_dtmf_events(&call_id)
        .await
        .map(Json)
        .map_err(|e| ApiError {
            error: e.to_string(),
        })
}
