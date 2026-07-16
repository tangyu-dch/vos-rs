use crate::models::{BillingAccount, BillingRate, LedgerEntry, ReconcileResult};
use crate::utils;
use crate::PostgresCdrStore;
use sqlx::Row;
use time::OffsetDateTime;
use tracing::warn;

impl PostgresCdrStore {
    pub async fn list_rates(&self) -> Result<Vec<BillingRate>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT id, prefix, rate_per_minute, billing_interval_secs, price_per_interval::DOUBLE PRECISION, description, created_at FROM billing_rates \
             ORDER BY length(prefix) DESC, prefix",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut rates = Vec::with_capacity(rows.len());
        for row in rows {
            rates.push(BillingRate {
                id: row.get(0),
                prefix: row.get(1),
                rate_per_minute: row.get(2),
                billing_interval_secs: row.get(3),
                price_per_interval: row.get(4),
                description: row.get(5),
                created_at: row.get(6),
            });
        }
        Ok(rates)
    }

    /// 按页读取费率配置。
    pub async fn list_rates_page(
        &self,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<BillingRate>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT id, prefix, rate_per_minute, billing_interval_secs, price_per_interval::DOUBLE PRECISION, description, created_at \
              FROM billing_rates ORDER BY length(prefix) DESC, prefix LIMIT $1 OFFSET $2",
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| BillingRate {
                id: row.get(0),
                prefix: row.get(1),
                rate_per_minute: row.get(2),
                billing_interval_secs: row.get(3),
                price_per_interval: row.get(4),
                description: row.get(5),
                created_at: row.get(6),
            })
            .collect())
    }

    /// 返回费率配置总数。
    pub async fn count_rates(&self) -> Result<i64, sqlx::Error> {
        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM billing_rates")
            .fetch_one(&self.pool)
            .await?;
        Ok(row.0)
    }

    pub async fn upsert_rate(
        &self,
        id: &str,
        prefix: &str,
        rate_per_minute: f64,
        billing_interval_secs: i32,
        price_per_interval: f64,
        description: Option<&str>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO billing_rates (id, prefix, rate_per_minute, billing_interval_secs, price_per_interval, description) VALUES ($1,$2,$3,$4,$5,$6) \
             ON CONFLICT (id) DO UPDATE SET prefix=EXCLUDED.prefix, rate_per_minute=EXCLUDED.rate_per_minute, billing_interval_secs=EXCLUDED.billing_interval_secs, price_per_interval=EXCLUDED.price_per_interval, description=EXCLUDED.description",
        )
        .bind(id)
        .bind(prefix)
        .bind(rate_per_minute)
        .bind(billing_interval_secs)
        .bind(price_per_interval)
        .bind(description)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn delete_rate(&self, id: &str) -> Result<bool, sqlx::Error> {
        let r = sqlx::query("DELETE FROM billing_rates WHERE id=$1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() > 0)
    }

    // ===== 计费：账户 =====
    pub async fn list_accounts(&self) -> Result<Vec<BillingAccount>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT username, balance, currency, created_at FROM billing_accounts ORDER BY username",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            out.push(BillingAccount {
                username: row.get(0),
                balance: row.get(1),
                currency: row.get(2),
                created_at: row.get(3),
            });
        }
        Ok(out)
    }

    /// 按页读取计费账户。
    pub async fn list_accounts_page(
        &self,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<BillingAccount>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT username, balance, currency, created_at FROM billing_accounts \
             ORDER BY username LIMIT $1 OFFSET $2",
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| BillingAccount {
                username: row.get(0),
                balance: row.get(1),
                currency: row.get(2),
                created_at: row.get(3),
            })
            .collect())
    }

    /// 返回计费账户总数。
    pub async fn count_accounts(&self) -> Result<i64, sqlx::Error> {
        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM billing_accounts")
            .fetch_one(&self.pool)
            .await?;
        Ok(row.0)
    }

    pub async fn credit_account(&self, username: &str, amount: f64) -> Result<f64, sqlx::Error> {
        sqlx::query(
            "INSERT INTO billing_accounts (username, balance) VALUES ($1, $2) \
             ON CONFLICT (username) DO UPDATE SET balance = billing_accounts.balance + $2",
        )
        .bind(username)
        .bind(amount)
        .execute(&self.pool)
        .await?;
        let balance: f64 =
            sqlx::query_scalar("SELECT balance FROM billing_accounts WHERE username=$1")
                .bind(username)
                .fetch_one(&self.pool)
                .await?;
        Ok(balance)
    }

    // ===== 实时计费 =====

    pub async fn check_balance(
        &self,
        username: &str,
        callee: &str,
    ) -> Result<(bool, f64, f64), sqlx::Error> {
        let balance: f64 = sqlx::query_scalar(
            "SELECT COALESCE(balance, 0.0)::DOUBLE PRECISION FROM billing_accounts WHERE username=$1",
        )
        .bind(username)
        .fetch_optional(&self.pool)
        .await?
        .unwrap_or(0.0);

        let rate: f64 = sqlx::query_scalar(
            "SELECT COALESCE(rate_per_minute, 0.0)::DOUBLE PRECISION FROM billing_rates \
              WHERE $2 LIKE prefix || '%' ORDER BY length(prefix) DESC LIMIT 1",
        )
        .bind(username)
        .bind(callee)
        .fetch_optional(&self.pool)
        .await?
        .unwrap_or(0.0);

        let has_balance = balance > 0.0 || rate == 0.0;
        Ok((has_balance, balance, rate))
    }

    pub async fn settle_call(
        &self,
        call_id: &str,
        username: &str,
        callee: &str,
        duration_ms: i64,
    ) -> Result<Option<f64>, sqlx::Error> {
        if username.is_empty() || duration_ms <= 0 {
            return Ok(None);
        }

        let exists: Option<i64> =
            sqlx::query_scalar("SELECT 1 FROM billing_ledger WHERE call_id=$1")
                .bind(call_id)
                .fetch_optional(&self.pool)
                .await?;
        if exists.is_some() {
            return Ok(None);
        }

        let rate: Option<(i32, f64)> = sqlx::query_as(
            "SELECT billing_interval_secs, price_per_interval::DOUBLE PRECISION FROM billing_rates \
              WHERE $1 LIKE prefix || '%' ORDER BY length(prefix) DESC LIMIT 1",
        )
        .bind(callee)
        .fetch_optional(&self.pool)
        .await?;

        let Some((interval_secs, price)) = rate else {
            return Ok(None);
        };

        let amount = pulse_amount(duration_ms, interval_secs, price);
        if amount <= 0.0 {
            return Ok(None);
        }

        let mut tx = self.pool.begin().await?;
        let updated = sqlx::query(
            "UPDATE billing_accounts \
              SET balance = balance - $1 \
              WHERE username = $2 AND balance - $1 >= -credit_limit \
              RETURNING balance::DOUBLE PRECISION",
        )
        .bind(amount)
        .bind(username)
        .fetch_optional(&mut *tx)
        .await?;

        let new_bal = match updated {
            Some(row) => row.get::<f64, _>(0),
            None => {
                warn!(%username, amount, call_id, "实时扣费失败：余额不足或账户未配置");
                tx.rollback().await?;
                return Ok(None);
            }
        };

        sqlx::query(
            "INSERT INTO billing_ledger (call_id, username, duration_ms, rate_per_minute, billing_interval_secs, price_per_interval, amount, balance_after) \
              VALUES ($1,$2,$3,$4,$5,$6,$7,$8)",
        )
        .bind(call_id)
        .bind(username)
        .bind(duration_ms)
        .bind(price * 60.0 / interval_secs as f64)
        .bind(interval_secs)
        .bind(price)
        .bind(amount)
        .bind(new_bal)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(Some(new_bal))
    }

    // ===== 计费：扣费明细 =====
    pub async fn list_ledger(
        &self,
        username: Option<&str>,
    ) -> Result<Vec<LedgerEntry>, sqlx::Error> {
        let rows = if let Some(u) = username {
            sqlx::query(
                "SELECT id, call_id, username, duration_ms, rate_per_minute, billing_interval_secs, price_per_interval::DOUBLE PRECISION, amount, balance_after, created_at \
                  FROM billing_ledger WHERE username=$1 ORDER BY created_at DESC LIMIT 500",
            )
            .bind(u)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query(
                "SELECT id, call_id, username, duration_ms, rate_per_minute, billing_interval_secs, price_per_interval::DOUBLE PRECISION, amount, balance_after, created_at \
                  FROM billing_ledger ORDER BY created_at DESC LIMIT 500",
            )
            .fetch_all(&self.pool)
            .await?
        };
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            out.push(LedgerEntry {
                id: row.get(0),
                call_id: row.get(1),
                username: row.get(2),
                duration_ms: row.get(3),
                rate_per_minute: row.get(4),
                billing_interval_secs: row.get(5),
                price_per_interval: row.get(6),
                amount: row.get(7),
                balance_after: row.get(8),
                created_at: row.get(9),
            });
        }
        Ok(out)
    }

    /// 按页读取扣费明细，支持按账户筛选。
    pub async fn list_ledger_page(
        &self,
        username: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<LedgerEntry>, sqlx::Error> {
        let rows = if let Some(username) = username {
            sqlx::query(
                "SELECT id, call_id, username, duration_ms, rate_per_minute, billing_interval_secs, price_per_interval::DOUBLE PRECISION, amount, balance_after, created_at \
                  FROM billing_ledger WHERE username = $1 ORDER BY created_at DESC, id DESC LIMIT $2 OFFSET $3",
            )
            .bind(username)
            .bind(limit)
            .bind(offset)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query(
                "SELECT id, call_id, username, duration_ms, rate_per_minute, billing_interval_secs, price_per_interval::DOUBLE PRECISION, amount, balance_after, created_at \
                  FROM billing_ledger ORDER BY created_at DESC, id DESC LIMIT $1 OFFSET $2",
            )
            .bind(limit)
            .bind(offset)
            .fetch_all(&self.pool)
            .await?
        };
        Ok(rows
            .into_iter()
            .map(|row| LedgerEntry {
                id: row.get(0),
                call_id: row.get(1),
                username: row.get(2),
                duration_ms: row.get(3),
                rate_per_minute: row.get(4),
                billing_interval_secs: row.get(5),
                price_per_interval: row.get(6),
                amount: row.get(7),
                balance_after: row.get(8),
                created_at: row.get(9),
            })
            .collect())
    }

    /// 返回扣费明细总数，可按账户筛选。
    pub async fn count_ledger(&self, username: Option<&str>) -> Result<i64, sqlx::Error> {
        let row: (i64,) = if let Some(username) = username {
            sqlx::query_as("SELECT COUNT(*) FROM billing_ledger WHERE username = $1")
                .bind(username)
                .fetch_one(&self.pool)
                .await?
        } else {
            sqlx::query_as("SELECT COUNT(*) FROM billing_ledger")
                .fetch_one(&self.pool)
                .await?
        };
        Ok(row.0)
    }

    // ===== 计费：离线对账 =====
    pub async fn reconcile_billing(
        &self,
        start: OffsetDateTime,
        end: OffsetDateTime,
    ) -> Result<ReconcileResult, sqlx::Error> {
        let rate_rows = sqlx::query(
            "SELECT prefix, billing_interval_secs, price_per_interval::DOUBLE PRECISION FROM billing_rates ORDER BY length(prefix) DESC",
        )
        .fetch_all(&self.pool)
        .await?;
        let rates: Vec<(String, i32, f64)> = rate_rows
            .into_iter()
            .map(|r| (r.get(0), r.get(1), r.get(2)))
            .collect();

        let cdr_rows = sqlx::query(
            "SELECT call_id, caller, callee, billable_duration_ms FROM call_cdrs \
              WHERE status='answered' AND started_at >= $1 AND started_at <= $2",
        )
        .bind(start)
        .bind(end)
        .fetch_all(&self.pool)
        .await?;

        let mut tx = self.pool.begin().await?;
        let mut processed: i64 = 0;
        let mut skipped: i64 = 0;
        let mut total_amount: f64 = 0.0;

        for row in cdr_rows {
            let call_id: String = row.get(0);
            let caller: Option<String> = row.get(1);
            let callee: Option<String> = row.get(2);
            let billable_ms: i64 = row.get(3);

            let exists: Option<i64> =
                sqlx::query_scalar("SELECT 1 FROM billing_ledger WHERE call_id=$1")
                    .bind(&call_id)
                    .fetch_optional(&mut *tx)
                    .await?;
            if exists.is_some() {
                skipped += 1;
                continue;
            }

            let user = caller
                .as_deref()
                .and_then(utils::extract_sip_user)
                .unwrap_or("");
            if user.is_empty() {
                skipped += 1;
                continue;
            }
            let callee_str = callee.unwrap_or_default();
            let Some((interval_secs, price)) = match_pulse_rate(&callee_str, &rates) else {
                skipped += 1;
                continue;
            };
            let amount = pulse_amount(billable_ms, interval_secs, price);

            let updated_row = sqlx::query(
                "UPDATE billing_accounts \
                  SET balance = balance - $1 \
                  WHERE username = $2 AND balance - $1 >= -credit_limit \
                  RETURNING (balance)::DOUBLE PRECISION",
            )
            .bind(amount)
            .bind(user)
            .fetch_optional(&mut *tx)
            .await?;

            let new_bal = match updated_row {
                Some(row) => row.get::<f64, _>(0),
                None => {
                    warn!(%user, amount, "扣除余额失败：余额不足以支付当前呼叫费用或账户未配置");
                    skipped += 1;
                    continue;
                }
            };

            sqlx::query(
                "INSERT INTO billing_ledger (call_id, username, duration_ms, rate_per_minute, billing_interval_secs, price_per_interval, amount, balance_after) \
                  VALUES ($1,$2,$3,$4,$5,$6,$7,$8)",
            )
            .bind(&call_id)
            .bind(user)
            .bind(billable_ms)
            .bind(price * 60.0 / interval_secs as f64)
            .bind(interval_secs)
            .bind(price)
            .bind(amount)
            .bind(new_bal)
            .execute(&mut *tx)
            .await?;
            processed += 1;
            total_amount += amount;
        }
        tx.commit().await?;
        Ok(ReconcileResult {
            processed,
            skipped,
            total_amount,
        })
    }
}

/// Calculates pulse billing by rounding any partial interval upward.
pub fn pulse_amount(duration_ms: i64, interval_secs: i32, price: f64) -> f64 {
    if duration_ms <= 0 || interval_secs <= 0 || price <= 0.0 {
        return 0.0;
    }
    let interval_ms = i64::from(interval_secs) * 1_000;
    let pulses = duration_ms.saturating_add(interval_ms - 1) / interval_ms;
    pulses as f64 * price
}

fn match_pulse_rate(callee: &str, rates: &[(String, i32, f64)]) -> Option<(i32, f64)> {
    rates
        .iter()
        .find(|(prefix, _, _)| callee.starts_with(prefix))
        .map(|(_, interval, price)| (*interval, *price))
}

#[cfg(test)]
mod tests {
    use super::pulse_amount;

    #[test]
    fn pulse_billing_rounds_partial_intervals_up() {
        assert_eq!(pulse_amount(45_000, 60, 1.0), 1.0);
        assert_eq!(pulse_amount(60_000, 60, 1.0), 1.0);
        assert_eq!(pulse_amount(61_000, 60, 1.0), 2.0);
        assert!((pulse_amount(45_000, 6, 0.05) - 0.40).abs() < f64::EPSILON);
    }
}
