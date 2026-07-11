use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use time::{Duration, OffsetDateTime};

use crate::{normalize_page, parse_dt, AppState, PageQuery, PaginatedResponse};
use cdr_core::{BillingAccount, BillingRate, LedgerEntry, ReconcileResult};

#[derive(Debug, Deserialize)]
pub struct RateBody {
    pub id: String,
    pub prefix: String,
    pub rate_per_minute: f64,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RateUpdate {
    pub prefix: String,
    pub rate_per_minute: f64,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreditBody {
    pub amount: f64,
}

#[derive(Debug, Deserialize)]
pub struct LedgerQuery {
    pub username: Option<String>,
    pub page: Option<i64>,
    pub page_size: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct ReconcileQuery {
    pub start_time: Option<String>,
    pub end_time: Option<String>,
}

#[derive(Serialize)]
pub struct CreditResult {
    pub username: String,
    pub balance: f64,
}

type E = (StatusCode, String);
fn err(e: impl std::fmt::Display) -> E {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

fn invalid(message: impl Into<String>) -> E {
    (StatusCode::BAD_REQUEST, message.into())
}

fn validate_rate(prefix: &str, rate_per_minute: f64) -> Result<(), E> {
    if prefix.len() > 64 || !prefix.chars().all(|ch| ch.is_ascii_digit()) {
        return Err(invalid("费率前缀必须是 64 个字符以内的数字"));
    }
    if !rate_per_minute.is_finite() || !(0.0..=1_000_000.0).contains(&rate_per_minute) {
        return Err(invalid("费率必须是 0 到 1000000 之间的有效数字"));
    }
    Ok(())
}

pub async fn list_rates(
    State(state): State<AppState>,
    Query(query): Query<PageQuery>,
) -> Result<Json<PaginatedResponse<BillingRate>>, E> {
    let (page, page_size, offset) = normalize_page(&query);
    let (items, total) = tokio::try_join!(
        state.store.list_rates_page(page_size, offset),
        state.store.count_rates(),
    )
    .map_err(err)?;
    Ok(Json(PaginatedResponse {
        items,
        total,
        page,
        page_size,
    }))
}

pub async fn create_rate(
    State(state): State<AppState>,
    Json(b): Json<RateBody>,
) -> Result<StatusCode, E> {
    validate_rate(&b.prefix, b.rate_per_minute)?;
    state
        .store
        .upsert_rate(
            &b.id,
            &b.prefix,
            b.rate_per_minute,
            b.description.as_deref(),
        )
        .await
        .map_err(err)?;
    Ok(StatusCode::CREATED)
}

pub async fn update_rate(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(b): Json<RateUpdate>,
) -> Result<StatusCode, E> {
    validate_rate(&b.prefix, b.rate_per_minute)?;
    state
        .store
        .upsert_rate(&id, &b.prefix, b.rate_per_minute, b.description.as_deref())
        .await
        .map_err(err)?;
    Ok(StatusCode::OK)
}

pub async fn delete_rate(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, E> {
    let deleted = state.store.delete_rate(&id).await.map_err(err)?;
    Ok(if deleted {
        StatusCode::OK
    } else {
        StatusCode::NOT_FOUND
    })
}

pub async fn list_accounts(
    State(state): State<AppState>,
    Query(query): Query<PageQuery>,
) -> Result<Json<PaginatedResponse<BillingAccount>>, E> {
    let (page, page_size, offset) = normalize_page(&query);
    let (items, total) = tokio::try_join!(
        state.store.list_accounts_page(page_size, offset),
        state.store.count_accounts(),
    )
    .map_err(err)?;
    Ok(Json(PaginatedResponse {
        items,
        total,
        page,
        page_size,
    }))
}

pub async fn credit_account(
    State(state): State<AppState>,
    Path(username): Path<String>,
    Json(b): Json<CreditBody>,
) -> Result<Json<CreditResult>, E> {
    if !b.amount.is_finite() || !(0.01..=100_000_000.0).contains(&b.amount) {
        return Err(invalid("充值金额必须是 0.01 到 100000000 之间的有效数字"));
    }
    if username.is_empty() || username.len() > 128 {
        return Err(invalid("账户名长度无效"));
    }
    let balance = state
        .store
        .credit_account(&username, b.amount)
        .await
        .map_err(err)?;
    Ok(Json(CreditResult { username, balance }))
}

pub async fn list_ledger(
    State(state): State<AppState>,
    Query(q): Query<LedgerQuery>,
) -> Result<Json<PaginatedResponse<LedgerEntry>>, E> {
    let page_query = PageQuery {
        page: q.page,
        page_size: q.page_size,
        gateway_type: None,
    };
    let (page, page_size, offset) = normalize_page(&page_query);
    let (items, total) = tokio::try_join!(
        state
            .store
            .list_ledger_page(q.username.as_deref(), page_size, offset),
        state.store.count_ledger(q.username.as_deref()),
    )
    .map_err(err)?;
    Ok(Json(PaginatedResponse {
        items,
        total,
        page,
        page_size,
    }))
}

pub async fn reconcile(
    State(state): State<AppState>,
    Query(q): Query<ReconcileQuery>,
) -> Result<Json<ReconcileResult>, E> {
    let end = q
        .end_time
        .as_deref()
        .and_then(parse_dt)
        .unwrap_or_else(OffsetDateTime::now_utc);
    let start = q
        .start_time
        .as_deref()
        .and_then(parse_dt)
        .unwrap_or(end - Duration::days(30));
    if start > end {
        return Err(invalid("对账开始时间不能晚于结束时间"));
    }
    state
        .store
        .reconcile_billing(start, end)
        .await
        .map(Json)
        .map_err(err)
}

#[cfg(test)]
mod tests {
    use super::{validate_rate, StatusCode};

    #[test]
    fn rejects_negative_or_non_finite_rates() {
        assert!(validate_rate("86", -0.01).is_err());
        assert!(validate_rate("86", f64::NAN).is_err());
        assert!(validate_rate("86", f64::INFINITY).is_err());
    }

    #[test]
    fn accepts_numeric_rate_prefixes() {
        assert!(validate_rate("8613", 0.35).is_ok());
        assert!(validate_rate("", 0.0).is_ok());
    }

    #[test]
    fn returns_bad_request_for_invalid_input() {
        let error = super::invalid("invalid");
        assert_eq!(error.0, StatusCode::BAD_REQUEST);
    }
}
