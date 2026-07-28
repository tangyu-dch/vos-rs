use crate::{ApiError, AppState};

const AUTH_USERS_KEY: &str = "vos_rs:auth:extensions";
const AUTH_EXTENSION_TENANTS_KEY: &str = "vos_rs:auth:extension_tenants";
const BILLING_RATES_KEY: &str = "vos_rs:billing:rates";
const BILLING_INTERVALS_KEY: &str = "vos_rs:billing:intervals";
const BILLING_PRICES_KEY: &str = "vos_rs:billing:prices";
const BILLING_BALANCES_KEY: &str = "vos_rs:billing:balances";

fn connection(state: &AppState) -> redis::aio::ConnectionManager {
    state.redis_client.clone()
}

/// 更新 SIP 鉴权热路径缓存。
pub(crate) async fn set_auth_user(
    state: &AppState,
    username: &str,
    password: &str,
) -> Result<(), ApiError> {
    let mut connection = connection(state);
    redis::cmd("HSET")
        .arg(AUTH_USERS_KEY)
        .arg(username)
        .arg(password)
        .query_async(&mut connection)
        .await
        .map_err(|error| ApiError::internal(format!("Redis 鉴权缓存更新失败: {error}")))
}

/// 删除 SIP 鉴权热路径缓存。
pub(crate) async fn delete_auth_user(state: &AppState, username: &str) -> Result<(), ApiError> {
    let mut connection = connection(state);
    redis::cmd("HDEL")
        .arg(AUTH_USERS_KEY)
        .arg(username)
        .query_async(&mut connection)
        .await
        .map_err(|error| ApiError::internal(format!("Redis 鉴权缓存删除失败: {error}")))
}

/// 更新分机-租户映射缓存，使热路径能按分机关联租户查找费率。
///
/// `tenant_id` 为 `None` 时从映射中删除该分机。
pub(crate) async fn set_extension_tenant(
    state: &AppState,
    username: &str,
    tenant_id: Option<&str>,
) -> Result<(), ApiError> {
    let mut connection = connection(state);
    let result: Result<(), redis::RedisError> = if let Some(tid) = tenant_id {
        redis::cmd("HSET")
            .arg(AUTH_EXTENSION_TENANTS_KEY)
            .arg(username)
            .arg(tid)
            .query_async(&mut connection)
            .await
    } else {
        redis::cmd("HDEL")
            .arg(AUTH_EXTENSION_TENANTS_KEY)
            .arg(username)
            .query_async(&mut connection)
            .await
    };
    result.map_err(|error| ApiError::internal(format!("Redis 分机租户映射缓存更新失败: {error}")))
}

/// 从 PostgreSQL 重建费率 Redis 缓存，管理端写入不影响 SIP 热路径。
///
/// 费率按 `tenant_id` 分桶：
/// - `tenant_id IS NULL` → 全局 hash（`vos_rs:billing:rates` 等）
/// - `tenant_id = Some(tid)` → 租户专属 hash（`vos_rs:billing:tenant_rates:{tid}` 等）
///
/// 余额校验路径优先查租户专属 hash，未命中时回退到全局 hash。
pub(crate) async fn rebuild_billing_rates(state: &AppState) -> Result<(), ApiError> {
    let rates = state
        .store
        .list_rates()
        .await
        .map_err(|error| ApiError::internal(error.to_string()))?;
    let mut connection = connection(state);
    let mut pipeline = redis::pipe();
    pipeline
        .atomic()
        .del(BILLING_RATES_KEY)
        .ignore()
        .del(BILLING_INTERVALS_KEY)
        .ignore()
        .del(BILLING_PRICES_KEY)
        .ignore();
    // 收集 tenant_id 列表，清理旧版 tenant 专属 hash（防止已删除租户残留）。
    let mut tenant_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    for rate in &rates {
        if let Some(tid) = rate.tenant_id.as_ref() {
            tenant_ids.insert(tid.clone());
        }
    }
    for tid in &tenant_ids {
        pipeline
            .del(format!("vos_rs:billing:tenant_rates:{tid}"))
            .ignore()
            .del(format!("vos_rs:billing:tenant_intervals:{tid}"))
            .ignore()
            .del(format!("vos_rs:billing:tenant_prices:{tid}"))
            .ignore();
    }
    for rate in rates {
        let (rates_key, intervals_key, prices_key) = match rate.tenant_id.as_deref() {
            Some(tid) => (
                format!("vos_rs:billing:tenant_rates:{tid}"),
                format!("vos_rs:billing:tenant_intervals:{tid}"),
                format!("vos_rs:billing:tenant_prices:{tid}"),
            ),
            None => (
                BILLING_RATES_KEY.to_string(),
                BILLING_INTERVALS_KEY.to_string(),
                BILLING_PRICES_KEY.to_string(),
            ),
        };
        pipeline
            .hset(rates_key, &rate.prefix, rate.rate_per_minute.to_string())
            .ignore()
            .hset(intervals_key, &rate.prefix, rate.billing_interval_secs)
            .ignore()
            .hset(
                prices_key,
                &rate.prefix,
                rate.price_per_interval.to_string(),
            )
            .ignore();
    }
    pipeline
        .query_async(&mut connection)
        .await
        .map_err(|error| ApiError::internal(format!("Redis 费率缓存重建失败: {error}")))
}

use rust_decimal::Decimal;

/// 更新账户余额热路径缓存。
pub(crate) async fn set_billing_balance(
    state: &AppState,
    username: &str,
    balance: Decimal,
) -> Result<(), ApiError> {
    let mut connection = connection(state);
    redis::cmd("HSET")
        .arg(BILLING_BALANCES_KEY)
        .arg(username)
        .arg(balance.to_string())
        .query_async(&mut connection)
        .await
        .map_err(|error| ApiError::internal(format!("Redis 余额缓存更新失败: {error}")))
}
