use crate::edge_state::EdgeState;
use cdr_core::PostgresCdrStore;
use tracing::info;

use super::AnyError;

pub(crate) async fn warm_hot_path_redis_cache(
    edge_state: &EdgeState,
    db: Option<&PostgresCdrStore>,
) -> Result<(), AnyError> {
    let Some(db) = db else {
        return Ok(());
    };
    let Some(mut connection) = edge_state.redis_connection() else {
        return Err(std::io::Error::other("Redis connection is not initialized").into());
    };
    let (credentials, trunk_creds, rates, accounts) = tokio::try_join!(
        db.list_user_credentials(),
        db.list_trunk_credentials(),
        db.list_rates(),
        db.list_accounts(),
    )?;

    let mut pipeline = redis::pipe();
    pipeline
        .atomic()
        .del("vos_rs:auth:extensions")
        .ignore()
        .del("vos_rs:auth:trunks")
        .ignore()
        .del("vos_rs:billing:rates")
        .ignore()
        .del("vos_rs:billing:intervals")
        .ignore()
        .del("vos_rs:billing:prices")
        .ignore()
        .del("vos_rs:billing:balances")
        .ignore()
        .del("vos_rs:billing:credit_limits")
        .ignore();
    for (username, password) in credentials {
        pipeline
            .hset("vos_rs:auth:extensions", username, password)
            .ignore();
    }
    for (_trunk_id, username, password) in trunk_creds {
        pipeline
            .hset("vos_rs:auth:trunks", username, password)
            .ignore();
    }
    for rate in rates {
        pipeline
            .hset(
                "vos_rs:billing:rates",
                &rate.prefix,
                rate.rate_per_minute.to_string(),
            )
            .ignore()
            .hset(
                "vos_rs:billing:intervals",
                &rate.prefix,
                rate.billing_interval_secs,
            )
            .ignore()
            .hset(
                "vos_rs:billing:prices",
                &rate.prefix,
                rate.price_per_interval.to_string(),
            )
            .ignore();
    }
    for account in accounts {
        pipeline
            .hset(
                "vos_rs:billing:balances",
                &account.username,
                account.balance.to_string(),
            )
            .ignore()
            .hset(
                "vos_rs:billing:credit_limits",
                &account.username,
                account.credit_limit.to_string(),
            )
            .ignore();
    }
    pipeline.query_async::<()>(&mut connection).await?;
    info!("Redis hot-path caches warmed from PostgreSQL");
    Ok(())
}
