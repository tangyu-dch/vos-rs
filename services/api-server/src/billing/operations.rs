use axum::{
    extract::{Query, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;

use crate::{normalize_page, parse_dt, AppState, PageQuery, PaginatedResponse};

#[derive(Debug, Deserialize)]
pub struct LedgerQuery {
    pub username: Option<String>,
    pub page: Option<i64>,
    pub page_size: Option<i64>,
    pub export: Option<bool>,
    /// 按发生时间起始过滤（RFC3339 或 YYYY-MM-DDTHH:mm）
    pub start_time: Option<String>,
    /// 按发生时间截止过滤（RFC3339 或 YYYY-MM-DDTHH:mm）
    pub end_time: Option<String>,
    /// 按流水类型过滤（call_charge / call_cost）
    pub entry_type: Option<String>,
}

type E = (StatusCode, String);
fn err(e: impl std::fmt::Display) -> E {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

#[cfg(test)]
fn invalid(message: impl Into<String>) -> E {
    (StatusCode::BAD_REQUEST, message.into())
}

/// 仅服务于单元测试：在新计费体系下，生产代码使用 `accounts::support::idempotency_key`。
#[cfg(test)]
fn idempotency_key(headers: &axum::http::HeaderMap) -> Result<&str, E> {
    let key = headers
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty() && value.len() <= 128)
        .ok_or_else(|| invalid("充值请求必须提供 128 字符以内的 Idempotency-Key"))?;
    if !key.bytes().all(|byte| byte.is_ascii_graphic()) {
        return Err(invalid("Idempotency-Key 只能包含可见 ASCII 字符"));
    }
    Ok(key)
}

pub async fn list_ledger(
    State(state): State<AppState>,
    Query(q): Query<LedgerQuery>,
) -> Result<axum::response::Response, E> {
    let page_query = PageQuery {
        page: q.page,
        page_size: q.page_size,
        gateway_type: None,
        role: None,
        export: q.export,
    };
    let (page, page_size, offset) = normalize_page(&page_query);
    let start = q.start_time.as_deref().and_then(parse_dt);
    let end = q.end_time.as_deref().and_then(parse_dt);
    let entry_type = q.entry_type.as_deref();
    let (items, total) = tokio::try_join!(
        state.store.list_ledger_page(
            q.username.as_deref(),
            start,
            end,
            page_size,
            offset,
            entry_type,
        ),
        state
            .store
            .count_ledger(q.username.as_deref(), start, end, entry_type),
    )
    .map_err(err)?;

    if q.export.unwrap_or(false) {
        let headers = vec![
            "流水号",
            "呼叫 ID",
            "账户名",
            "流水类型",
            "通话时长(ms)",
            "费率/分钟",
            "计费周期(秒)",
            "周期单价",
            "扣费金额",
            "期后余额",
            "创建时间",
        ];
        let mut rows = Vec::new();
        for item in items {
            rows.push(vec![
                item.id.to_string(),
                item.call_id.clone(),
                item.username.clone(),
                item.entry_type.clone(),
                item.duration_ms.to_string(),
                item.rate_per_minute.to_string(),
                item.billing_interval_secs.to_string(),
                item.price_per_interval.to_string(),
                item.amount.to_string(),
                item.balance_after.to_string(),
                item.created_at.map(|t| t.to_string()).unwrap_or_default(),
            ]);
        }
        return Ok(crate::system::utils::to_csv_response(
            "ledger.csv",
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

#[cfg(test)]
mod tests {
    use super::{idempotency_key, StatusCode};

    #[test]
    fn returns_bad_request_for_invalid_input() {
        let error = super::invalid("invalid");
        assert_eq!(error.0, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn requires_valid_credit_idempotency_key() {
        let mut headers = axum::http::HeaderMap::new();
        assert_eq!(
            idempotency_key(&headers).unwrap_err().0,
            StatusCode::BAD_REQUEST
        );
        headers.insert("idempotency-key", "credit-123".parse().unwrap());
        assert_eq!(idempotency_key(&headers).unwrap(), "credit-123");
    }
}
