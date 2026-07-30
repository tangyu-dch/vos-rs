use crate::edge_state::EdgeState;

/// Redis 热路径余额与费率校验结果。
///
/// 计费规则完全由账户配置决定（对接账户向客户计费、落地账户向供应商计成本），
/// 不再依赖按号码前缀匹配的费率表。
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

type AccountRateResult = (Option<f64>, Option<f64>, Option<u32>, Option<f64>);

pub(crate) fn build_balance_check(
    balance: Option<f64>,
    credit_limit: Option<f64>,
    pulse: Option<(u32, f64)>,
) -> RedisBalanceCheck {
    let account_found = balance.is_some();
    let balance = balance.unwrap_or(0.0);
    let credit_limit = credit_limit.unwrap_or(0.0).max(0.0);
    let rate_found = pulse.is_some();
    let (billing_interval_secs, price_per_interval) = pulse.unwrap_or((60, 0.0));
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
    /// 从 Redis 按账户 username 读取余额与账户级费率。
    ///
    /// 费率来源于账户自身的 `billing_interval_secs` / `price_per_interval`，
    /// 不再依赖号码前缀匹配。
    pub(crate) async fn redis_balance_check(
        &self,
        username: &str,
        _callee: &str,
        _tenant_id: Option<&str>,
    ) -> Option<RedisBalanceCheck> {
        let mut connection = self.redis_connection()?;
        let (balance, credit_limit, interval, price): AccountRateResult = redis::pipe()
            .cmd("HGET")
            .arg("vos_rs:billing:balances")
            .arg(username)
            .cmd("HGET")
            .arg("vos_rs:billing:credit_limits")
            .arg(username)
            .cmd("HGET")
            .arg("vos_rs:billing:account_intervals")
            .arg(username)
            .cmd("HGET")
            .arg("vos_rs:billing:account_prices")
            .arg(username)
            .query_async(&mut connection)
            .await
            .ok()?;

        if balance.is_none() {
            // 当分机未绑定独立计费账户时，免费内部分机通话放行（设定为0费率）
            return Some(RedisBalanceCheck {
                has_balance: true,
                account_found: true,
                rate_found: true,
                balance: 0.0,
                credit_limit: 0.0,
                billing_interval_secs: 60,
                price_per_interval: 0.0,
            });
        }

        let pulse = interval.zip(price).filter(|(secs, _)| *secs > 0);
        Some(build_balance_check(balance, credit_limit, pulse))
    }
}
