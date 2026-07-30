use crate::{
    BillingAccountRecord, BillingAccountType, BillingCreditOutcome, BillingJournalRecord,
    CreateBillingAccountInput, DeleteBillingAccountOutcome, PostgresCdrStore,
    UpdateBillingAccountInput,
};
use rust_decimal::Decimal;

use super::query::{load_journal_entry, ACCOUNT_COLUMNS};

impl PostgresCdrStore {
    /// 创建余额为零的计费账户，可同时关联多个同类型网关。
    pub async fn create_billing_account(
        &self,
        input: &CreateBillingAccountInput<'_>,
    ) -> Result<BillingAccountRecord, sqlx::Error> {
        let mut transaction = self.pool.begin().await?;
        let account = sqlx::query_as::<_, BillingAccountRecord>(&format!(
            "INSERT INTO billing_accounts (username, account_type, balance, credit_limit, \
             billing_interval_secs, price_per_interval, enabled, tenant_id, updated_at) \
             VALUES ($1, $2, 0, $3, $4, $5, $6, $7, now()) RETURNING {ACCOUNT_COLUMNS}"
        ))
        .bind(input.username)
        .bind(input.account_type.as_str())
        .bind(input.credit_limit)
        .bind(input.billing_interval_secs)
        .bind(input.price_per_interval)
        .bind(input.enabled)
        .bind(input.tenant_id)
        .fetch_one(&mut *transaction)
        .await?;
        link_gateways(&mut transaction, input.gateway_ids, account.id).await?;
        transaction.commit().await?;
        Ok(account)
    }

    /// 修改账户名称、信用额度和默认计费脉冲，并重建网关关联。
    pub async fn update_billing_account(
        &self,
        id: i64,
        account_type: BillingAccountType,
        input: &UpdateBillingAccountInput<'_>,
    ) -> Result<Option<BillingAccountRecord>, sqlx::Error> {
        let mut transaction = self.pool.begin().await?;
        let account = sqlx::query_as::<_, BillingAccountRecord>(&format!(
            "UPDATE billing_accounts SET username = $3, credit_limit = $4, \
             billing_interval_secs = $5, price_per_interval = $6, enabled = $7, tenant_id = $8, \
             updated_at = now() \
             WHERE id = $1 AND account_type = $2 AND deleted_at IS NULL \
             RETURNING {ACCOUNT_COLUMNS}"
        ))
        .bind(id)
        .bind(account_type.as_str())
        .bind(input.username)
        .bind(input.credit_limit)
        .bind(input.billing_interval_secs)
        .bind(input.price_per_interval)
        .bind(input.enabled)
        .bind(input.tenant_id)
        .fetch_optional(&mut *transaction)
        .await?;
        if account.is_some() {
            // 先解除所有旧关联，再按新列表重建，确保一个中继只绑定一个账户。
            sqlx::query("UPDATE sip_gateways SET account_id = NULL WHERE account_id = $1")
                .bind(id)
                .execute(&mut *transaction)
                .await?;
            link_gateways(&mut transaction, input.gateway_ids, id).await?;
        }
        transaction.commit().await?;
        Ok(account)
    }

    /// 软删除未被网关引用的账户，保留不可变财务流水。
    pub async fn delete_billing_account(
        &self,
        id: i64,
        account_type: BillingAccountType,
    ) -> Result<DeleteBillingAccountOutcome, sqlx::Error> {
        let mut transaction = self.pool.begin().await?;
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM billing_accounts \
             WHERE id = $1 AND account_type = $2 AND deleted_at IS NULL)",
        )
        .bind(id)
        .bind(account_type.as_str())
        .fetch_one(&mut *transaction)
        .await?;
        if !exists {
            transaction.commit().await?;
            return Ok(DeleteBillingAccountOutcome::NotFound);
        }
        let in_use: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sip_gateways WHERE account_id = $1)")
                .bind(id)
                .fetch_one(&mut *transaction)
                .await?;
        if in_use {
            transaction.commit().await?;
            return Ok(DeleteBillingAccountOutcome::InUse);
        }
        sqlx::query(
            "UPDATE billing_accounts SET deleted_at = now(), enabled = FALSE, updated_at = now() \
             WHERE id = $1",
        )
        .bind(id)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(DeleteBillingAccountOutcome::Deleted)
    }

    /// 向已存在的具体账户充值并写入不可变财务流水。
    pub async fn credit_billing_account(
        &self,
        account_id: i64,
        account_type: BillingAccountType,
        amount: Decimal,
        idempotency_key: &str,
        operator_username: &str,
        remark: &str,
    ) -> Result<BillingCreditOutcome, sqlx::Error> {
        let mut transaction = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind(idempotency_key)
            .execute(&mut *transaction)
            .await?;

        if let Some(record) = load_journal_entry(&mut transaction, idempotency_key).await? {
            transaction.commit().await?;
            let same_request = record.account_id == account_id
                && record.amount == amount
                && record.entry_type == "credit";
            return Ok(if same_request {
                BillingCreditOutcome::Replayed(record)
            } else {
                BillingCreditOutcome::Conflict
            });
        }

        let account: Option<(String, String, Decimal)> = sqlx::query_as(
            "SELECT username, account_type, balance FROM billing_accounts \
             WHERE id = $1 AND account_type = $2 AND deleted_at IS NULL AND enabled = TRUE FOR UPDATE",
        )
        .bind(account_id)
        .bind(account_type.as_str())
        .fetch_optional(&mut *transaction)
        .await?;
        let Some((username, stored_account_type, balance_before)) = account else {
            transaction.commit().await?;
            return Ok(BillingCreditOutcome::AccountNotFound);
        };
        let balance_after = balance_before + amount;

        sqlx::query("UPDATE billing_accounts SET balance = $2, updated_at = now() WHERE id = $1")
            .bind(account_id)
            .bind(balance_after)
            .execute(&mut *transaction)
            .await?;
        sqlx::query(
            "INSERT INTO billing_credits \
             (idempotency_key, account_id, username, amount, balance_after, operator_username, remark) \
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(idempotency_key)
        .bind(account_id)
        .bind(&username)
        .bind(amount)
        .bind(balance_after)
        .bind(operator_username)
        .bind(remark)
        .execute(&mut *transaction)
        .await?;
        let record = sqlx::query_as::<_, BillingJournalRecord>(
            "INSERT INTO billing_journal (account_id, account_type, entry_type, amount, \
             balance_before, balance_after, operator_username, remark, idempotency_key) \
             VALUES ($1, $2, 'credit', $3, $4, $5, $6, $7, $8) \
             RETURNING id, account_id, $9::TEXT AS username, account_type, entry_type, amount, \
             balance_before, balance_after, call_id, operator_username, \
             occurred_at AS created_at, remark, \
             idempotency_key",
        )
        .bind(account_id)
        .bind(stored_account_type)
        .bind(amount)
        .bind(balance_before)
        .bind(balance_after)
        .bind(operator_username)
        .bind(remark)
        .bind(idempotency_key)
        .bind(username)
        .fetch_one(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(BillingCreditOutcome::Applied(record))
    }
}

/// 将多个网关绑定到指定账户，要求网关未被其他账户占用。
///
/// 传入空列表时直接返回（账户不关联任何中继）。
async fn link_gateways(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    gateway_ids: &[String],
    account_id: i64,
) -> Result<(), sqlx::Error> {
    for raw_id in gateway_ids {
        let gateway_id = raw_id.trim();
        if gateway_id.is_empty() {
            continue;
        }
        let updated = sqlx::query(
            "UPDATE sip_gateways SET account_id = $2 \
             WHERE id = $1 AND (account_id IS NULL OR account_id = $2)",
        )
        .bind(gateway_id)
        .bind(account_id)
        .execute(&mut **transaction)
        .await?;
        if updated.rows_affected() != 1 {
            return Err(sqlx::Error::RowNotFound);
        }
    }
    Ok(())
}
