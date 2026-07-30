//! # 租户数据存储
//!
//! 管理 `tenants` 表的 CRUD 操作，与 `sip-edge/src/tenant/store.rs` 中的
//! 内存加载逻辑（`TenantStore::load_all`）共用同一张表。
//!
//! ## 表结构
//!
//! 见 `schema/sip_tables.rs::CREATE_TENANTS_TABLE_SQL`。关键字段：
//! - `id` (TEXT PK)：租户唯一标识，通常为 UUID 或短字符串
//! - `domain` (TEXT UNIQUE)：SIP From 头中解析的域，作为查表键
//! - `billing_account_id` (BIGINT, nullable)：关联的计费账户 ID
//!   （`billing_accounts.id`），用于将租户的呼叫统一计费到此账户
//! - `allowed_gateway_ids` (JSONB, nullable)：网关白名单
//! - `enabled` (BOOLEAN)：是否加载到内存注册表

use crate::PostgresCdrStore;
use sqlx::Row;
use time::OffsetDateTime;

/// 租户数据库记录（对齐 `tenants` 表）。
///
/// 与 `sip_edge::tenant::store::TenantRecord` 字段保持一致，
/// 但此处面向 API 层提供完整 CRUD 支持，包含时间戳与所有策略字段。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
pub struct TenantRecord {
    pub id: String,
    pub name: String,
    pub domain: String,
    pub max_concurrent_calls: i32,
    pub max_cps: i32,
    pub cross_tenant_policy: String,
    pub recording_enabled: Option<bool>,
    pub allowed_gateway_ids: Option<serde_json::Value>,
    pub billing_account_id: Option<i64>,
    pub enabled: bool,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

/// 创建/更新租户时的输入参数。
///
/// `id` 在创建时由调用方指定（UUID 或业务编码），更新时不允许修改。
#[derive(Debug, Clone, serde::Deserialize)]
pub struct UpsertTenantInput {
    pub name: String,
    pub domain: String,
    #[serde(default)]
    pub max_concurrent_calls: i32,
    #[serde(default)]
    pub max_cps: i32,
    #[serde(default = "default_cross_tenant_policy")]
    pub cross_tenant_policy: String,
    #[serde(default)]
    pub recording_enabled: Option<bool>,
    #[serde(default)]
    pub allowed_gateway_ids: Option<Vec<String>>,
    #[serde(default)]
    pub billing_account_id: Option<i64>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_cross_tenant_policy() -> String {
    "allow_if_same_domain".to_string()
}

fn default_enabled() -> bool {
    true
}

/// 校验 `cross_tenant_policy` 取值是否合法。
///
/// 合法值：`allow` / `deny` / `allow_if_same_domain`。
/// 不合法时返回错误字符串，由调用方转换为 HTTP 400。
pub fn validate_cross_tenant_policy(policy: &str) -> Result<(), String> {
    match policy {
        "allow" | "deny" | "allow_if_same_domain" => Ok(()),
        other => Err(format!(
            "cross_tenant_policy 仅支持 allow / deny / allow_if_same_domain，收到: {other}"
        )),
    }
}

impl PostgresCdrStore {
    /// 列出所有租户记录（按 domain 升序）。
    ///
    /// 包括 `enabled=FALSE` 的记录，便于管理 API 展示已禁用的租户。
    pub async fn list_tenants(&self) -> Result<Vec<TenantRecord>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT id, name, domain, max_concurrent_calls, max_cps, cross_tenant_policy, \
             recording_enabled, allowed_gateway_ids, billing_account_id, enabled, \
             created_at, updated_at \
             FROM tenants ORDER BY domain ASC",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(parse_tenant_row).collect())
    }

    /// 分页列出租户记录，可选按关键字和启用状态在 SQL 层过滤。
    ///
    /// - `q`：对 name / domain 做大小写不敏感的 LIKE 匹配
    /// - `enabled`：精确匹配 enabled 字段
    ///
    /// 将过滤条件下沉到 SQL，避免分页后内存过滤导致 total 不准、跨页丢数据。
    pub async fn list_tenants_page(
        &self,
        limit: i64,
        offset: i64,
        q: Option<&str>,
        enabled: Option<bool>,
    ) -> Result<Vec<TenantRecord>, sqlx::Error> {
        let like = q.map(|s| format!("%{s}%"));
        let rows = sqlx::query(
            "SELECT id, name, domain, max_concurrent_calls, max_cps, cross_tenant_policy, \
             recording_enabled, allowed_gateway_ids, billing_account_id, enabled, \
             created_at, updated_at \
             FROM tenants \
             WHERE ($3::TEXT IS NULL OR LOWER(name) LIKE LOWER($3) OR LOWER(domain) LIKE LOWER($3)) \
               AND ($4::BOOL IS NULL OR enabled = $4) \
             ORDER BY domain ASC LIMIT $1 OFFSET $2",
        )
        .bind(limit)
        .bind(offset)
        .bind(like)
        .bind(enabled)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(parse_tenant_row).collect())
    }

    /// 返回与 `list_tenants_page` 相同过滤条件下的租户总数。
    pub async fn count_tenants(
        &self,
        q: Option<&str>,
        enabled: Option<bool>,
    ) -> Result<i64, sqlx::Error> {
        let like = q.map(|s| format!("%{s}%"));
        let row: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM tenants \
             WHERE ($1::TEXT IS NULL OR LOWER(name) LIKE LOWER($1) OR LOWER(domain) LIKE LOWER($1)) \
               AND ($2::BOOL IS NULL OR enabled = $2)",
        )
        .bind(like)
        .bind(enabled)
        .fetch_one(&self.pool)
        .await?;
        Ok(row.0)
    }

    /// 按 ID 查询租户记录。
    pub async fn get_tenant(&self, id: &str) -> Result<Option<TenantRecord>, sqlx::Error> {
        let row = sqlx::query(
            "SELECT id, name, domain, max_concurrent_calls, max_cps, cross_tenant_policy, \
             recording_enabled, allowed_gateway_ids, billing_account_id, enabled, \
             created_at, updated_at \
             FROM tenants WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(parse_tenant_row))
    }

    /// 创建新租户。
    ///
    /// `id` 由调用方指定（建议使用 UUID 或业务编码）。
    /// 若 `domain` 已存在则返回唯一约束冲突错误。
    pub async fn create_tenant(
        &self,
        id: &str,
        input: &UpsertTenantInput,
    ) -> Result<TenantRecord, sqlx::Error> {
        validate_cross_tenant_policy(&input.cross_tenant_policy)
            .map_err(|e| sqlx::Error::Configuration(e.into()))?;

        let allowed_gateway_ids_json = input
            .allowed_gateway_ids
            .as_ref()
            .map(|v| serde_json::to_value(v).unwrap_or(serde_json::Value::Null));

        let row = sqlx::query(
            "INSERT INTO tenants (id, name, domain, max_concurrent_calls, max_cps, \
             cross_tenant_policy, recording_enabled, allowed_gateway_ids, billing_account_id, enabled) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) \
             RETURNING id, name, domain, max_concurrent_calls, max_cps, cross_tenant_policy, \
             recording_enabled, allowed_gateway_ids, billing_account_id, enabled, \
             created_at, updated_at",
        )
        .bind(id)
        .bind(&input.name)
        .bind(&input.domain)
        .bind(input.max_concurrent_calls)
        .bind(input.max_cps)
        .bind(&input.cross_tenant_policy)
        .bind(input.recording_enabled)
        .bind(&allowed_gateway_ids_json)
        .bind(input.billing_account_id)
        .bind(input.enabled)
        .fetch_one(&self.pool)
        .await?;
        Ok(parse_tenant_row(row))
    }

    /// 按 ID 更新租户字段（全量更新，不含 `id`）。
    pub async fn update_tenant(
        &self,
        id: &str,
        input: &UpsertTenantInput,
    ) -> Result<Option<TenantRecord>, sqlx::Error> {
        validate_cross_tenant_policy(&input.cross_tenant_policy)
            .map_err(|e| sqlx::Error::Configuration(e.into()))?;

        let allowed_gateway_ids_json = input
            .allowed_gateway_ids
            .as_ref()
            .map(|v| serde_json::to_value(v).unwrap_or(serde_json::Value::Null));

        let row = sqlx::query(
            "UPDATE tenants SET name = $2, domain = $3, max_concurrent_calls = $4, \
             max_cps = $5, cross_tenant_policy = $6, recording_enabled = $7, \
             allowed_gateway_ids = $8, billing_account_id = $9, enabled = $10, \
             updated_at = now() \
             WHERE id = $1 \
             RETURNING id, name, domain, max_concurrent_calls, max_cps, cross_tenant_policy, \
             recording_enabled, allowed_gateway_ids, billing_account_id, enabled, \
             created_at, updated_at",
        )
        .bind(id)
        .bind(&input.name)
        .bind(&input.domain)
        .bind(input.max_concurrent_calls)
        .bind(input.max_cps)
        .bind(&input.cross_tenant_policy)
        .bind(input.recording_enabled)
        .bind(&allowed_gateway_ids_json)
        .bind(input.billing_account_id)
        .bind(input.enabled)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(parse_tenant_row))
    }

    /// 按 ID 删除租户记录。
    ///
    /// 返回 `true` 表示删除成功，`false` 表示记录不存在。
    pub async fn delete_tenant(&self, id: &str) -> Result<bool, sqlx::Error> {
        let affected = sqlx::query("DELETE FROM tenants WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?
            .rows_affected();
        Ok(affected > 0)
    }

    /// 按 domain 查询租户记录（用于校验 domain 唯一性）。
    pub async fn get_tenant_by_domain(
        &self,
        domain: &str,
    ) -> Result<Option<TenantRecord>, sqlx::Error> {
        let row = sqlx::query(
            "SELECT id, name, domain, max_concurrent_calls, max_cps, cross_tenant_policy, \
             recording_enabled, allowed_gateway_ids, billing_account_id, enabled, \
             created_at, updated_at \
             FROM tenants WHERE domain = $1",
        )
        .bind(domain)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(parse_tenant_row))
    }
}

/// 将 sqlx::Row 解析为 TenantRecord。
///
/// 单独提取为函数，避免 `sqlx::FromRow` 对 `Option<serde_json::Value>` 与
/// `OffsetDateTime` 的默认派生在某些 sqlx 版本下产生不兼容。
fn parse_tenant_row(row: sqlx::postgres::PgRow) -> TenantRecord {
    TenantRecord {
        id: row.get("id"),
        name: row.get("name"),
        domain: row.get("domain"),
        max_concurrent_calls: row.get("max_concurrent_calls"),
        max_cps: row.get("max_cps"),
        cross_tenant_policy: row.get("cross_tenant_policy"),
        recording_enabled: row.get("recording_enabled"),
        allowed_gateway_ids: row.get("allowed_gateway_ids"),
        billing_account_id: row.get("billing_account_id"),
        enabled: row.get("enabled"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_policy_accepts_known_values() {
        assert!(validate_cross_tenant_policy("allow").is_ok());
        assert!(validate_cross_tenant_policy("deny").is_ok());
        assert!(validate_cross_tenant_policy("allow_if_same_domain").is_ok());
    }

    #[test]
    fn validate_policy_rejects_unknown_value() {
        let err = validate_cross_tenant_policy("invalid").expect_err("应拒绝非法值");
        assert!(err.contains("invalid"));
    }
}
