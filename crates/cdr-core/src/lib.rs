//! # cdr-core：数据存储层
//!
//! 本 crate 是 VoIP 软交换平台的数据存储层，负责：
//!
//! - **CDR 存储**：通话详单（Call Detail Record）的持久化
//! - **网关管理**：网关配置和健康状态的 CRUD
//! - **路由管理**：路由规则 of the CRUD
//! - **用户管理**：SIP 用户的 CRUD
//! - **计费管理**：费率、账户、账本、实时计费、离线对账
//! - **注册管理**：SIP REGISTER 绑定的存储
//! - **反欺诈**：反欺诈规则的 CRUD
//! - **号码库存**：号码资源的 CRUD
//! - **数据迁移**：数据库表结构自动迁移
//!
//! ## 数据库表
//!
//! | 表名 | 用途 |
//! |------|------|
//! | `call_cdrs` | 通话详单 |
//! | `sip_gateways` | 网关配置 |
//! | `sip_routes` | 路由规则 |
//! | `sip_users` | SIP 用户 |
//! | `sip_registrations` | 注册绑定 |
//! | `billing_rates` | 费率表 |
//! | `billing_accounts` | 计费账户 |
//! | `billing_ledger` | 扣费流水 |
//! | `gateway_health_status` | 网关健康状态 |
//! | `anti_fraud_rules` | 反欺诈规则 |
//! | `dtmf_events` | DTMF 事件 |
//! | `number_inventory` | 号码库存 |
//!
//! ## 设计原则
//!
//! - 使用 `sqlx` 编译期 SQL 检查
//! - 所有方法返回 `Result`，不使用 panic
//! - 在线迁移（`ALTER TABLE ... ADD COLUMN IF NOT EXISTS`）
//! - 实时计费使用事务保证原子性
//!

mod models;
mod schema;
pub mod store;
mod utils;

pub use models::*;
pub use utils::current_hhmm;

use sqlx::{postgres::PgPoolOptions, PgPool};

use schema::*;

/// PostgreSQL 数据存储：所有数据访问的入口。
///
/// 封装了 PostgreSQL 连接池，提供所有数据表的 CRUD 操作。
/// 使用 `sqlx` 编译期 SQL 检查，确保类型安全。
#[derive(Debug, Clone)]
pub struct PostgresCdrStore {
    pub(crate) pool: PgPool,
}

impl PostgresCdrStore {
    pub async fn connect(database_url: &str) -> Result<Self, sqlx::Error> {
        let pool = PgPoolOptions::new()
            .max_connections(10)
            .connect(database_url)
            .await?;
        let store = Self { pool };
        store.migrate().await?;
        Ok(store)
    }

    pub async fn migrate(&self) -> Result<(), sqlx::Error> {
        sqlx::query(CREATE_CDR_TABLE_SQL)
            .execute(&self.pool)
            .await?;
        sqlx::query(CREATE_CALL_ID_INDEX_SQL)
            .execute(&self.pool)
            .await?;
        // CDR 可能因 NATS 重投或 ACK 失败重复到达，数据库约束是最终幂等边界。
        sqlx::query(MIGRATE_CDR_IDEMPOTENCY_SQL)
            .execute(&self.pool)
            .await?;
        sqlx::query(CREATE_STARTED_AT_INDEX_SQL)
            .execute(&self.pool)
            .await?;
        sqlx::query(CREATE_STATUS_INDEX_SQL)
            .execute(&self.pool)
            .await?;
        sqlx::query(CREATE_CDR_CALLER_INDEX_SQL)
            .execute(&self.pool)
            .await?;
        sqlx::query(CREATE_CDR_CALLEE_INDEX_SQL)
            .execute(&self.pool)
            .await?;
        sqlx::query(CREATE_SIP_USERS_TABLE_SQL)
            .execute(&self.pool)
            .await?;
        sqlx::query(CREATE_SIP_GATEWAYS_TABLE_SQL)
            .execute(&self.pool)
            .await?;
        sqlx::query(CREATE_SIP_ROUTES_TABLE_SQL)
            .execute(&self.pool)
            .await?;
        sqlx::query(CREATE_ROUTES_PRIORITY_INDEX_SQL)
            .execute(&self.pool)
            .await?;
        sqlx::query(MIGRATION_ADD_ROUTE_WEIGHT)
            .execute(&self.pool)
            .await?;
        sqlx::query(CREATE_SIP_REGISTRATIONS_TABLE_SQL)
            .execute(&self.pool)
            .await?;
        sqlx::query("ALTER TABLE sip_registrations ADD COLUMN IF NOT EXISTS path TEXT")
            .execute(&self.pool)
            .await?;
        sqlx::query(CREATE_REGISTRATIONS_EXPIRES_INDEX_SQL)
            .execute(&self.pool)
            .await?;
        sqlx::query("ALTER TABLE sip_gateways ADD COLUMN IF NOT EXISTS max_capacity INTEGER")
            .execute(&self.pool)
            .await?;
        sqlx::query(
            "ALTER TABLE sip_routes ADD COLUMN IF NOT EXISTS cost DOUBLE PRECISION NOT NULL DEFAULT 0.0",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query("ALTER TABLE sip_routes ADD COLUMN IF NOT EXISTS time_start TEXT")
            .execute(&self.pool)
            .await?;
        sqlx::query("ALTER TABLE sip_routes ADD COLUMN IF NOT EXISTS time_end TEXT")
            .execute(&self.pool)
            .await?;
        sqlx::query(CREATE_DTMF_EVENTS_TABLE_SQL)
            .execute(&self.pool)
            .await?;
        sqlx::query(CREATE_DTMF_CALL_ID_INDEX_SQL)
            .execute(&self.pool)
            .await?;
        sqlx::query(CREATE_BILLING_RATES_TABLE_SQL)
            .execute(&self.pool)
            .await?;
        sqlx::query(CREATE_BILLING_ACCOUNTS_TABLE_SQL)
            .execute(&self.pool)
            .await?;
        sqlx::query(CREATE_BILLING_LEDGER_TABLE_SQL)
            .execute(&self.pool)
            .await?;
        sqlx::query(CREATE_LEDGER_USERNAME_INDEX_SQL)
            .execute(&self.pool)
            .await?;
        sqlx::query(CREATE_LEDGER_CREATED_AT_INDEX_SQL)
            .execute(&self.pool)
            .await?;
        sqlx::query(CREATE_NUMBER_INVENTORY_TABLE_SQL)
            .execute(&self.pool)
            .await?;
        sqlx::query(CREATE_GATEWAY_HEALTH_TABLE_SQL)
            .execute(&self.pool)
            .await?;
        sqlx::query(CREATE_GATEWAY_HEALTH_STATE_INDEX_SQL)
            .execute(&self.pool)
            .await?;
        sqlx::query("ALTER TABLE gateway_health_status ADD COLUMN IF NOT EXISTS state TEXT NOT NULL DEFAULT 'closed'")
            .execute(&self.pool)
            .await?;
        sqlx::query("ALTER TABLE gateway_health_status ADD COLUMN IF NOT EXISTS last_failure_at TIMESTAMPTZ")
            .execute(&self.pool)
            .await?;
        sqlx::query("ALTER TABLE gateway_health_status ADD COLUMN IF NOT EXISTS half_open_successes INTEGER NOT NULL DEFAULT 0")
            .execute(&self.pool)
            .await?;
        sqlx::query(
            "ALTER TABLE gateway_health_status ADD COLUMN IF NOT EXISTS last_probe_at TIMESTAMPTZ",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query("ALTER TABLE gateway_health_status ADD COLUMN IF NOT EXISTS active_calls INTEGER NOT NULL DEFAULT 0")
            .execute(&self.pool)
            .await?;
        sqlx::query(CREATE_ANTI_FRAUD_RULES_TABLE_SQL)
            .execute(&self.pool)
            .await?;
        sqlx::query(MIGRATE_LEGACY_ANTI_FRAUD_RULES_SQL)
            .execute(&self.pool)
            .await?;
        sqlx::query(MIGRATE_LEGACY_ANTI_FRAUD_RULES_STEP2_SQL)
            .execute(&self.pool)
            .await?;
        sqlx::query(MIGRATE_LEGACY_ANTI_FRAUD_RULES_STEP3_SQL)
            .execute(&self.pool)
            .await?;
        sqlx::query(MIGRATE_LEGACY_ANTI_FRAUD_RULES_STEP4_SQL)
            .execute(&self.pool)
            .await?;
        sqlx::query(CREATE_ANTI_FRAUD_CONFIG_TABLE_SQL)
            .execute(&self.pool)
            .await?;
        sqlx::query(SEED_ANTI_FRAUD_CONFIG_SQL)
            .execute(&self.pool)
            .await?;
        sqlx::query(CREATE_AUDIT_LOGS_TABLE_SQL)
            .execute(&self.pool)
            .await?;
        sqlx::query(CREATE_AUDIT_LOGS_INDEX_SQL)
            .execute(&self.pool)
            .await?;
        // 迁移：添加审计日志的 query_params 和 request_body 列
        sqlx::query("ALTER TABLE api_audit_logs ADD COLUMN IF NOT EXISTS query_params TEXT")
            .execute(&self.pool)
            .await?;
        sqlx::query("ALTER TABLE api_audit_logs ADD COLUMN IF NOT EXISTS request_body TEXT")
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// 检查数据库连接是否仍然可用，用于服务就绪探针。
    pub async fn ping(&self) -> Result<(), sqlx::Error> {
        sqlx::query("SELECT 1")
            .execute(&self.pool)
            .await
            .map(|_| ())
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::{extract_sip_user, match_rate};

    #[test]
    fn test_extract_sip_user() {
        assert_eq!(extract_sip_user("<sip:1001@vos-rs>"), Some("1001"));
        assert_eq!(extract_sip_user("sip:1002@host"), Some("1002"));
        assert_eq!(extract_sip_user("sip:;user=phone@host"), None);
        assert_eq!(extract_sip_user("no-sip-here"), None);
    }

    #[test]
    fn test_match_rate() {
        let rates = vec![("86".to_string(), 0.5), ("8613".to_string(), 0.3)];
        assert_eq!(match_rate("8613800138000", &rates), 0.3);
        assert_eq!(match_rate("861012345678", &rates), 0.5);
        assert_eq!(match_rate("12345", &rates), 0.0);
    }

    #[test]
    fn test_cdr_event_roundtrip() {
        let event = CdrEvent {
            call_id: "test-123".to_string(),
            caller: Some("sip:1001@host".to_string()),
            callee: Some("sip:1002@host".to_string()),
            started_at_ms: 1000000,
            answered_at_ms: Some(1001000),
            ended_at_ms: 1010000,
            duration_ms: 10000,
            billable_duration_ms: 9000,
            status: "answered".to_string(),
            failure_status_code: None,
            failure_reason: None,
            caller_rtcp_loss_rate: None,
            caller_rtcp_jitter_ms: None,
            caller_rtcp_rtt_ms: None,
            gateway_rtcp_loss_rate: None,
            gateway_rtcp_jitter_ms: None,
            gateway_rtcp_rtt_ms: None,
            mos: Some(4.5),
            dtmf_digits: None,
            recording_path: None,
            direction: "outbound".to_string(),
        };
        let json = event.to_json_bytes();
        let decoded = CdrEvent::from_json_slice(&json).unwrap();
        assert_eq!(event, decoded);
    }
}
