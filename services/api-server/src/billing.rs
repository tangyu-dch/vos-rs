use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use serde::{Deserialize, Serialize};
use time::{Duration, OffsetDateTime};

use crate::{normalize_page, parse_dt, AppState, PageQuery, PaginatedResponse};
use cdr_core::{BillingAccount, BillingRate, CreditAccountOutcome, LedgerEntry, ReconcileResult};

#[derive(Debug, Deserialize)]
pub struct RateBody {
    pub id: String,
    pub prefix: String,
    pub rate_per_minute: Option<f64>,
    pub billing_interval_secs: Option<i32>,
    pub price_per_interval: Option<f64>,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RateUpdate {
    pub prefix: String,
    pub rate_per_minute: Option<f64>,
    pub billing_interval_secs: Option<i32>,
    pub price_per_interval: Option<f64>,
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

fn has_max_three_decimals(value: f64) -> bool {
    let scaled = value * 1_000.0;
    (scaled - scaled.round()).abs() < 1e-7
}

fn idempotency_key(headers: &HeaderMap) -> Result<&str, E> {
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

#[cfg(test)]
fn validate_rate(prefix: &str, rate_per_minute: f64) -> Result<(), E> {
    validate_prefix(prefix)?;
    validate_price(rate_per_minute, "费率")
}

fn validate_prefix(prefix: &str) -> Result<(), E> {
    if prefix.len() > 64 || !prefix.chars().all(|ch| ch.is_ascii_digit()) {
        return Err(invalid("费率前缀必须是 64 个字符以内的数字"));
    }
    Ok(())
}

fn validate_price(price: f64, label: &str) -> Result<(), E> {
    if !price.is_finite() || !(0.0..=1_000_000.0).contains(&price) {
        return Err(invalid(format!(
            "{label}必须是 0 到 1000000 之间的有效数字"
        )));
    }
    if !has_max_three_decimals(price) {
        return Err(invalid(format!("{label}最多保留三位小数")));
    }
    Ok(())
}

fn resolve_rate(
    legacy_rate: Option<f64>,
    interval_secs: Option<i32>,
    price: Option<f64>,
) -> Result<(i32, f64, f64), E> {
    let (interval_secs, price) = match (interval_secs, price) {
        (Some(interval), Some(price)) => (interval, price),
        (None, None) => (60, legacy_rate.ok_or_else(|| invalid("缺少费率价格"))?),
        _ => return Err(invalid("计费周期和周期价格必须同时提供")),
    };
    if interval_secs <= 0 || interval_secs > 86_400 {
        return Err(invalid("计费周期必须是 1 到 86400 秒之间的整数"));
    }
    validate_price(price, "周期价格")?;
    let equivalent_per_minute = price * 60.0 / f64::from(interval_secs);
    Ok((interval_secs, price, equivalent_per_minute))
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
    let (interval, price, equivalent) = resolve_rate(
        b.rate_per_minute,
        b.billing_interval_secs,
        b.price_per_interval,
    )?;
    validate_prefix(&b.prefix)?;
    state
        .store
        .upsert_rate(
            &b.id,
            &b.prefix,
            equivalent,
            interval,
            price,
            b.description.as_deref(),
        )
        .await
        .map_err(err)?;
    crate::hot_cache::rebuild_billing_rates(&state)
        .await
        .map_err(|error| err(error.error))?;
    Ok(StatusCode::CREATED)
}

pub async fn update_rate(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(b): Json<RateUpdate>,
) -> Result<StatusCode, E> {
    let (interval, price, equivalent) = resolve_rate(
        b.rate_per_minute,
        b.billing_interval_secs,
        b.price_per_interval,
    )?;
    validate_prefix(&b.prefix)?;
    state
        .store
        .upsert_rate(
            &id,
            &b.prefix,
            equivalent,
            interval,
            price,
            b.description.as_deref(),
        )
        .await
        .map_err(err)?;
    crate::hot_cache::rebuild_billing_rates(&state)
        .await
        .map_err(|error| err(error.error))?;
    Ok(StatusCode::OK)
}

pub async fn delete_rate(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, E> {
    let deleted = state.store.delete_rate(&id).await.map_err(err)?;
    if deleted {
        crate::hot_cache::rebuild_billing_rates(&state)
            .await
            .map_err(|error| err(error.error))?;
    }
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
    headers: HeaderMap,
    Json(b): Json<CreditBody>,
) -> Result<Json<CreditResult>, E> {
    if !b.amount.is_finite() || !(0.001..=100_000_000.0).contains(&b.amount) {
        return Err(invalid("充值金额必须是 0.001 到 100000000 之间的有效数字"));
    }
    if !has_max_three_decimals(b.amount) {
        return Err(invalid("充值金额最多保留三位小数"));
    }
    if username.is_empty() || username.len() > 128 {
        return Err(invalid("账户名长度无效"));
    }
    let key = idempotency_key(&headers)?;
    let outcome = state
        .store
        .credit_account(&username, b.amount, key)
        .await
        .map_err(err)?;
    let (balance, applied) = match outcome {
        CreditAccountOutcome::Applied(balance) => (balance, true),
        CreditAccountOutcome::Replayed(balance) => (balance, false),
        CreditAccountOutcome::Conflict => {
            return Err((
                StatusCode::CONFLICT,
                "Idempotency-Key 已用于其他账户或金额".to_string(),
            ));
        }
    };
    if applied {
        if let Err(error) = crate::hot_cache::set_billing_balance(&state, &username, balance).await
        {
            tracing::warn!(
                %username,
                balance,
                error = %error.error,
                "充值已提交到数据库，但 Redis 余额缓存同步失败；返回成功以避免客户端重复充值"
            );
        }
    }
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
        role: None,
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
    use super::{
        has_max_three_decimals, idempotency_key, resolve_rate, validate_rate, HeaderMap, StatusCode,
    };

    #[test]
    fn rejects_negative_or_non_finite_rates() {
        assert!(validate_rate("86", -0.01).is_err());
        assert!(validate_rate("86", f64::NAN).is_err());
        assert!(validate_rate("86", f64::INFINITY).is_err());
    }

    #[test]
    fn accepts_numeric_rate_prefixes() {
        assert!(validate_rate("8613", 0.35).is_ok());
        assert!(validate_rate("8613", 0.351).is_ok());
        assert!(validate_rate("", 0.0).is_ok());
    }

    #[test]
    fn rejects_billing_values_with_more_than_three_decimals() {
        assert!(validate_rate("86", 0.3519).is_err());
        assert!(has_max_three_decimals(100.123));
        assert!(!has_max_three_decimals(100.1234));
    }

    #[test]
    fn returns_bad_request_for_invalid_input() {
        let error = super::invalid("invalid");
        assert_eq!(error.0, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn accepts_pulse_rate_and_maps_legacy_minute_rate() {
        assert_eq!(
            resolve_rate(Some(0.25), None, None).unwrap(),
            (60, 0.25, 0.25)
        );
        let (interval, price, equivalent) = resolve_rate(None, Some(6), Some(0.05)).unwrap();
        assert_eq!((interval, price), (6, 0.05));
        assert!((equivalent - 0.5).abs() < f64::EPSILON);
        assert!(resolve_rate(None, Some(6), Some(0.0501)).is_err());
        assert!(resolve_rate(None, Some(7), Some(0.05)).is_ok());
    }

    #[test]
    fn requires_valid_credit_idempotency_key() {
        let mut headers = HeaderMap::new();
        assert_eq!(
            idempotency_key(&headers).unwrap_err().0,
            StatusCode::BAD_REQUEST
        );
        headers.insert("idempotency-key", "credit-123".parse().unwrap());
        assert_eq!(idempotency_key(&headers).unwrap(), "credit-123");
    }
}
