//! # cdr-core：数据存储层
//!
//! 本 crate 是 VoIP 软交换平台的数据存储层，负责：
//!
//! - **CDR 存储**：通话详单（Call Detail Record）的持久化
//! - **网关管理**：网关配置和健康状态的 CRUD
//! - **路由管理**：路由规则的 CRUD
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

mod models;
mod schema;
mod utils;

pub use models::*;
pub use utils::current_hhmm;

use sqlx::{postgres::PgPoolOptions, PgPool, Row};
use time::OffsetDateTime;
use tracing::warn;

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
        sqlx::query(CREATE_STARTED_AT_INDEX_SQL)
            .execute(&self.pool)
            .await?;
        sqlx::query(CREATE_STATUS_INDEX_SQL)
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
        sqlx::query(MIGRATION_ADD_ROUTE_WEIGHT)
            .execute(&self.pool)
            .await?;
        sqlx::query("ALTER TABLE sip_registrations ADD COLUMN IF NOT EXISTS path TEXT")
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
        sqlx::query(CREATE_NUMBER_INVENTORY_TABLE_SQL)
            .execute(&self.pool)
            .await?;
        sqlx::query(CREATE_SIP_REGISTRATIONS_TABLE_SQL)
            .execute(&self.pool)
            .await?;
        sqlx::query(CREATE_GATEWAY_HEALTH_TABLE_SQL)
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
        Ok(())
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// 持久化一条管理 API 审计记录。
    pub async fn insert_audit_log(&self, input: &AuditLogInput<'_>) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO api_audit_logs (request_id, username, role, method, path, status_code, source_ip) \
             VALUES ($1, $2, $3, $4, $5, $6, $7::inet)",
        )
        .bind(input.request_id)
        .bind(input.username)
        .bind(input.role)
        .bind(input.method)
        .bind(input.path)
        .bind(i32::from(input.status_code))
        .bind(input.source_ip)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// 分页查询管理 API 审计日志。
    pub async fn list_audit_logs(
        &self,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<AuditLog>, sqlx::Error> {
        sqlx::query_as::<_, AuditLog>(
            "SELECT id, request_id, username, role, method, path, status_code, \
                    host(source_ip) AS source_ip, created_at \
             FROM api_audit_logs ORDER BY created_at DESC, id DESC LIMIT $1 OFFSET $2",
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await
    }

    // ===== CDR =====

    pub async fn insert_call_cdr(&self, cdr: &call_core::CallCdr) -> Result<(), sqlx::Error> {
        self.insert_event(&CdrEvent::from_call_cdr(cdr)).await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn save_gateway_health(
        &self,
        gateway_id: &str,
        circuit_open: bool,
        consecutive_failures: i32,
        state: &str,
        last_failure_at: Option<OffsetDateTime>,
        half_open_successes: i32,
        last_probe_at: Option<OffsetDateTime>,
        active_calls: i32,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO gateway_health_status \
             (gateway_id, circuit_open, consecutive_failures, state, last_failure_at, half_open_successes, last_probe_at, active_calls, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, now()) \
             ON CONFLICT (gateway_id) DO UPDATE \
             SET circuit_open = EXCLUDED.circuit_open, \
                 consecutive_failures = EXCLUDED.consecutive_failures, \
                 state = EXCLUDED.state, \
                 last_failure_at = EXCLUDED.last_failure_at, \
                 half_open_successes = EXCLUDED.half_open_successes, \
                 last_probe_at = EXCLUDED.last_probe_at, \
                 active_calls = EXCLUDED.active_calls, \
                 updated_at = now()",
        )
        .bind(gateway_id)
        .bind(circuit_open)
        .bind(consecutive_failures)
        .bind(state)
        .bind(last_failure_at)
        .bind(half_open_successes)
        .bind(last_probe_at)
        .bind(active_calls)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn load_gateway_health_list(
        &self,
    ) -> Result<
        Vec<(
            String,
            bool,
            i32,
            String,
            Option<OffsetDateTime>,
            i32,
            Option<OffsetDateTime>,
            i32,
        )>,
        sqlx::Error,
    > {
        let rows = sqlx::query(
            "SELECT gateway_id, circuit_open, consecutive_failures, state, last_failure_at, half_open_successes, last_probe_at, active_calls \
             FROM gateway_health_status",
        )
        .fetch_all(&self.pool)
        .await?;

        let mut list = Vec::new();
        for row in rows {
            let id: String = row.get(0);
            let open: bool = row.get(1);
            let failures: i32 = row.get(2);
            let state: String = row.get(3);
            let last_failure_at: Option<OffsetDateTime> = row.get(4);
            let half_open_successes: i32 = row.get(5);
            let last_probe_at: Option<OffsetDateTime> = row.get(6);
            let active_calls: i32 = row.get(7);
            list.push((
                id,
                open,
                failures,
                state,
                last_failure_at,
                half_open_successes,
                last_probe_at,
                active_calls,
            ));
        }
        Ok(list)
    }

    pub async fn insert_anti_fraud_rule(&self, rule: &AntiFraudRule) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO anti_fraud_rules (id, rule_type, target_value, limit_number, enabled) \
             VALUES ($1, $2, $3, $4, $5) \
             ON CONFLICT (id) DO UPDATE \
             SET rule_type = EXCLUDED.rule_type, \
                 target_value = EXCLUDED.target_value, \
                 limit_number = EXCLUDED.limit_number, \
                 enabled = EXCLUDED.enabled",
        )
        .bind(&rule.id)
        .bind(&rule.rule_type)
        .bind(&rule.target_value)
        .bind(rule.limit_number)
        .bind(rule.enabled)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_anti_fraud_rules(&self) -> Result<Vec<AntiFraudRule>, sqlx::Error> {
        sqlx::query_as::<_, AntiFraudRule>(
            "SELECT id, rule_type, target_value, limit_number, enabled FROM anti_fraud_rules ORDER BY created_at DESC"
        )
        .fetch_all(&self.pool)
        .await
    }

    pub async fn delete_anti_fraud_rule(&self, id: &str) -> Result<bool, sqlx::Error> {
        let r = sqlx::query("DELETE FROM anti_fraud_rules WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() > 0)
    }

    pub async fn list_anti_fraud_configs(&self) -> Result<Vec<AntiFraudConfigItem>, sqlx::Error> {
        sqlx::query_as::<_, AntiFraudConfigItem>(
            "SELECT config_key, config_value, description, updated_at FROM anti_fraud_config ORDER BY config_key"
        )
        .fetch_all(&self.pool)
        .await
    }

    pub async fn update_anti_fraud_config(
        &self,
        key: &str,
        value: &str,
    ) -> Result<bool, sqlx::Error> {
        let r = sqlx::query(
            "UPDATE anti_fraud_config SET config_value = $1, updated_at = NOW() WHERE config_key = $2"
        )
        .bind(value)
        .bind(key)
        .execute(&self.pool)
        .await?;
        Ok(r.rows_affected() > 0)
    }

    pub async fn insert_event(&self, event: &CdrEvent) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"
            INSERT INTO call_cdrs (
                call_id, caller, callee, started_at, answered_at, ended_at,
                duration_ms, billable_duration_ms, status, failure_status_code, failure_reason,
                caller_rtcp_loss_rate, caller_rtcp_jitter_ms, caller_rtcp_rtt_ms,
                gateway_rtcp_loss_rate, gateway_rtcp_jitter_ms, gateway_rtcp_rtt_ms,
                mos, dtmf_digits, recording_path, direction
            ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21)
            "#,
        )
        .bind(&event.call_id)
        .bind(&event.caller)
        .bind(&event.callee)
        .bind(utils::offset_from_millis(event.started_at_ms))
        .bind(event.answered_at_ms.map(utils::offset_from_millis))
        .bind(utils::offset_from_millis(event.ended_at_ms))
        .bind(event.duration_ms)
        .bind(event.billable_duration_ms)
        .bind(&event.status)
        .bind(event.failure_status_code.map(|c| c as i32))
        .bind(&event.failure_reason)
        .bind(event.caller_rtcp_loss_rate)
        .bind(event.caller_rtcp_jitter_ms)
        .bind(event.caller_rtcp_rtt_ms.map(|v| v as i32))
        .bind(event.gateway_rtcp_loss_rate)
        .bind(event.gateway_rtcp_jitter_ms)
        .bind(event.gateway_rtcp_rtt_ms.map(|v| v as i32))
        .bind(event.mos)
        .bind(&event.dtmf_digits)
        .bind(&event.recording_path)
        .bind(&event.direction)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn insert_events_batch(&self, events: &[CdrEvent]) -> Result<(), sqlx::Error> {
        if events.is_empty() {
            return Ok(());
        }
        let mut tx = self.pool.begin().await?;
        for event in events {
            sqlx::query(
                r#"
                INSERT INTO call_cdrs (
                    call_id, caller, callee, started_at, answered_at, ended_at,
                    duration_ms, billable_duration_ms, status, failure_status_code, failure_reason,
                    caller_rtcp_loss_rate, caller_rtcp_jitter_ms, caller_rtcp_rtt_ms,
                    gateway_rtcp_loss_rate, gateway_rtcp_jitter_ms, gateway_rtcp_rtt_ms,
                    mos, dtmf_digits, recording_path, direction
                ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21)
                "#,
            )
            .bind(&event.call_id)
            .bind(&event.caller)
            .bind(&event.callee)
            .bind(utils::offset_from_millis(event.started_at_ms))
            .bind(event.answered_at_ms.map(utils::offset_from_millis))
            .bind(utils::offset_from_millis(event.ended_at_ms))
            .bind(event.duration_ms)
            .bind(event.billable_duration_ms)
            .bind(&event.status)
            .bind(event.failure_status_code.map(|c| c as i32))
            .bind(&event.failure_reason)
            .bind(event.caller_rtcp_loss_rate)
            .bind(event.caller_rtcp_jitter_ms)
            .bind(event.caller_rtcp_rtt_ms.map(|v| v as i32))
            .bind(event.gateway_rtcp_loss_rate)
            .bind(event.gateway_rtcp_jitter_ms)
            .bind(event.gateway_rtcp_rtt_ms.map(|v| v as i32))
            .bind(event.mos)
            .bind(&event.dtmf_digits)
            .bind(&event.recording_path)
            .bind(&event.direction)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn load_gateways(
        &self,
    ) -> Result<
        Vec<(
            String,
            String,
            Option<u16>,
            String,
            Option<u32>,
            Option<String>,
            Option<String>,
            Option<String>,
        )>,
        sqlx::Error,
    > {
        let rows = sqlx::query(
            "SELECT id, host, port, transport, max_capacity, caller_id_mode, virtual_caller, prefix_rules FROM sip_gateways",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut gateways = Vec::new();
        for row in rows {
            let id: String = row.get(0);
            let host: String = row.get(1);
            let port: Option<i32> = row.get(2);
            let transport: String = row.get(3);
            let max_capacity: Option<i32> = row.get(4);
            let caller_id_mode: Option<String> = row.get(5);
            let virtual_caller: Option<String> = row.get(6);
            let prefix_rules: Option<String> = row.get(7);
            gateways.push((
                id,
                host,
                port.map(|p| p as u16),
                transport,
                max_capacity.and_then(|c| u32::try_from(c).ok()),
                caller_id_mode,
                virtual_caller,
                prefix_rules,
            ));
        }
        Ok(gateways)
    }

    pub async fn load_gateway_number_info(
        &self,
    ) -> Result<Vec<(String, String, Option<i32>, i32)>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT n.gateway_id, n.direction, n.max_concurrent, n.current_concurrent \
             FROM number_inventory n WHERE n.gateway_id IS NOT NULL",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut result = Vec::new();
        for row in rows {
            let gateway_id: String = row.get(0);
            let direction: String = row.get(1);
            let max_concurrent: Option<i32> = row.get(2);
            let current_concurrent: i32 = row.get(3);
            result.push((gateway_id, direction, max_concurrent, current_concurrent));
        }
        Ok(result)
    }

    pub async fn load_routes(
        &self,
    ) -> Result<
        Vec<(
            String,
            String,
            i32,
            String,
            f64,
            i32,
            Option<String>,
            Option<String>,
        )>,
        sqlx::Error,
    > {
        let rows = sqlx::query(
            "SELECT id, prefix, priority, gateway_id, cost, weight, time_start, time_end FROM sip_routes",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut routes = Vec::new();
        for row in rows {
            let id: String = row.get(0);
            let prefix: String = row.get(1);
            let priority: i32 = row.get(2);
            let gateway_id: String = row.get(3);
            let cost: f64 = row.get(4);
            let weight: i32 = row.get(5);
            let time_start: Option<String> = row.get(6);
            let time_end: Option<String> = row.get(7);
            routes.push((
                id, prefix, priority, gateway_id, cost, weight, time_start, time_end,
            ));
        }
        Ok(routes)
    }

    pub async fn insert_user(&self, username: &str, password: &str) -> Result<(), sqlx::Error> {
        sqlx::query("INSERT INTO sip_users (username, password) VALUES ($1, $2) ON CONFLICT (username) DO UPDATE SET password = EXCLUDED.password")
            .bind(username)
            .bind(password)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn insert_gateway(
        &self,
        id: &str,
        host: &str,
        port: Option<u16>,
        transport: &str,
    ) -> Result<(), sqlx::Error> {
        self.insert_gateway_with_capacity(id, host, port, transport, None)
            .await
    }

    pub async fn insert_gateway_with_capacity(
        &self,
        id: &str,
        host: &str,
        port: Option<u16>,
        transport: &str,
        max_capacity: Option<u32>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query("INSERT INTO sip_gateways (id, host, port, transport, max_capacity) VALUES ($1, $2, $3, $4, $5) ON CONFLICT (id) DO UPDATE SET host = EXCLUDED.host, port = EXCLUDED.port, transport = EXCLUDED.transport, max_capacity = EXCLUDED.max_capacity")
            .bind(id)
            .bind(host)
            .bind(port.map(i32::from))
            .bind(transport)
            .bind(max_capacity.map(i32::try_from).and_then(Result::ok))
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn upsert_gateway_full(&self, gw: &SipGateway) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO sip_gateways (id, host, port, transport, max_capacity, gateway_type, prefix_rules, supports_registration, caller_id_mode, virtual_caller, max_concurrent, account_id, reg_auth_type, reg_username, enabled)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15)
             ON CONFLICT (id) DO UPDATE SET
               host=EXCLUDED.host, port=EXCLUDED.port, transport=EXCLUDED.transport,
               max_capacity=EXCLUDED.max_capacity, gateway_type=EXCLUDED.gateway_type,
               prefix_rules=EXCLUDED.prefix_rules, supports_registration=EXCLUDED.supports_registration,
               caller_id_mode=EXCLUDED.caller_id_mode, virtual_caller=EXCLUDED.virtual_caller,
               max_concurrent=EXCLUDED.max_concurrent, account_id=EXCLUDED.account_id,
               reg_auth_type=EXCLUDED.reg_auth_type, reg_username=EXCLUDED.reg_username,
               enabled=EXCLUDED.enabled",
        )
        .bind(&gw.id)
        .bind(&gw.host)
        .bind(gw.port.map(i32::from))
        .bind(&gw.transport)
        .bind(gw.max_capacity.map(i32::try_from).and_then(Result::ok))
        .bind(&gw.gateway_type)
        .bind(&gw.prefix_rules)
        .bind(gw.supports_registration)
        .bind(&gw.reg_auth_type)
        .bind(&gw.reg_username)
        .bind(gw.max_concurrent)
        .bind(gw.account_id)
        .bind(&gw.reg_password)
        .bind(&gw.parent_gateway_id)
        .bind(gw.enabled)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn insert_route_with_cost(
        &self,
        id: &str,
        prefix: &str,
        priority: i32,
        gateway_id: &str,
        cost: f64,
        weight: i32,
        time_start: Option<&str>,
        time_end: Option<&str>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query("INSERT INTO sip_routes (id, prefix, priority, gateway_id, cost, weight, time_start, time_end) VALUES ($1, $2, $3, $4, $5, $6, $7, $8) ON CONFLICT (id) DO UPDATE SET prefix = EXCLUDED.prefix, priority = EXCLUDED.priority, gateway_id = EXCLUDED.gateway_id, cost = EXCLUDED.cost, weight = EXCLUDED.weight, time_start = EXCLUDED.time_start, time_end = EXCLUDED.time_end")
            .bind(id)
            .bind(prefix)
            .bind(priority)
            .bind(gateway_id)
            .bind(cost)
            .bind(weight)
            .bind(time_start)
            .bind(time_end)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn insert_route(
        &self,
        id: &str,
        prefix: &str,
        priority: i32,
        gateway_id: &str,
    ) -> Result<(), sqlx::Error> {
        self.insert_route_with_cost(id, prefix, priority, gateway_id, 0.0, 100, None, None)
            .await
    }

    pub async fn get_registrations(
        &self,
        aor: &str,
    ) -> Result<Vec<(String, String, OffsetDateTime, Vec<String>)>, sqlx::Error> {
        let rows = sqlx::query("SELECT contact_uri, received_from, expires_at, path FROM sip_registrations WHERE aor = $1 AND expires_at > now()")
            .bind(aor)
            .fetch_all(&self.pool)
            .await?;
        let mut regs = Vec::new();
        for row in rows {
            let contact_uri: String = row.get(0);
            let received_from: String = row.get(1);
            let expires_at: OffsetDateTime = row.get(2);
            let path_str: Option<String> = row.get(3);
            let path = path_str
                .unwrap_or_default()
                .split(',')
                .map(|s| s.to_string())
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>();
            regs.push((contact_uri, received_from, expires_at, path));
        }
        Ok(regs)
    }

    pub async fn get_all_active_received_from(&self) -> Result<Vec<String>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT DISTINCT received_from FROM sip_registrations WHERE expires_at > now()",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut addrs = Vec::new();
        for row in rows {
            let addr: String = row.get(0);
            addrs.push(addr);
        }
        Ok(addrs)
    }

    pub async fn upsert_registration(
        &self,
        aor: &str,
        contact_uri: &str,
        received_from: &str,
        expires_at: OffsetDateTime,
        path: &[String],
    ) -> Result<(), sqlx::Error> {
        let path_str = path.join(",");
        sqlx::query(
            r#"
            INSERT INTO sip_registrations (aor, contact_uri, received_from, expires_at, path, updated_at)
            VALUES ($1, $2, $3, $4, $5, now())
            ON CONFLICT (aor, contact_uri)
            DO UPDATE SET received_from = EXCLUDED.received_from, expires_at = EXCLUDED.expires_at, path = EXCLUDED.path, updated_at = now()
            "#
        )
        .bind(aor)
        .bind(contact_uri)
        .bind(received_from)
        .bind(expires_at)
        .bind(path_str)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn delete_registration(
        &self,
        aor: &str,
        contact_uri: &str,
    ) -> Result<bool, sqlx::Error> {
        let result =
            sqlx::query("DELETE FROM sip_registrations WHERE aor = $1 AND contact_uri = $2")
                .bind(aor)
                .bind(contact_uri)
                .execute(&self.pool)
                .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn delete_all_registrations(&self, aor: &str) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM sip_registrations WHERE aor = $1")
            .bind(aor)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn prune_expired_registrations(&self) -> Result<u64, sqlx::Error> {
        let result = sqlx::query("DELETE FROM sip_registrations WHERE expires_at <= now()")
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
    }

    pub async fn insert_dtmf_event(&self, event: &DtmfEventRecord) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO dtmf_events (call_id, digit, source, timestamp_ms, rtp_timestamp, duration_ms, volume) \
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(&event.call_id)
        .bind(&event.digit)
        .bind(event.source.as_str())
        .bind(event.timestamp_ms)
        .bind(event.rtp_timestamp.map(|v| v as i64))
        .bind(event.duration_ms.map(|v| v as i32))
        .bind(event.volume.map(|v| v as i32))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn insert_dtmf_events_batch(
        &self,
        events: &[DtmfEventRecord],
    ) -> Result<(), sqlx::Error> {
        if events.is_empty() {
            return Ok(());
        }
        let mut tx = self.pool.begin().await?;
        for event in events {
            sqlx::query(
                "INSERT INTO dtmf_events (call_id, digit, source, timestamp_ms, rtp_timestamp, duration_ms, volume) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7)",
            )
            .bind(&event.call_id)
            .bind(&event.digit)
            .bind(event.source.as_str())
            .bind(event.timestamp_ms)
            .bind(event.rtp_timestamp.map(|v| v as i64))
            .bind(event.duration_ms.map(|v| v as i32))
            .bind(event.volume.map(|v| v as i32))
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn list_cdrs(
        &self,
        page: i64,
        page_size: i64,
        status: Option<&str>,
        caller: Option<&str>,
        callee: Option<&str>,
        start: Option<OffsetDateTime>,
        end: Option<OffsetDateTime>,
    ) -> Result<(Vec<CdrEvent>, i64), sqlx::Error> {
        let offset = (page - 1) * page_size;
        let rows = sqlx::query(
            "SELECT call_id, caller, callee, started_at, answered_at, ended_at, \
             duration_ms, billable_duration_ms, status, failure_status_code, failure_reason, \
             caller_rtcp_loss_rate, caller_rtcp_jitter_ms, caller_rtcp_rtt_ms, \
             gateway_rtcp_loss_rate, gateway_rtcp_jitter_ms, gateway_rtcp_rtt_ms, \
             mos, dtmf_digits, recording_path, direction \
             FROM call_cdrs \
             WHERE ($1::text IS NULL OR status = $1) \
               AND ($2::text IS NULL OR caller LIKE '%' || $2 || '%') \
               AND ($3::text IS NULL OR callee LIKE '%' || $3 || '%') \
               AND ($4::timestamptz IS NULL OR started_at >= $4) \
               AND ($5::timestamptz IS NULL OR started_at <= $5) \
             ORDER BY started_at DESC \
             LIMIT $6 OFFSET $7",
        )
        .bind(status)
        .bind(caller)
        .bind(callee)
        .bind(start)
        .bind(end)
        .bind(page_size)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;

        let count_row = sqlx::query_scalar(
            "SELECT COUNT(*) FROM call_cdrs \
             WHERE ($1::text IS NULL OR status = $1) \
               AND ($2::text IS NULL OR caller LIKE '%' || $2 || '%') \
               AND ($3::text IS NULL OR callee LIKE '%' || $3 || '%') \
               AND ($4::timestamptz IS NULL OR started_at >= $4) \
               AND ($5::timestamptz IS NULL OR started_at <= $5)",
        )
        .bind(status)
        .bind(caller)
        .bind(callee)
        .bind(start)
        .bind(end)
        .fetch_one(&self.pool)
        .await?;

        let total: i64 = count_row;
        let items: Vec<CdrEvent> = rows.iter().map(utils::cdr_event_from_row).collect();
        Ok((items, total))
    }

    pub async fn get_cdr(&self, call_id: &str) -> Result<Option<CdrEvent>, sqlx::Error> {
        let row = sqlx::query(
            "SELECT call_id, caller, callee, started_at, answered_at, ended_at, \
             duration_ms, billable_duration_ms, status, failure_status_code, failure_reason, \
             caller_rtcp_loss_rate, caller_rtcp_jitter_ms, caller_rtcp_rtt_ms, \
             gateway_rtcp_loss_rate, gateway_rtcp_jitter_ms, gateway_rtcp_rtt_ms, \
             mos, dtmf_digits, recording_path, direction \
             FROM call_cdrs WHERE call_id = $1",
        )
        .bind(call_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|r| utils::cdr_event_from_row(&r)))
    }

    pub async fn get_dashboard_stats(
        &self,
        active_calls: i64,
    ) -> Result<DashboardStats, sqlx::Error> {
        let today_start = time::OffsetDateTime::now_utc()
            .replace_time(time::Time::from_hms(0, 0, 0).unwrap_or(time::Time::MIDNIGHT));
        let row = sqlx::query(
            "SELECT \
               COUNT(*) as total, \
               COUNT(*) FILTER (WHERE status = 'answered') as answered, \
               COUNT(*) FILTER (WHERE status = 'canceled') as canceled, \
               COUNT(*) FILTER (WHERE status = 'failed') as failed, \
               AVG(mos) as avg_mos, \
               AVG(caller_rtcp_loss_rate) as avg_loss, \
               AVG(caller_rtcp_jitter_ms) as avg_jitter \
             FROM call_cdrs WHERE started_at >= $1",
        )
        .bind(today_start)
        .fetch_one(&self.pool)
        .await?;

        let total: i64 = row.get(0);
        let answered: i64 = row.get(1);
        let canceled: i64 = row.get(2);
        let failed: i64 = row.get(3);
        let avg_mos: Option<f64> = row.get(4);
        let avg_loss: Option<f64> = row.get(5);
        let avg_jitter: Option<f64> = row.get(6);

        let reg_row: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM sip_registrations WHERE expires_at > now()")
                .fetch_one(&self.pool)
                .await?;

        let gw_row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM sip_gateways")
            .fetch_one(&self.pool)
            .await?;

        let answer_rate = if total > 0 {
            answered as f64 / total as f64
        } else {
            0.0
        };

        Ok(DashboardStats {
            active_calls,
            today_total_calls: total,
            today_answered_calls: answered,
            today_canceled_calls: canceled,
            today_failed_calls: failed,
            answer_rate,
            avg_mos,
            avg_loss_rate: avg_loss,
            avg_jitter_ms: avg_jitter,
            registered_users: reg_row.0,
            active_gateways: gw_row.0,
        })
    }

    pub async fn get_hourly_trend(&self) -> Result<Vec<HourlyTrend>, sqlx::Error> {
        let today_start = time::OffsetDateTime::now_utc()
            .replace_time(time::Time::from_hms(0, 0, 0).unwrap_or(time::Time::MIDNIGHT));
        let rows = sqlx::query(
            "SELECT EXTRACT(HOUR FROM started_at)::INTEGER as hour, \
                    COUNT(*) as total, \
                    COUNT(*) FILTER (WHERE status = 'answered') as answered \
             FROM call_cdrs WHERE started_at >= $1 \
             GROUP BY hour ORDER BY hour",
        )
        .bind(today_start)
        .fetch_all(&self.pool)
        .await?;

        let trends: Vec<HourlyTrend> = rows
            .iter()
            .map(|row| HourlyTrend {
                hour: row.get(0),
                total: row.get(1),
                answered: row.get(2),
            })
            .collect();
        Ok(trends)
    }

    pub async fn list_users(&self) -> Result<Vec<SipUser>, sqlx::Error> {
        let rows = sqlx::query("SELECT username, created_at FROM sip_users ORDER BY username")
            .fetch_all(&self.pool)
            .await?;
        let mut users = Vec::with_capacity(rows.len());
        for row in rows {
            users.push(SipUser {
                username: row.get(0),
                password: None,
                created_at: row.get(1),
            });
        }
        Ok(users)
    }

    pub async fn delete_user(&self, username: &str) -> Result<bool, sqlx::Error> {
        let result = sqlx::query("DELETE FROM sip_users WHERE username = $1")
            .bind(username)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn get_user_password(&self, username: &str) -> Result<Option<String>, sqlx::Error> {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT password FROM sip_users WHERE username = $1")
                .bind(username)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.map(|(pw,)| pw))
    }

    pub async fn list_gateways_full(&self) -> Result<Vec<SipGateway>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT g.id, g.host, g.port, g.transport, g.max_capacity, g.gateway_type, g.prefix_rules, \
             g.supports_registration, g.caller_id_mode, g.virtual_caller, g.max_concurrent, g.account_id, \
             g.enabled, g.created_at, h.active_calls, h.state \
             FROM sip_gateways g \
             LEFT JOIN gateway_health_status h ON g.id = h.gateway_id \
             ORDER BY g.id",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut gateways = Vec::with_capacity(rows.len());
        for row in rows {
            let active_calls: Option<i32> = row.get(14);
            let state: Option<String> = row.get(15);
            gateways.push(SipGateway {
                id: row.get(0),
                host: row.get(1),
                port: row.get::<Option<i32>, _>(2).map(|p| p as u16),
                transport: row.get(3),
                max_capacity: row
                    .get::<Option<i32>, _>(4)
                    .and_then(|c| u32::try_from(c).ok()),
                gateway_type: row.get(5),
                prefix_rules: row.get(6),
                supports_registration: row.get(7),
                reg_auth_type: None,
                reg_username: None,
                reg_password: None,
                parent_gateway_id: None,
                caller_id_mode: row.get(8),
                virtual_caller: row.get(9),
                current_concurrent: Some(active_calls.unwrap_or(0)),
                circuit_state: Some(state.unwrap_or_else(|| "closed".to_string())),
                account_id: row.get(10),
                max_concurrent: row.get(11),
                enabled: row.get(12),
                created_at: row.get(13),
            });
        }
        Ok(gateways)
    }

    pub async fn delete_gateway(&self, id: &str) -> Result<bool, sqlx::Error> {
        let result = sqlx::query("DELETE FROM sip_gateways WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn list_routes_full(&self) -> Result<Vec<SipRoute>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT id, prefix, priority, gateway_id, cost, weight, time_start, time_end, created_at FROM sip_routes ORDER BY priority, id"
        )
        .fetch_all(&self.pool)
        .await?;
        let mut routes = Vec::with_capacity(rows.len());
        for row in rows {
            routes.push(SipRoute {
                id: row.get(0),
                prefix: row.get(1),
                priority: row.get(2),
                gateway_id: row.get(3),
                cost: row.get(4),
                weight: row.get(5),
                time_start: row.get(6),
                time_end: row.get(7),
                created_at: row.get(8),
            });
        }
        Ok(routes)
    }

    pub async fn delete_route(&self, id: &str) -> Result<bool, sqlx::Error> {
        let result = sqlx::query("DELETE FROM sip_routes WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn list_registrations(&self) -> Result<Vec<SipRegistration>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT aor, contact_uri, received_from, expires_at, path, updated_at \
             FROM sip_registrations WHERE expires_at > now() ORDER BY aor",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut registrations = Vec::with_capacity(rows.len());
        for row in rows {
            let path_str: Option<String> = row.get(4);
            let path = path_str
                .unwrap_or_default()
                .split(',')
                .map(|s| s.to_string())
                .filter(|s| !s.is_empty())
                .collect();
            registrations.push(SipRegistration {
                aor: row.get(0),
                contact_uri: row.get(1),
                received_from: row.get(2),
                expires_at: row.get(3),
                path,
                updated_at: row.get(5),
            });
        }
        Ok(registrations)
    }

    pub async fn get_dtmf_events(
        &self,
        call_id: &str,
    ) -> Result<Vec<DtmfEventRecord>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT call_id, digit, source, timestamp_ms, rtp_timestamp, duration_ms, volume \
             FROM dtmf_events WHERE call_id = $1 ORDER BY timestamp_ms",
        )
        .bind(call_id)
        .fetch_all(&self.pool)
        .await?;
        let mut events = Vec::with_capacity(rows.len());
        for row in rows {
            let source_str: String = row.get(2);
            let source = match source_str.as_str() {
                "rtp" => DtmfSource::Rtp,
                "sip-info" => DtmfSource::SipInfo,
                _ => DtmfSource::SipInfo,
            };
            events.push(DtmfEventRecord {
                call_id: row.get(0),
                digit: row.get(1),
                source,
                timestamp_ms: row.get(3),
                rtp_timestamp: row.get::<Option<i64>, _>(4).map(|v| v as u32),
                duration_ms: row.get::<Option<i32>, _>(5).map(|v| v as u16),
                volume: row.get::<Option<i32>, _>(6).map(|v| v as u8),
            });
        }
        Ok(events)
    }

    // ===== 计费：费率 =====

    pub async fn list_rates(&self) -> Result<Vec<BillingRate>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT id, prefix, rate_per_minute, description, created_at FROM billing_rates \
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
                description: row.get(3),
                created_at: row.get(4),
            });
        }
        Ok(rates)
    }

    pub async fn upsert_rate(
        &self,
        id: &str,
        prefix: &str,
        rate_per_minute: f64,
        description: Option<&str>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO billing_rates (id, prefix, rate_per_minute, description) VALUES ($1,$2,$3,$4) \
             ON CONFLICT (id) DO UPDATE SET prefix=EXCLUDED.prefix, rate_per_minute=EXCLUDED.rate_per_minute, description=EXCLUDED.description",
        )
        .bind(id)
        .bind(prefix)
        .bind(rate_per_minute)
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

        let rate: f64 = sqlx::query_scalar(
            "SELECT COALESCE(rate_per_minute, 0.0)::DOUBLE PRECISION FROM billing_rates \
             WHERE $1 LIKE prefix || '%' ORDER BY length(prefix) DESC LIMIT 1",
        )
        .bind(callee)
        .fetch_optional(&self.pool)
        .await?
        .unwrap_or(0.0);

        if rate <= 0.0 {
            return Ok(None);
        }

        let amount = rate * duration_ms as f64 / 60000.0;
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
            "INSERT INTO billing_ledger (call_id, username, duration_ms, rate_per_minute, amount, balance_after) \
             VALUES ($1,$2,$3,$4,$5,$6)",
        )
        .bind(call_id)
        .bind(username)
        .bind(duration_ms)
        .bind(rate)
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
                "SELECT id, call_id, username, duration_ms, rate_per_minute, amount, balance_after, created_at \
                 FROM billing_ledger WHERE username=$1 ORDER BY created_at DESC LIMIT 500",
            )
            .bind(u)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query(
                "SELECT id, call_id, username, duration_ms, rate_per_minute, amount, balance_after, created_at \
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
                amount: row.get(5),
                balance_after: row.get(6),
                created_at: row.get(7),
            });
        }
        Ok(out)
    }

    // ===== 计费：离线对账 =====
    pub async fn reconcile_billing(
        &self,
        start: OffsetDateTime,
        end: OffsetDateTime,
    ) -> Result<ReconcileResult, sqlx::Error> {
        let rate_rows = sqlx::query(
            "SELECT prefix, rate_per_minute FROM billing_rates ORDER BY length(prefix) DESC",
        )
        .fetch_all(&self.pool)
        .await?;
        let rates: Vec<(String, f64)> = rate_rows
            .into_iter()
            .map(|r| (r.get::<String, _>(0), r.get::<f64, _>(1)))
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
            let rate = utils::match_rate(&callee_str, &rates);
            let amount = rate * billable_ms as f64 / 60000.0;

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
                "INSERT INTO billing_ledger (call_id, username, duration_ms, rate_per_minute, amount, balance_after) \
                 VALUES ($1,$2,$3,$4,$5,$6)",
            )
            .bind(&call_id)
            .bind(user)
            .bind(billable_ms)
            .bind(rate)
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

    // ===== 号码库存 =====
    pub async fn list_numbers(&self) -> Result<Vec<NumberInventory>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT number, username, gateway_id, direction, max_concurrent, current_concurrent, status, created_at, updated_at FROM number_inventory ORDER BY number",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut numbers = Vec::with_capacity(rows.len());
        for row in rows {
            numbers.push(NumberInventory {
                number: row.get(0),
                username: row.get(1),
                gateway_id: row.get(2),
                direction: row.get(3),
                max_concurrent: row.get(4),
                current_concurrent: row.get(5),
                status: row.get(6),
                created_at: row.get(7),
                updated_at: row.get(8),
            });
        }
        Ok(numbers)
    }

    pub async fn upsert_number(
        &self,
        number: &str,
        username: Option<&str>,
        gateway_id: Option<&str>,
        direction: Option<&str>,
        max_concurrent: Option<i32>,
        status: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO number_inventory (number, username, gateway_id, direction, max_concurrent, status, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, now()) \
             ON CONFLICT (number) DO UPDATE SET username=EXCLUDED.username, gateway_id=EXCLUDED.gateway_id, \
             direction=EXCLUDED.direction, max_concurrent=EXCLUDED.max_concurrent, status=EXCLUDED.status, updated_at=now()",
        )
        .bind(number)
        .bind(username)
        .bind(gateway_id)
        .bind(direction)
        .bind(max_concurrent)
        .bind(status)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn delete_number(&self, number: &str) -> Result<bool, sqlx::Error> {
        let r = sqlx::query("DELETE FROM number_inventory WHERE number=$1")
            .bind(number)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() > 0)
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
