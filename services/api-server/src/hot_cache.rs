use crate::{ApiError, AppState};

const AUTH_USERS_KEY: &str = "vos_rs:auth_users";
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

/// 从 PostgreSQL 重建费率 Redis 缓存，管理端写入不影响 SIP 热路径。
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
    for rate in rates {
        pipeline
            .hset(BILLING_RATES_KEY, &rate.prefix, rate.rate_per_minute)
            .ignore()
            .hset(
                BILLING_INTERVALS_KEY,
                &rate.prefix,
                rate.billing_interval_secs,
            )
            .ignore()
            .hset(BILLING_PRICES_KEY, rate.prefix, rate.price_per_interval)
            .ignore();
    }
    pipeline
        .query_async(&mut connection)
        .await
        .map_err(|error| ApiError::internal(format!("Redis 费率缓存重建失败: {error}")))
}

/// 更新账户余额热路径缓存。
pub(crate) async fn set_billing_balance(
    state: &AppState,
    username: &str,
    balance: f64,
) -> Result<(), ApiError> {
    let mut connection = connection(state);
    redis::cmd("HSET")
        .arg(BILLING_BALANCES_KEY)
        .arg(username)
        .arg(balance)
        .query_async(&mut connection)
        .await
        .map_err(|error| ApiError::internal(format!("Redis 余额缓存更新失败: {error}")))
}
