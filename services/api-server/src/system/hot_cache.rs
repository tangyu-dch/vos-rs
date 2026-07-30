use crate::{ApiError, AppState};

const AUTH_USERS_KEY: &str = "vos_rs:auth:extensions";
const AUTH_EXTENSION_TENANTS_KEY: &str = "vos_rs:auth:extension_tenants";
const BILLING_BALANCES_KEY: &str = "vos_rs:billing:balances";
const BILLING_CREDIT_LIMITS_KEY: &str = "vos_rs:billing:credit_limits";
const BILLING_ACCOUNT_INTERVALS_KEY: &str = "vos_rs:billing:account_intervals";
const BILLING_ACCOUNT_PRICES_KEY: &str = "vos_rs:billing:account_prices";

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

/// 更新分机-租户映射缓存。
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

/// 更新单个账户的 Redis 计费缓存（余额、授信、周期、价格）。
pub(crate) async fn set_billing_account(
    state: &AppState,
    username: &str,
    balance: rust_decimal::Decimal,
    credit_limit: rust_decimal::Decimal,
    billing_interval_secs: u32,
    price_per_interval: rust_decimal::Decimal,
) -> Result<(), ApiError> {
    let mut connection = connection(state);
    redis::pipe()
        .atomic()
        .hset(BILLING_BALANCES_KEY, username, balance.to_string())
        .ignore()
        .hset(
            BILLING_CREDIT_LIMITS_KEY,
            username,
            credit_limit.to_string(),
        )
        .ignore()
        .hset(
            BILLING_ACCOUNT_INTERVALS_KEY,
            username,
            billing_interval_secs,
        )
        .ignore()
        .hset(
            BILLING_ACCOUNT_PRICES_KEY,
            username,
            price_per_interval.to_string(),
        )
        .ignore()
        .query_async(&mut connection)
        .await
        .map_err(|error| ApiError::internal(format!("Redis 账户缓存更新失败: {error}")))
}

/// 仅更新账户余额缓存（充值后调用）。
pub(crate) async fn set_account_balance(
    state: &AppState,
    username: &str,
    balance: rust_decimal::Decimal,
) -> Result<(), ApiError> {
    let mut connection = connection(state);
    redis::cmd("HSET")
        .arg(BILLING_BALANCES_KEY)
        .arg(username)
        .arg(balance.to_string())
        .query_async(&mut connection)
        .await
        .map_err(|error| ApiError::internal(format!("Redis 账户余额缓存更新失败: {error}")))
}

/// 删除单个账户的 Redis 计费缓存。
pub(crate) async fn delete_billing_account(
    state: &AppState,
    username: &str,
) -> Result<(), ApiError> {
    let mut connection = connection(state);
    redis::pipe()
        .atomic()
        .hdel(BILLING_BALANCES_KEY, username)
        .ignore()
        .hdel(BILLING_CREDIT_LIMITS_KEY, username)
        .ignore()
        .hdel(BILLING_ACCOUNT_INTERVALS_KEY, username)
        .ignore()
        .hdel(BILLING_ACCOUNT_PRICES_KEY, username)
        .ignore()
        .query_async(&mut connection)
        .await
        .map_err(|error| ApiError::internal(format!("Redis 账户缓存删除失败: {error}")))
}
