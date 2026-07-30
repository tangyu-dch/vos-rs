use crate::{
    BillingAccountRecord, BillingAccountType, BillingJournalFilter, BillingJournalRecord,
    PostgresCdrStore, TenantAccountSummary,
};

pub(super) const ACCOUNT_COLUMNS: &str = "id, username, account_type, balance, credit_limit, \
    billing_interval_secs, price_per_interval, enabled, tenant_id, created_at, updated_at";

const JOURNAL_LIST_SQL: &str = "SELECT journal.id, journal.account_id, accounts.username, \
    journal.account_type, journal.entry_type, journal.amount, journal.balance_before, \
    journal.balance_after, journal.call_id, journal.operator_username, \
    journal.occurred_at AS created_at, \
    journal.remark, journal.idempotency_key FROM billing_journal journal \
    JOIN billing_accounts accounts ON accounts.id = journal.account_id \
    WHERE ($1::TEXT IS NULL OR journal.account_type = $1) \
    AND ($2::BIGINT IS NULL OR journal.account_id = $2) \
    AND ($3::TEXT IS NULL OR journal.entry_type = $3) \
    AND ($4::TIMESTAMPTZ IS NULL OR journal.occurred_at >= $4) \
    AND ($5::TIMESTAMPTZ IS NULL OR journal.occurred_at <= $5) \
    AND ($6::TEXT IS NULL OR LOWER(accounts.username) LIKE LOWER($6) \
        OR LOWER(journal.operator_username) LIKE LOWER($6)) \
    ORDER BY journal.occurred_at DESC LIMIT $7 OFFSET $8";

const JOURNAL_BY_KEY_SQL: &str = "SELECT journal.id, journal.account_id, accounts.username, \
    journal.account_type, journal.entry_type, journal.amount, journal.balance_before, \
    journal.balance_after, journal.call_id, journal.operator_username, \
    journal.occurred_at AS created_at, \
    journal.remark, journal.idempotency_key FROM billing_journal journal \
    JOIN billing_accounts accounts ON accounts.id = journal.account_id \
    WHERE journal.idempotency_key = $1";

impl PostgresCdrStore {
    /// 按类型分页查询未删除的计费账户。
    pub async fn list_billing_accounts_page(
        &self,
        account_type: BillingAccountType,
        limit: i64,
        offset: i64,
        query: Option<&str>,
    ) -> Result<Vec<BillingAccountRecord>, sqlx::Error> {
        let pattern = query.map(|value| format!("%{value}%"));
        sqlx::query_as::<_, BillingAccountRecord>(&format!(
            "SELECT {ACCOUNT_COLUMNS} FROM billing_accounts \
             WHERE account_type = $1 AND deleted_at IS NULL \
             AND ($4::TEXT IS NULL OR LOWER(username) LIKE LOWER($4)) \
             ORDER BY username LIMIT $2 OFFSET $3"
        ))
        .bind(account_type.as_str())
        .bind(limit)
        .bind(offset)
        .bind(pattern)
        .fetch_all(&self.pool)
        .await
    }

    /// 统计指定类型且未删除的计费账户数量。
    pub async fn count_billing_accounts(
        &self,
        account_type: BillingAccountType,
        query: Option<&str>,
    ) -> Result<i64, sqlx::Error> {
        let pattern = query.map(|value| format!("%{value}%"));
        sqlx::query_scalar(
            "SELECT COUNT(*) FROM billing_accounts \
             WHERE account_type = $1 AND deleted_at IS NULL \
             AND ($2::TEXT IS NULL OR LOWER(username) LIKE LOWER($2))",
        )
        .bind(account_type.as_str())
        .bind(pattern)
        .fetch_one(&self.pool)
        .await
    }

    /// 分页查询不可变财务流水，可按类型、账户和时间过滤。
    pub async fn list_billing_journal_page(
        &self,
        filter: &BillingJournalFilter<'_>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<BillingJournalRecord>, sqlx::Error> {
        sqlx::query_as(JOURNAL_LIST_SQL)
            .bind(filter.account_type.map(BillingAccountType::as_str))
            .bind(filter.account_id)
            .bind(filter.entry_type)
            .bind(filter.start_time)
            .bind(filter.end_time)
            .bind(filter.query.map(|value| format!("%{value}%")))
            .bind(limit)
            .bind(offset)
            .fetch_all(&self.pool)
            .await
    }

    /// 统计与财务流水筛选条件匹配的记录数。
    pub async fn count_billing_journal(
        &self,
        filter: &BillingJournalFilter<'_>,
    ) -> Result<i64, sqlx::Error> {
        sqlx::query_scalar(
            "SELECT COUNT(*) FROM billing_journal journal \
             WHERE ($1::TEXT IS NULL OR journal.account_type = $1) \
             AND ($2::BIGINT IS NULL OR journal.account_id = $2) \
             AND ($3::TEXT IS NULL OR journal.entry_type = $3) \
             AND ($4::TIMESTAMPTZ IS NULL OR journal.occurred_at >= $4) \
             AND ($5::TIMESTAMPTZ IS NULL OR journal.occurred_at <= $5) \
             AND ($6::TEXT IS NULL OR EXISTS(SELECT 1 FROM billing_accounts accounts \
                 WHERE accounts.id = journal.account_id AND (LOWER(accounts.username) LIKE LOWER($6) \
                 OR LOWER(journal.operator_username) LIKE LOWER($6))))",
        )
        .bind(filter.account_type.map(BillingAccountType::as_str))
        .bind(filter.account_id)
        .bind(filter.entry_type)
        .bind(filter.start_time)
        .bind(filter.end_time)
        .bind(filter.query.map(|value| format!("%{value}%")))
        .fetch_one(&self.pool)
        .await
    }

    /// 验证网关关联账户存在、启用且类型匹配。
    pub async fn billing_account_matches_type(
        &self,
        account_id: i64,
        account_type: BillingAccountType,
    ) -> Result<bool, sqlx::Error> {
        sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM billing_accounts WHERE id = $1 AND account_type = $2 \
             AND deleted_at IS NULL AND enabled = TRUE)",
        )
        .bind(account_id)
        .bind(account_type.as_str())
        .fetch_one(&self.pool)
        .await
    }

    /// 按账户 ID 与类型查询 username（用于删除前获取缓存键）。
    pub async fn get_billing_account_username(
        &self,
        account_id: i64,
        account_type: BillingAccountType,
    ) -> Result<Option<String>, sqlx::Error> {
        sqlx::query_scalar(
            "SELECT username FROM billing_accounts WHERE id = $1 AND account_type = $2",
        )
        .bind(account_id)
        .bind(account_type.as_str())
        .fetch_optional(&self.pool)
        .await
    }

    /// 验证租户存在且已启用。
    pub async fn active_tenant_exists(&self, tenant_id: &str) -> Result<bool, sqlx::Error> {
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM tenants WHERE id = $1 AND enabled = TRUE)")
            .bind(tenant_id)
            .fetch_one(&self.pool)
            .await
    }

    /// 验证网关可绑定到指定账户类型；对接网关同时必须有启用租户。
    pub async fn gateway_available_for_account(
        &self,
        gateway_id: &str,
        account_type: BillingAccountType,
        current_account_id: Option<i64>,
    ) -> Result<bool, sqlx::Error> {
        sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM sip_gateways gateway \
             LEFT JOIN tenants tenant ON tenant.id = gateway.tenant_id \
             WHERE gateway.id = $1 AND gateway.role = $2 \
             AND (gateway.account_id IS NULL OR gateway.account_id = $3) \
             AND ($2 <> 'access' OR (gateway.tenant_id IS NOT NULL AND tenant.enabled = TRUE)))",
        )
        .bind(gateway_id)
        .bind(account_type.as_str())
        .bind(current_account_id)
        .fetch_one(&self.pool)
        .await
    }

    /// 查询租户下所有未删除的对接账户完整记录（按用户名升序）。
    pub async fn list_access_accounts_by_tenant(
        &self,
        tenant_id: &str,
    ) -> Result<Vec<BillingAccountRecord>, sqlx::Error> {
        sqlx::query_as::<_, BillingAccountRecord>(&format!(
            "SELECT {ACCOUNT_COLUMNS} FROM billing_accounts \
             WHERE tenant_id = $1 AND account_type = 'access' AND deleted_at IS NULL \
             ORDER BY username ASC"
        ))
        .bind(tenant_id)
        .fetch_all(&self.pool)
        .await
    }

    /// 批量按租户 ID 列表加载对接账户摘要（用于租户列表展示总余额与欠费状态）。
    ///
    /// 返回扁平的 `(tenant_id, TenantAccountSummary)` 元组列表，调用方按 tenant_id 分组。
    pub async fn list_account_summaries_by_tenants(
        &self,
        tenant_ids: &[String],
    ) -> Result<Vec<(String, TenantAccountSummary)>, sqlx::Error> {
        if tenant_ids.is_empty() {
            return Ok(Vec::new());
        }
        let rows = sqlx::query(
            "SELECT tenant_id, id, username, balance, credit_limit, price_per_interval, enabled \
             FROM billing_accounts \
             WHERE tenant_id = ANY($1) AND account_type = 'access' AND deleted_at IS NULL \
             ORDER BY tenant_id, username",
        )
        .bind(tenant_ids)
        .fetch_all(&self.pool)
        .await?;
        use sqlx::Row;
        rows.into_iter()
            .map(|row| {
                Ok::<_, sqlx::Error>((
                    row.get::<String, _>("tenant_id"),
                    TenantAccountSummary {
                        id: row.get("id"),
                        username: row.get("username"),
                        balance: row.get("balance"),
                        credit_limit: row.get("credit_limit"),
                        price_per_interval: row.get("price_per_interval"),
                        enabled: row.get("enabled"),
                    },
                ))
            })
            .collect()
    }

    /// 校验租户存在且启用（用于账户关联租户时的校验）。
    pub async fn tenant_exists_and_enabled(
        &self,
        tenant_id: Option<&str>,
    ) -> Result<bool, sqlx::Error> {
        match tenant_id {
            Some(id) if !id.is_empty() => {
                sqlx::query_scalar(
                    "SELECT EXISTS(SELECT 1 FROM tenants WHERE id = $1 AND enabled = TRUE)",
                )
                .bind(id)
                .fetch_one(&self.pool)
                .await
            }
            _ => Ok(true),
        }
    }
}

pub(super) async fn load_journal_entry(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    idempotency_key: &str,
) -> Result<Option<BillingJournalRecord>, sqlx::Error> {
    sqlx::query_as(JOURNAL_BY_KEY_SQL)
        .bind(idempotency_key)
        .fetch_optional(&mut **transaction)
        .await
}
