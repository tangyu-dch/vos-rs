use std::time::SystemTime;

use call_core::CallId;

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
    let Some(account) = call.billing_account.clone() else {
        return;
    };
    let duration_ms = answered_duration_ms(call.answered_at, call.ended_at);
    if duration_ms <= 0 {
        return;
    }
    let callee = call.inbound.remote_uri.user.unwrap_or_default();
    let call_id = call_id.as_str().to_string();
    let redis_connection = edge_state.redis_connection();
    tokio::spawn(async move {
        match db
            .settle_call(&call_id, &account, &callee, duration_ms)
            .await
        {
            Ok(Some(balance)) => {
                if let Some(mut connection) = redis_connection {
                    let _: Result<(), redis::RedisError> = redis::cmd("HSET")
                        .arg("vos_rs:billing:balances")
                        .arg(&account)
                        .arg(balance.to_string())
                        .query_async(&mut connection)
                        .await;
                }
                tracing::info!(%call_id, %account, %balance, "实时计费结算完成");
            }
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(%call_id, %account, %error, "实时计费结算失败");
            }
        }
    });
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
