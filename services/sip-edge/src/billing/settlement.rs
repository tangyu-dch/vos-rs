use std::time::SystemTime;

use call_core::CallId;
use cdr_core::{CallSettlementInput, PostgresCdrStore};
use rust_decimal::prelude::ToPrimitive;

use crate::edge_state::EdgeState;

/// Returns the answered portion of a completed call in milliseconds.
pub(crate) fn answered_duration_ms(
    answered_at: Option<SystemTime>,
    ended_at: Option<SystemTime>,
) -> i64 {
    answered_at
        .zip(ended_at)
        .and_then(|(answered, ended)| ended.duration_since(answered).ok())
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

/// Returns the maximum whole-pulse call duration purchasable by the balance.
pub(crate) fn maximum_duration_secs(
    balance: f64,
    billing_interval_secs: u32,
    price_per_interval: f64,
) -> Option<u32> {
    if balance <= 0.0 || billing_interval_secs == 0 || price_per_interval <= 0.0 {
        return None;
    }
    let pulses = (balance / price_per_interval).floor();
    if !pulses.is_finite() || pulses < 1.0 {
        return Some(0);
    }
    Some(
        pulses
            .min(f64::from(u32::MAX / billing_interval_secs))
            .mul_add(f64::from(billing_interval_secs), 0.0) as u32,
    )
}

/// Settles a completed call against the billing account frozen at call setup.
pub(crate) fn settle_completed_call(edge_state: &EdgeState, call_id: &CallId) {
    crate::resource_lease::release(edge_state, call_id);
    if !edge_state.billing_settlement_enabled {
        return;
    }
    let Some(db) = edge_state.db_store.as_ref().cloned() else {
        return;
    };
    let Some(call) = edge_state.call_manager.get(call_id) else {
        return;
    };
    let duration_ms = answered_duration_ms(call.answered_at, call.ended_at);
    if duration_ms <= 0 {
        return;
    }
    let callee = call.inbound.remote_uri.user.unwrap_or_default();
    // 结算时使用 Call 上已注入的 tenant_id，按租户查找专属费率（回退全局）。
    let tenant_id = call.tenant_id.clone();
    let call_id = call_id.as_str().to_string();
    let access_account = call.billing_account.clone();
    let egress_account = call.audit.egress_billing_account.clone();
    let access_redis = edge_state.redis_connection();
    let egress_redis = edge_state.redis_connection();
    tokio::spawn(async move {
        if let Some(account) = access_account {
            settle_account(
                &db,
                CallSettlementInput {
                    call_id: &call_id,
                    entry_type: "call_charge",
                    username: &account,
                    callee: &callee,
                    duration_ms,
                    tenant_id: tenant_id.as_deref(),
                },
                access_redis,
            )
            .await;
        }
        if let Some(account) = egress_account {
            settle_account(
                &db,
                CallSettlementInput {
                    call_id: &call_id,
                    entry_type: "call_cost",
                    username: &account,
                    callee: &callee,
                    duration_ms,
                    tenant_id: tenant_id.as_deref(),
                },
                egress_redis,
            )
            .await;
        }
    });
}

async fn settle_account(
    db: &PostgresCdrStore,
    input: CallSettlementInput<'_>,
    redis_connection: Option<redis::aio::ConnectionManager>,
) {
    match db.settle_call_entry(input).await {
        Ok(Some(result)) => {
            update_cached_balance(
                redis_connection,
                input.username,
                result.balance_after.to_string(),
            )
            .await;
            if let Err(error) = db
                .update_call_billing_snapshot(
                    input.call_id,
                    input.entry_type,
                    result.billed_duration_ms,
                    result.amount.to_f64().unwrap_or(0.0),
                )
                .await
            {
                tracing::warn!(call_id = input.call_id, entry_type = input.entry_type, %error, "CDR 计费快照回写失败");
            }
            tracing::info!(call_id = input.call_id, account = input.username, entry_type = input.entry_type, amount = %result.amount, "实时计费结算完成");
        }
        Ok(None) => {}
        Err(error) => {
            tracing::warn!(call_id = input.call_id, account = input.username, entry_type = input.entry_type, %error, "实时计费结算失败")
        }
    }
}

async fn update_cached_balance(
    connection: Option<redis::aio::ConnectionManager>,
    account: &str,
    balance: String,
) {
    if let Some(mut connection) = connection {
        let _: Result<(), redis::RedisError> = redis::cmd("HSET")
            .arg("vos_rs:billing:balances")
            .arg(account)
            .arg(balance)
            .query_async(&mut connection)
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::{answered_duration_ms, maximum_duration_secs};
    use std::time::{Duration, SystemTime};

    #[test]
    fn answered_duration_excludes_ringing_time() {
        let started = SystemTime::UNIX_EPOCH;
        let answered = started + Duration::from_secs(15);
        let ended = started + Duration::from_secs(60);
        assert_eq!(answered_duration_ms(Some(answered), Some(ended)), 45_000);
        assert_eq!(answered_duration_ms(None, Some(ended)), 0);
    }

    #[test]
    fn maximum_duration_uses_only_complete_pulses() {
        assert_eq!(maximum_duration_secs(0.49, 60, 0.5), Some(0));
        assert_eq!(maximum_duration_secs(0.5, 60, 0.5), Some(60));
        assert_eq!(maximum_duration_secs(0.4, 6, 0.05), Some(48));
        assert_eq!(maximum_duration_secs(0.0, 6, 0.05), None);
    }

    #[test]
    fn maximum_duration_includes_credit_limit_in_available_funds() {
        let balance = -0.25;
        let credit_limit = 1.0;
        assert_eq!(
            maximum_duration_secs(balance + credit_limit, 6, 0.05),
            Some(90)
        );
    }
}
