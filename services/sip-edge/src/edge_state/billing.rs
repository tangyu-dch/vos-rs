use crate::edge_state::EdgeState;

#[derive(Debug, Clone, Copy)]
pub(crate) struct RedisBalanceCheck {
    pub(crate) has_balance: bool,
    pub(crate) account_found: bool,
    pub(crate) rate_found: bool,
    pub(crate) balance: f64,
    pub(crate) credit_limit: f64,
    pub(crate) billing_interval_secs: u32,
    pub(crate) price_per_interval: f64,
}

type RedisBillingPipelineResult = (
    Option<f64>,
    Option<f64>,
    Vec<Option<u32>>,
    Vec<Option<f64>>,
    Vec<Option<f64>>,
);

pub(crate) fn build_balance_check(
    balance: Option<f64>,
    credit_limit: Option<f64>,
    pulse: Option<(u32, f64)>,
    legacy_rate: Option<f64>,
) -> RedisBalanceCheck {
    let account_found = balance.is_some();
    let balance = balance.unwrap_or(0.0);
    let credit_limit = credit_limit.unwrap_or(0.0).max(0.0);
    let rate_found = pulse.is_some() || legacy_rate.is_some();
    let (billing_interval_secs, price_per_interval) =
        pulse.unwrap_or_else(|| (60, legacy_rate.unwrap_or(0.0)));
    RedisBalanceCheck {
        has_balance: account_found
            && rate_found
            && (price_per_interval == 0.0 || balance + credit_limit >= price_per_interval),
        account_found,
        rate_found,
        balance,
        credit_limit,
        billing_interval_secs,
        price_per_interval,
    }
}

impl EdgeState {
    /// 从 Redis 一次读取账户余额与最长前缀费率。
    pub(crate) async fn redis_balance_check(
        &self,
        username: &str,
        callee: &str,
    ) -> Option<RedisBalanceCheck> {
        let mut connection = self.redis_connection()?;
        let prefixes = (0..=callee.len())
            .rev()
            .filter(|index| callee.is_char_boundary(*index))
            .map(|index| &callee[..index])
            .collect::<Vec<_>>();
        let mut pipeline = redis::pipe();
        pipeline
            .cmd("HGET")
            .arg("vos_rs:billing:balances")
            .arg(username)
            .cmd("HGET")
            .arg("vos_rs:billing:credit_limits")
            .arg(username)
            .cmd("HMGET")
            .arg("vos_rs:billing:intervals");
        for prefix in &prefixes {
            pipeline.arg(prefix);
        }
        pipeline.cmd("HMGET").arg("vos_rs:billing:prices");
        for prefix in &prefixes {
            pipeline.arg(prefix);
        }
        pipeline.cmd("HMGET").arg("vos_rs:billing:rates");
        for prefix in &prefixes {
            pipeline.arg(prefix);
        }
        let (balance, credit_limit, intervals, prices, legacy_rates): RedisBillingPipelineResult =
            pipeline.query_async(&mut connection).await.ok()?;
        let pulse = intervals
            .into_iter()
            .zip(prices)
            .find_map(|(interval, price)| interval.zip(price));
        let legacy_rate = legacy_rates.into_iter().flatten().next();
        Some(build_balance_check(
            balance,
            credit_limit,
            pulse,
            legacy_rate,
        ))
    }
}
