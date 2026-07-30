use crate::models::{CallSettlementInput, CallSettlementResult, LedgerEntry};
use crate::PostgresCdrStore;
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use sqlx::Row;
use tracing::warn;

impl PostgresCdrStore {
    // ===== 实时计费 =====

    /// 按账户级费率解析计费脉冲（周期秒数 + 周期价格）。
    ///
    /// 计费规则完全由账户配置决定：对接账户的 `billing_interval_secs` /
    /// `price_per_interval` 用于向客户计费，落地账户的同名字段用于向供应商计成本。
    pub async fn resolve_billing_pulse(
        &self,
        username: &str,
        _callee: &str,
        _tenant_id: Option<&str>,
    ) -> Result<Option<(u32, f64)>, sqlx::Error> {
        let rate: Option<(i32, Decimal)> = sqlx::query_as(
            "SELECT billing_interval_secs, price_per_interval \
               FROM billing_accounts \
              WHERE username = $1 AND enabled AND deleted_at IS NULL \
                AND price_per_interval > 0",
        )
        .bind(username)
        .fetch_optional(&self.pool)
        .await?;

        Ok(rate.and_then(|(interval, price)| {
            u32::try_from(interval)
                .ok()
                .zip(price.to_f64())
                .filter(|(seconds, amount)| *seconds > 0 && *amount >= 0.0)
        }))
    }

    /// Settles either the access charge or egress cost for a completed call.
    pub async fn settle_call_entry(
        &self,
        input: CallSettlementInput<'_>,
    ) -> Result<Option<CallSettlementResult>, sqlx::Error> {
        if input.username.is_empty()
            || input.duration_ms <= 0
            || !matches!(input.entry_type, "call_charge" | "call_cost")
        {
            return Ok(None);
        }

        let exists: Option<i64> =
            sqlx::query_scalar("SELECT 1 FROM billing_ledger WHERE call_id=$1 AND entry_type=$2")
                .bind(input.call_id)
                .bind(input.entry_type)
                .fetch_optional(&self.pool)
                .await?;
        if exists.is_some() {
            return Ok(None);
        }

        // 费率直接取自账户配置（对接账户向客户计费，落地账户向供应商计成本）。
        let rate: Option<(i32, Decimal)> = sqlx::query_as(
            "SELECT billing_interval_secs, price_per_interval \
               FROM billing_accounts \
              WHERE username = $1 AND enabled AND deleted_at IS NULL \
                AND price_per_interval > 0",
        )
        .bind(input.username)
        .fetch_optional(&self.pool)
        .await?;

        let Some((interval_secs, price)) = rate else {
            return Ok(None);
        };

        let amount = pulse_amount(input.duration_ms, interval_secs, price);
        if amount.is_zero() {
            return Ok(None);
        }

        let mut tx = self.pool.begin().await?;
        let updated = sqlx::query(
            "UPDATE billing_accounts \
              SET balance = balance - $1 \
              WHERE username = $2 AND enabled AND deleted_at IS NULL \
                AND account_type = CASE WHEN $3 = 'call_charge' THEN 'access' ELSE 'egress' END \
                AND balance - $1 >= -credit_limit \
              RETURNING balance",
        )
        .bind(amount)
        .bind(input.username)
        .bind(input.entry_type)
        .fetch_optional(&mut *tx)
        .await?;

        let new_bal = match updated {
            Some(row) => row.get::<Decimal, _>(0),
            None => {
                warn!(username = input.username, %amount, call_id = input.call_id, entry_type = input.entry_type, "实时扣费失败：余额不足或账户未配置");
                tx.rollback().await?;
                return Ok(None);
            }
        };

        sqlx::query(
            "INSERT INTO billing_ledger (call_id, entry_type, username, duration_ms, rate_per_minute, billing_interval_secs, price_per_interval, amount, balance_after) \
              VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)",
        )
        .bind(input.call_id)
        .bind(input.entry_type)
        .bind(input.username)
        .bind(input.duration_ms)
        .bind(price * Decimal::from(60) / Decimal::from(interval_secs))
        .bind(interval_secs)
        .bind(price)
        .bind(amount)
        .bind(new_bal)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "INSERT INTO billing_journal ( \
                account_id, account_type, entry_type, amount, balance_before, balance_after, \
                call_id, operator_username, remark, idempotency_key \
             ) \
             SELECT id, account_type, $2, -$3, $4 + $3, $4, $1, 'system', \
                '通话自动结算', $1 || ':' || $2 \
             FROM billing_accounts WHERE username = $5 \
             ON CONFLICT (idempotency_key) WHERE idempotency_key IS NOT NULL DO NOTHING",
        )
        .bind(input.call_id)
        .bind(input.entry_type)
        .bind(amount)
        .bind(new_bal)
        .bind(input.username)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(Some(CallSettlementResult {
            balance_after: new_bal,
            billed_duration_ms: pulse_duration_ms(input.duration_ms, interval_secs),
            amount,
        }))
    }

    // ===== 计费：扣费明细 =====

    /// 按页读取扣费明细，支持按账户、时间范围和流水类型筛选。
    ///
    /// - `username`：精确匹配账户名
    /// - `start`/`end`：按 created_at 做 >= / <= 过滤
    /// - `entry_type`：按流水类型过滤（call_charge / call_cost）
    pub async fn list_ledger_page(
        &self,
        username: Option<&str>,
        start: Option<time::OffsetDateTime>,
        end: Option<time::OffsetDateTime>,
        limit: i64,
        offset: i64,
        entry_type: Option<&str>,
    ) -> Result<Vec<LedgerEntry>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT id, call_id, entry_type, username, duration_ms, CAST(rate_per_minute AS NUMERIC), billing_interval_secs, CAST(price_per_interval AS NUMERIC), CAST(amount AS NUMERIC), CAST(balance_after AS NUMERIC), created_at \
              FROM billing_ledger \
              WHERE ($1::TEXT IS NULL OR username = $1) \
                AND ($2::TIMESTAMPTZ IS NULL OR created_at >= $2) \
                AND ($3::TIMESTAMPTZ IS NULL OR created_at <= $3) \
                AND ($6::TEXT IS NULL OR entry_type = $6) \
              ORDER BY created_at DESC, id DESC LIMIT $4 OFFSET $5",
        )
        .bind(username)
        .bind(start)
        .bind(end)
        .bind(limit)
        .bind(offset)
        .bind(entry_type)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| LedgerEntry {
                id: row.get(0),
                call_id: row.get(1),
                entry_type: row.get(2),
                username: row.get(3),
                duration_ms: row.get(4),
                rate_per_minute: row.get(5),
                billing_interval_secs: row.get(6),
                price_per_interval: row.get(7),
                amount: row.get(8),
                balance_after: row.get(9),
                created_at: row.get(10),
            })
            .collect())
    }

    /// 返回与 `list_ledger_page` 相同过滤条件下的扣费明细总数。
    pub async fn count_ledger(
        &self,
        username: Option<&str>,
        start: Option<time::OffsetDateTime>,
        end: Option<time::OffsetDateTime>,
        entry_type: Option<&str>,
    ) -> Result<i64, sqlx::Error> {
        let row: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM billing_ledger \
             WHERE ($1::TEXT IS NULL OR username = $1) \
               AND ($2::TIMESTAMPTZ IS NULL OR created_at >= $2) \
               AND ($3::TIMESTAMPTZ IS NULL OR created_at <= $3) \
               AND ($4::TEXT IS NULL OR entry_type = $4)",
        )
        .bind(username)
        .bind(start)
        .bind(end)
        .bind(entry_type)
        .fetch_one(&self.pool)
        .await?;
        Ok(row.0)
    }
}

/// Calculates pulse billing by rounding any partial interval upward.
pub fn pulse_amount(duration_ms: i64, interval_secs: i32, price: Decimal) -> Decimal {
    if duration_ms <= 0 || interval_secs <= 0 || price.is_zero() {
        return Decimal::ZERO;
    }
    let interval_ms = i64::from(interval_secs) * 1_000;
    let pulses = duration_ms.saturating_add(interval_ms - 1) / interval_ms;
    Decimal::from(pulses) * price
}

/// Calculates billed milliseconds by rounding any partial interval upward.
pub fn pulse_duration_ms(duration_ms: i64, interval_secs: i32) -> i64 {
    if duration_ms <= 0 || interval_secs <= 0 {
        return 0;
    }
    let interval_ms = i64::from(interval_secs) * 1_000;
    duration_ms
        .saturating_add(interval_ms - 1)
        .saturating_div(interval_ms)
        .saturating_mul(interval_ms)
}

#[cfg(test)]
mod tests {
    use super::{pulse_amount, pulse_duration_ms};
    use rust_decimal::Decimal;

    #[test]
    fn pulse_billing_rounds_partial_intervals_up() {
        assert_eq!(pulse_amount(45_000, 60, Decimal::from(1)), Decimal::from(1));
        assert_eq!(pulse_amount(60_000, 60, Decimal::from(1)), Decimal::from(1));
        assert_eq!(pulse_amount(61_000, 60, Decimal::from(1)), Decimal::from(2));
        let price = Decimal::new(5, 2); // 0.05
        assert_eq!(pulse_amount(45_000, 6, price), Decimal::new(40, 2)); // 0.40
    }

    #[test]
    fn pulse_duration_rounds_sixty_one_seconds_to_two_minutes() {
        assert_eq!(pulse_duration_ms(61_000, 60), 120_000);
        assert_eq!(pulse_duration_ms(60_000, 60), 60_000);
        assert_eq!(pulse_duration_ms(0, 60), 0);
    }
}
