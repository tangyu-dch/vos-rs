//! # cdr-core：数据存储层
//!
//! 本 crate 是 VoIP 软交换平台的数据存储层，负责：
//! - CDR 存储与数据模型
//! - 数据库在线表结构自动迁移
//! - 高吞吐量 CDR 异步批处理通道

pub mod batch;
mod migrations;
mod models;
mod schema;
pub mod store;
mod termination_models;
mod termination_schema;
mod utils;

pub use batch::{CdrBatchChannel, CdrBatchConfig};
pub use call_core::CdrAuditSnapshot;
pub use models::*;
pub use store::access_control::*;
pub use store::announcement::{Announcement, AnnouncementSummary, UpsertAnnouncementInput};
pub use store::copilot::{AppendCopilotMessageInput, CopilotMessage, CopilotSession};
pub use store::copilot_action::CopilotAction;
pub use store::llm_config::{LlmConfigRecord, UpsertLlmConfigInput};
pub use store::notification::{
    CreateNotificationInput, Notification, NotificationCategory, NotificationSeverity,
    NotificationSummary,
};
pub use store::tenant::{TenantRecord, UpsertTenantInput};
pub use termination_models::*;
pub use utils::current_hhmm;

use sqlx::{postgres::PgPoolOptions, PgPool};

/// PostgreSQL 数据存储：所有数据访问的入口。
#[derive(Debug, Clone)]
pub struct PostgresCdrStore {
    pub(crate) pool: PgPool,
}

impl PostgresCdrStore {
    pub async fn connect(database_url: &str, max_connections: u32) -> Result<Self, sqlx::Error> {
        let pool = PgPoolOptions::new()
            .max_connections(max_connections)
            .connect(database_url)
            .await?;
        let store = Self { pool };
        store.migrate().await?;
        Ok(store)
    }

    pub async fn migrate(&self) -> Result<(), sqlx::Error> {
        migrations::run_migrations(&self.pool).await
    }

    /// 检查数据库连接是否仍然可用。
    pub async fn ping(&self) -> Result<(), sqlx::Error> {
        sqlx::query("SELECT 1")
            .execute(&self.pool)
            .await
            .map(|_| ())
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// 获取系统配置值。
    pub async fn get_system_config(&self, key: &str) -> Result<Option<String>, sqlx::Error> {
        let row = sqlx::query("SELECT config_value FROM system_configs WHERE config_key = $1")
            .bind(key)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| {
            use sqlx::Row;
            r.get::<String, _>("config_value")
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::*;
    use crate::utils::extract_sip_user;

    #[test]
    fn test_extract_sip_user() {
        assert_eq!(extract_sip_user("<sip:1001@vos-rs>"), Some("1001"));
        assert_eq!(extract_sip_user("sip:1002@host"), Some("1002"));
        assert_eq!(extract_sip_user("sip:;user=phone@host"), None);
        assert_eq!(extract_sip_user("no-sip-here"), None);
    }

    #[test]
    fn test_cdr_event_roundtrip() {
        let event = CdrEvent {
            call_id: "test-123".to_string(),
            caller: Some("sip:1001@host".to_string()),
            callee: Some("sip:1002@host".to_string()),
            started_at_ms: 1000000,
            ringing_at_ms: Some(1000500),
            answered_at_ms: Some(1001000),
            ended_at_ms: 1010000,
            duration_ms: 10000,
            billable_duration_ms: 9000,
            talk_duration_ms: Some(9000),
            ringing_duration_ms: Some(500),
            access_billable_duration_ms: Some(60_000),
            access_charge_amount: Some(0.05),
            egress_billable_duration_ms: Some(60_000),
            egress_cost_amount: Some(0.03),
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
            tenant_id: Some("t1".to_string()),
            tenant_name: Some("Tenant 1".to_string()),
            auth_realm: Some("realm1.com".to_string()),
            audit: call_core::CdrAuditSnapshot {
                source_type: Some("trunk".to_string()),
                source_id: Some("access-a".to_string()),
                billing_account: Some("account-a".to_string()),
                billing_interval_secs: Some(6),
                price_per_interval: Some(0.05),
                ..call_core::CdrAuditSnapshot::default()
            },
        };
        let json = event.to_json_bytes().unwrap();
        let decoded = CdrEvent::from_json_slice(&json).unwrap();
        assert_eq!(event, decoded);
    }

    #[test]
    fn test_legacy_cdr_event_defaults_missing_audit_snapshot() {
        let payload = br#"{
            "call_id":"legacy","caller":null,"callee":null,
            "started_at_ms":1,"answered_at_ms":null,"ended_at_ms":2,
            "duration_ms":1,"billable_duration_ms":0,"status":"failed",
            "failure_status_code":null,"failure_reason":null,
            "caller_rtcp_loss_rate":null,"caller_rtcp_jitter_ms":null,"caller_rtcp_rtt_ms":null,
            "gateway_rtcp_loss_rate":null,"gateway_rtcp_jitter_ms":null,"gateway_rtcp_rtt_ms":null,
            "mos":null,"dtmf_digits":null,"recording_path":null,"direction":"outbound"
        }"#;
        let decoded = CdrEvent::from_json_slice(payload).unwrap();
        assert_eq!(decoded.audit, call_core::CdrAuditSnapshot::default());
    }

    #[test]
    fn test_runtime_schema_contains_gateway_and_number_domain_columns() {
        for column in [
            "gateway_type",
            "reg_auth_type",
            "reg_password",
            "parent_gateway_id",
            "account_id BIGINT",
            "tenant_id TEXT",
            "enabled BOOLEAN",
        ] {
            assert!(CREATE_SIP_GATEWAYS_TABLE_SQL.contains(column));
        }
        for column in [
            "gateway_id TEXT",
            "direction VARCHAR",
            "current_concurrent INTEGER",
            "updated_at TIMESTAMPTZ",
        ] {
            assert!(CREATE_NUMBER_INVENTORY_TABLE_SQL.contains(column));
        }
    }

    #[test]
    fn test_billing_account_schema_separates_account_types_and_keeps_journal_time() {
        assert!(CREATE_BILLING_ACCOUNTS_TABLE_SQL.contains("account_type TEXT"));
        assert!(CREATE_BILLING_ACCOUNTS_TABLE_SQL.contains("billing_interval_secs"));
        assert!(CREATE_BILLING_JOURNAL_SQL.contains("occurred_at TIMESTAMPTZ"));
        for entry_type in [
            "credit",
            "call_charge",
            "call_cost",
            "adjustment",
            "refund",
            "reversal",
        ] {
            assert!(CREATE_BILLING_JOURNAL_SQL.contains(entry_type));
        }
    }

    #[test]
    fn test_domain_migrations_are_non_destructive() {
        let migrations = MIGRATE_SIP_GATEWAYS_SQL
            .iter()
            .chain(MIGRATE_BILLING_ACCOUNTS_SQL)
            .chain(MIGRATE_NUMBER_INVENTORY_SQL);
        for migration in migrations {
            let normalized = migration.to_ascii_uppercase();
            assert!(!normalized.contains("DROP TABLE"));
            assert!(!normalized.contains("TRUNCATE"));
            assert!(!normalized.contains("DELETE FROM"));
        }
    }

    #[test]
    fn test_legacy_anti_fraud_value_no_longer_blocks_current_writes() {
        let normalized = MIGRATE_LEGACY_ANTI_FRAUD_RULES_SQL.to_ascii_uppercase();
        assert!(normalized.contains("ALTER COLUMN VALUE DROP NOT NULL"));
        assert!(normalized.contains("COLUMN_NAME = 'VALUE'"));
    }
}
