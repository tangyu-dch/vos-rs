//! 租户持久化存储（PostgreSQL）。
//!
//! 对应数据库表 `tenants`：
//! ```sql
//! CREATE TABLE IF NOT EXISTS tenants (
//!     id TEXT PRIMARY KEY,
//!     name TEXT NOT NULL,
//!     domain TEXT UNIQUE NOT NULL,
//!     max_concurrent_calls INTEGER NOT NULL DEFAULT 0,
//!     max_cps INTEGER NOT NULL DEFAULT 0,
//!     cross_tenant_policy TEXT NOT NULL DEFAULT 'allow_if_same_domain',
//!     recording_enabled BOOLEAN,
//!     allowed_gateway_ids JSONB,
//!     billing_account_id BIGINT,
//!     enabled BOOLEAN NOT NULL DEFAULT TRUE,
//!     created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
//!     updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
//! );
//! ```
//!
//! 迁移脚本应单独执行（见 scripts/ 目录）。

use super::policy::{CrossTenantPolicy, TenantPolicy};
use sqlx::{PgPool, Row};
use std::collections::HashMap;
use tracing::warn;

/// 租户数据库记录。
#[derive(Debug, Clone)]
pub struct TenantRecord {
    pub id: String,
    pub name: String,
    pub domain: String,
    pub policy: TenantPolicy,
    /// 是否启用（仅当为 true 时才会被加载到内存注册表）。
    pub enabled: bool,
}

impl TenantRecord {
    /// 是否处于启用状态（DB 中 `enabled = TRUE`）。
    ///
    /// 由于 `TenantStore::load_all` 仅返回 `enabled = TRUE` 的记录，
    /// 内存注册表中的所有 `TenantRecord` 此值均为 `true`。
    /// 在 `TenantRegistry::context_for_domain` 中做防御性检查，
    /// 为未来支持热更新单条记录的场景预留。
    pub fn is_active(&self) -> bool {
        self.enabled
    }
}

/// 租户存储：从 PostgreSQL 加载所有启用的租户到内存。
#[derive(Debug, Clone)]
pub struct TenantStore {
    pool: PgPool,
}

impl TenantStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// 加载所有启用的租户记录。
    ///
    /// 失败时返回空 HashMap 并记录警告，调用方应使用空表降级（所有呼叫走默认策略）。
    pub async fn load_all(&self) -> HashMap<String, TenantRecord> {
        let rows = match sqlx::query(
            r#"
            SELECT id, name, domain, max_concurrent_calls, max_cps,
                   cross_tenant_policy, recording_enabled,
                   allowed_gateway_ids, billing_account_id, enabled
            FROM tenants
            WHERE enabled = TRUE
            "#,
        )
        .fetch_all(&self.pool)
        .await
        {
            Ok(rows) => rows,
            Err(error) => {
                warn!(%error, "failed to load tenants from database");
                return HashMap::new();
            }
        };

        let mut map = HashMap::with_capacity(rows.len());
        for row in rows {
            let id: String = row.try_get("id").unwrap_or_default();
            let name: String = row.try_get("name").unwrap_or_default();
            let domain: String = row.try_get("domain").unwrap_or_default();
            let max_concurrent_calls: i32 = row.try_get("max_concurrent_calls").unwrap_or(0);
            let max_cps: i32 = row.try_get("max_cps").unwrap_or(0);
            let cross_tenant_policy_str: String = row
                .try_get("cross_tenant_policy")
                .unwrap_or_else(|_| "allow_if_same_domain".to_string());
            let recording_enabled: Option<bool> = row.try_get("recording_enabled").ok();
            let allowed_gateway_ids: Option<serde_json::Value> =
                row.try_get("allowed_gateway_ids").ok();
            let billing_account_id: Option<i64> = row.try_get("billing_account_id").ok();
            let enabled: bool = row.try_get("enabled").unwrap_or(true);

            let cross_tenant_policy = match cross_tenant_policy_str.as_str() {
                "allow" => CrossTenantPolicy::Allow,
                "deny" => CrossTenantPolicy::Deny,
                _ => CrossTenantPolicy::AllowIfSameDomain,
            };

            let allowed_gateway_ids =
                allowed_gateway_ids.and_then(|v| serde_json::from_value::<Vec<String>>(v).ok());

            let policy = TenantPolicy {
                max_concurrent_calls: max_concurrent_calls.max(0) as u32,
                max_cps: max_cps.max(0) as u32,
                cross_tenant_policy,
                recording_enabled,
                allowed_gateway_ids,
                billing_account_id,
            };

            map.insert(
                domain.to_ascii_lowercase(),
                TenantRecord {
                    id,
                    name,
                    domain,
                    policy,
                    enabled,
                },
            );
        }
        map
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tenant_record_clones_correctly() {
        let record = TenantRecord {
            id: "t-001".to_string(),
            name: "Acme".to_string(),
            domain: "acme.com".to_string(),
            policy: TenantPolicy::default(),
            enabled: true,
        };
        let cloned = record.clone();
        assert_eq!(cloned.id, record.id);
        assert_eq!(cloned.domain, record.domain);
    }
}
