use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use serde::{Deserialize, Serialize};
use time::{Duration, OffsetDateTime};

use crate::{normalize_page, parse_dt, AppState, PageQuery, PaginatedResponse};
use cdr_core::{CreditAccountOutcome, ReconcileResult};
use rust_decimal::Decimal;

#[derive(Debug, Deserialize)]
pub struct RateBody {
    pub id: String,
    pub prefix: String,
    pub rate_per_minute: Option<Decimal>,
    pub billing_interval_secs: Option<i32>,
    pub price_per_interval: Option<Decimal>,
    pub description: Option<String>,
    /// 关联租户 ID（可空，NULL 表示全局费率）。
    pub tenant_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RateUpdate {
    pub prefix: String,
    pub rate_per_minute: Option<Decimal>,
    pub billing_interval_secs: Option<i32>,
    pub price_per_interval: Option<Decimal>,
    pub description: Option<String>,
    /// 更新关联租户 ID（可空，显式传空字符串清除关联）。
    pub tenant_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreditBody {
    pub amount: Decimal,
}

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
}

#[derive(Debug, Deserialize)]
pub struct ReconcileQuery {
    pub start_time: Option<String>,
    pub end_time: Option<String>,
}

#[derive(Serialize)]
pub struct CreditResult {
    pub username: String,
    pub balance: Decimal,
}

type E = (StatusCode, String);
fn err(e: impl std::fmt::Display) -> E {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

fn invalid(message: impl Into<String>) -> E {
    (StatusCode::BAD_REQUEST, message.into())
}

fn has_max_three_decimals(value: Decimal) -> bool {
    value.scale() <= 3
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
fn validate_rate(prefix: &str, rate_per_minute: Decimal) -> Result<(), E> {
    validate_prefix(prefix)?;
    validate_price(rate_per_minute, "费率")
}

fn validate_prefix(prefix: &str) -> Result<(), E> {
    if prefix.len() > 64 || !prefix.chars().all(|ch| ch.is_ascii_digit()) {
        return Err(invalid("费率前缀必须是 64 个字符以内的数字"));
    }
    Ok(())
}

fn validate_price(price: Decimal, label: &str) -> Result<(), E> {
    if price < Decimal::ZERO || price > Decimal::from(1_000_000) {
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
    legacy_rate: Option<Decimal>,
    interval_secs: Option<i32>,
    price: Option<Decimal>,
) -> Result<(i32, Decimal, Decimal), E> {
    let (interval_secs, price) = match (interval_secs, price) {
        (Some(interval), Some(price)) => (interval, price),
        (None, None) => (60, legacy_rate.ok_or_else(|| invalid("缺少费率价格"))?),
        _ => return Err(invalid("计费周期和周期价格必须同时提供")),
    };
    if interval_secs <= 0 || interval_secs > 86_400 {
        return Err(invalid("计费周期必须是 1 到 86400 秒之间的整数"));
    }
    validate_price(price, "周期价格")?;
    let equivalent_per_minute = price * Decimal::from(60) / Decimal::from(interval_secs);
    Ok((interval_secs, price, equivalent_per_minute))
}

/// 费率列表查询参数，支持 `?tenant_id=xxx` 过滤。
#[derive(Debug, Deserialize)]
pub struct RateListQuery {
    #[serde(flatten)]
    pub page: PageQuery,
    /// 按租户 ID 过滤（不传则返回全部费率，含全局与租户专属）。
    pub tenant_id: Option<String>,
}

/// 计费账户列表查询参数：在通用分页基础上增加关键字筛选。
#[derive(Debug, Deserialize)]
pub struct ListAccountsQuery {
    #[serde(flatten)]
    pub page: PageQuery,
    /// 按账户用户名模糊搜索
    pub q: Option<String>,
}

pub async fn list_rates(
    State(state): State<AppState>,
    Query(query): Query<RateListQuery>,
) -> Result<axum::response::Response, E> {
    let (page, page_size, offset) = normalize_page(&query.page);

    // 按租户过滤：tenant_id=xxx 时仅返回该租户专属费率（不含全局）
    let (items, total) = if let Some(ref tid) = query.tenant_id {
        let items = state.store.list_rates_by_tenant(tid).await.map_err(err)?;
        let total = items.len() as i64;
        (items, total)
    } else {
        tokio::try_join!(
            state.store.list_rates_page(page_size, offset),
            state.store.count_rates(),
        )
        .map_err(err)?
    };

    if query.page.export.unwrap_or(false) {
        let headers = vec![
            "费率标识",
            "前缀号码",
            "每分钟费率",
            "计费周期(秒)",
            "单周期价格",
            "租户ID",
        ];
        let mut rows = Vec::new();
        for item in &items {
            rows.push(vec![
                item.id.clone(),
                item.prefix.clone(),
                item.rate_per_minute.to_string(),
                item.billing_interval_secs.to_string(),
                item.price_per_interval.to_string(),
                item.tenant_id.clone().unwrap_or_default(),
            ]);
        }
        return Ok(crate::system::utils::to_csv_response(
            "rates.csv",
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
    let input = cdr_core::RateUpsert {
        id: &b.id,
        prefix: &b.prefix,
        rate_per_minute: equivalent,
        billing_interval_secs: interval,
        price_per_interval: price,
        description: b.description.as_deref(),
        tenant_id: b.tenant_id.as_deref(),
    };
    state.store.upsert_rate(&input).await.map_err(err)?;
    crate::system::hot_cache::rebuild_billing_rates(&state)
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
    let input = cdr_core::RateUpsert {
        id: &id,
        prefix: &b.prefix,
        rate_per_minute: equivalent,
        billing_interval_secs: interval,
        price_per_interval: price,
        description: b.description.as_deref(),
        tenant_id: b.tenant_id.as_deref(),
    };
    state.store.upsert_rate(&input).await.map_err(err)?;
    crate::system::hot_cache::rebuild_billing_rates(&state)
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
        crate::system::hot_cache::rebuild_billing_rates(&state)
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
    Query(query): Query<ListAccountsQuery>,
) -> Result<axum::response::Response, E> {
    let (page, page_size, offset) = normalize_page(&query.page);
    let q_trim = query.q.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let (items, total) = tokio::try_join!(
        state.store.list_accounts_page(page_size, offset, q_trim),
        state.store.count_accounts(q_trim),
    )
    .map_err(err)?;

    if query.page.export.unwrap_or(false) {
        let headers = vec!["账户用户名", "当前余额", "信用额度", "货币单位", "创建时间"];
        let mut rows = Vec::new();
        for item in items {
            rows.push(vec![
                item.username.clone(),
                item.balance.to_string(),
                item.credit_limit.to_string(),
                item.currency.clone(),
                item.created_at.map(|t| t.to_string()).unwrap_or_default(),
            ]);
        }
        return Ok(crate::system::utils::to_csv_response(
            "accounts.csv",
            &headers,
            &rows,
        ));
    }

    // 批量加载关联租户摘要：按 billing_account_id 分组，避免 N+1 查询
    let account_ids: Vec<i64> = items.iter().map(|a| a.id).collect();
    let tenant_map = load_tenants_by_account_ids(&state, &account_ids)
        .await
        .unwrap_or_default();

    let items_with_tenants: Vec<serde_json::Value> = items
        .into_iter()
        .map(|account| {
            let tenants = tenant_map.get(&account.id).cloned().unwrap_or_default();
            serde_json::json!({
                "id": account.id,
                "username": account.username,
                "balance": account.balance,
                "credit_limit": account.credit_limit,
                "currency": account.currency,
                "created_at": account.created_at,
                "associated_tenants": tenants,
            })
        })
        .collect();

    use axum::response::IntoResponse;
    Ok(Json(PaginatedResponse {
        items: items_with_tenants,
        total,
        page,
        page_size,
    })
    .into_response())
}

/// 批量按计费账户 ID 加载关联租户摘要（id + name + domain）。
///
/// 返回 HashMap<billing_account_id, Vec<TenantSummary>>，用于在账户列表中
/// 一次性展示所有关联租户，避免对每个账户单独查询。
async fn load_tenants_by_account_ids(
    state: &AppState,
    account_ids: &[i64],
) -> Result<std::collections::HashMap<i64, Vec<serde_json::Value>>, sqlx::Error> {
    if account_ids.is_empty() {
        return Ok(std::collections::HashMap::new());
    }
    #[derive(sqlx::FromRow)]
    struct TenantSummary {
        billing_account_id: i64,
        id: String,
        name: String,
        domain: String,
        enabled: bool,
    }
    let rows: Vec<TenantSummary> = sqlx::query_as(
        "SELECT billing_account_id, id, name, domain, enabled FROM tenants \
         WHERE billing_account_id = ANY($1) ORDER BY domain ASC",
    )
    .bind(account_ids)
    .fetch_all(state.store.pool())
    .await?;

    let mut map: std::collections::HashMap<i64, Vec<serde_json::Value>> =
        std::collections::HashMap::new();
    for row in rows {
        map.entry(row.billing_account_id)
            .or_default()
            .push(serde_json::json!({
                "id": row.id,
                "name": row.name,
                "domain": row.domain,
                "enabled": row.enabled,
            }));
    }
    Ok(map)
}

pub async fn credit_account(
    State(state): State<AppState>,
    Path(username): Path<String>,
    headers: HeaderMap,
    Json(b): Json<CreditBody>,
) -> Result<Json<CreditResult>, E> {
    if b.amount < Decimal::new(1, 3) || b.amount > Decimal::from(100_000_000) {
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
        if let Err(error) =
            crate::system::hot_cache::set_billing_balance(&state, &username, balance).await
        {
            tracing::warn!(
                %username,
                %balance,
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
    let (items, total) = tokio::try_join!(
        state
            .store
            .list_ledger_page(q.username.as_deref(), start, end, page_size, offset),
        state.store.count_ledger(q.username.as_deref(), start, end),
    )
    .map_err(err)?;

    if q.export.unwrap_or(false) {
        let headers = vec![
            "流水号",
            "呼叫 ID",
            "账户名",
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
    use rust_decimal::Decimal;

    #[test]
    fn rejects_negative_or_non_finite_rates() {
        assert!(validate_rate("86", Decimal::new(-1, 2)).is_err());
    }

    #[test]
    fn accepts_numeric_rate_prefixes() {
        assert!(validate_rate("8613", Decimal::new(35, 2)).is_ok());
        assert!(validate_rate("8613", Decimal::new(351, 3)).is_ok());
        assert!(validate_rate("", Decimal::ZERO).is_ok());
    }

    #[test]
    fn rejects_billing_values_with_more_than_three_decimals() {
        assert!(validate_rate("86", Decimal::new(3519, 4)).is_err());
        assert!(has_max_three_decimals(Decimal::new(100123, 3)));
        assert!(!has_max_three_decimals(Decimal::new(1001234, 4)));
    }

    #[test]
    fn returns_bad_request_for_invalid_input() {
        let error = super::invalid("invalid");
        assert_eq!(error.0, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn accepts_pulse_rate_and_maps_legacy_minute_rate() {
        assert_eq!(
            resolve_rate(Some(Decimal::new(25, 2)), None, None).unwrap(),
            (60, Decimal::new(25, 2), Decimal::new(25, 2))
        );
        let (interval, price, equivalent) =
            resolve_rate(None, Some(6), Some(Decimal::new(5, 2))).unwrap();
        assert_eq!((interval, price), (6, Decimal::new(5, 2)));
        assert_eq!(equivalent, Decimal::new(5, 1)); // 0.5
        assert!(resolve_rate(None, Some(6), Some(Decimal::new(501, 4))).is_err());
        assert!(resolve_rate(None, Some(7), Some(Decimal::new(5, 2))).is_ok());
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
