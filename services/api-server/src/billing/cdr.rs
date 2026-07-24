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
    pub before_id: Option<i64>,
    pub export: Option<bool>,
}

pub async fn list_cdrs(
    State(state): State<AppState>,
    Query(query): Query<ListCdrsQuery>,
) -> Result<axum::response::Response, ApiError> {
    let (page, page_size) = if query.export.unwrap_or(false) {
        (1, 100000)
    } else {
        (query.page.unwrap_or(1), query.page_size.unwrap_or(20).min(100))
    };

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
            query.before_id,
        )
        .await
        .map_err(|e| ApiError {
            error: e.to_string(),
        })?;

    if query.export.unwrap_or(false) {
        let headers = vec![
            "通话 ID", "主叫号码", "被叫号码", "状态", "呼叫开始时间", "应答时间", 
            "结束时间", "通话时长(毫秒)", "计费时长(毫秒)", "失败代码", "失败原因"
        ];
        let mut rows = Vec::new();
        for item in items {
            rows.push(vec![
                item.call_id.clone(),
                item.caller.clone().unwrap_or_default(),
                item.callee.clone().unwrap_or_default(),
                item.status.clone(),
                item.started_at_ms.to_string(),
                item.answered_at_ms.map(|t| t.to_string()).unwrap_or_default(),
                item.ended_at_ms.to_string(),
                item.duration_ms.to_string(),
                item.billable_duration_ms.to_string(),
                item.failure_status_code.map(|c| c.to_string()).unwrap_or_default(),
                item.failure_reason.clone().unwrap_or_default(),
            ]);
        }
        return Ok(crate::system::utils::to_csv_response("cdrs.csv", &headers, &rows));
    }

    use axum::response::IntoResponse;
    Ok(Json(PaginatedResponse {
        items,
        total,
        page,
        page_size,
    }).into_response())
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
