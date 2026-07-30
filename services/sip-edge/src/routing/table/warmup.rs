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
    let (credentials, trunk_creds, access, egress) = tokio::try_join!(
        db.list_user_credentials(),
        db.list_trunk_credentials(),
        db.list_billing_accounts_page(cdr_core::BillingAccountType::Access, 10_000, 0, None),
        db.list_billing_accounts_page(cdr_core::BillingAccountType::Egress, 10_000, 0, None),
    )?;
    let accounts: Vec<_> = access.into_iter().chain(egress).collect();

    let mut pipeline = redis::pipe();
    pipeline
        .atomic()
        .del("vos_rs:auth:extensions")
        .ignore()
        .del("vos_rs:auth:trunks")
        .ignore()
        .del("vos_rs:auth:extension_tenants")
        .ignore()
        .del("vos_rs:billing:balances")
        .ignore()
        .del("vos_rs:billing:credit_limits")
        .ignore()
        .del("vos_rs:billing:account_intervals")
        .ignore()
        .del("vos_rs:billing:account_prices")
        .ignore();
    for (username, password, tenant_id) in credentials {
        pipeline
            .hset("vos_rs:auth:extensions", &username, password)
            .ignore();
        if let Some(tid) = tenant_id {
            pipeline
                .hset("vos_rs:auth:extension_tenants", &username, &tid)
                .ignore();
        } else {
            pipeline
                .hdel("vos_rs:auth:extension_tenants", &username)
                .ignore();
        }
    }
    for (_trunk_id, username, password) in trunk_creds {
        pipeline
            .hset("vos_rs:auth:trunks", username, password)
            .ignore();
    }
    // 账户级费率缓存：余额、授信、计费周期、周期价格均按 username 索引。
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
            .ignore()
            .hset(
                "vos_rs:billing:account_intervals",
                &account.username,
                account.billing_interval_secs,
            )
            .ignore()
            .hset(
                "vos_rs:billing:account_prices",
                &account.username,
                account.price_per_interval.to_string(),
            )
            .ignore();
    }
    pipeline.query_async::<()>(&mut connection).await?;
    info!("Redis hot-path caches warmed from PostgreSQL");
    Ok(())
}
