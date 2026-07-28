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
        .del("vos_rs:auth:extension_tenants")
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
    // 收集所有租户 ID，用于在重建后清理过期 tenant 专属费率缓存。
    // 简化策略：每次 warmup 重建 global hash，对 tenant 专属 hash 用 scan + set 的方式增量更新。
    // 但 Redis pipeline 不便做 scan，这里采用"清空 + 重建"模式：通过 keys 模式删除所有 tenant_*_rates。
    let mut tenant_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    for rate in &rates {
        if let Some(tid) = rate.tenant_id.as_ref() {
            tenant_ids.insert(tid.clone());
        }
    }
    // 删除已知的 tenant 专属费率 hash（仅在已知 tenant_id 集合范围内）。
    for tid in &tenant_ids {
        pipeline
            .del(format!("vos_rs:billing:tenant_rates:{tid}"))
            .ignore()
            .del(format!("vos_rs:billing:tenant_intervals:{tid}"))
            .ignore()
            .del(format!("vos_rs:billing:tenant_prices:{tid}"))
            .ignore();
    }
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
    for rate in rates {
        // 费率缓存按 tenant_id 分桶：tenant_id IS NULL → 全局 hash；否则 → tenant 专属 hash。
        // 余额校验路径优先查 tenant 专属 hash，找不到时回退到全局 hash。
        let (rates_key, intervals_key, prices_key) = match rate.tenant_id.as_deref() {
            Some(tid) => (
                format!("vos_rs:billing:tenant_rates:{tid}"),
                format!("vos_rs:billing:tenant_intervals:{tid}"),
                format!("vos_rs:billing:tenant_prices:{tid}"),
            ),
            None => (
                "vos_rs:billing:rates".to_string(),
                "vos_rs:billing:intervals".to_string(),
                "vos_rs:billing:prices".to_string(),
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
    info!(
        tenant_count = tenant_ids.len(),
        "Redis hot-path caches warmed from PostgreSQL"
    );
    Ok(())
}
