//! # 数据库表结构定义
//!
//! 本模块定义了所有数据库表的 CREATE TABLE 语句。
//! 使用 `sqlx` 编译期 SQL 检查，确保 SQL 语法正确。
//!
//! ## 表结构概览
//!
//! | 表名 | 用途 | 关键字段 |
//! |------|------|----------|
//! | `call_cdrs` | 通话详单 | call_id, status, duration_ms, mos |
//! | `sip_gateways` | 网关配置 | id, host, port, max_capacity |
//! | `sip_routes` | 路由规则 | prefix, priority, gateway_id, cost, weight |
//! | `sip_users` | SIP 用户 | username, password |
//! | `sip_registrations` | 注册绑定 | aor, contact_uri, expires_at |
//! | `billing_rates` | 费率表 | prefix, rate_per_minute |
//! | `billing_accounts` | 计费账户 | username, balance, credit_limit |
//! | `billing_ledger` | 扣费流水 | call_id, amount, balance_after |
//! | `gateway_health_status` | 网关健康 | gateway_id, state, last_failure_at |
//! | `anti_fraud_rules` | 反欺诈规则 | rule_type, target_value, limit_number |
//! | `dtmf_events` | DTMF 事件 | call_id, digit, source, timestamp_ms |
//! | `number_inventory` | 号码库存 | number, status, direction |

pub(super) const CREATE_CDR_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS call_cdrs (
    id BIGSERIAL PRIMARY KEY,
    call_id TEXT NOT NULL,
    caller TEXT,
    callee TEXT,
    started_at TIMESTAMPTZ NOT NULL,
    answered_at TIMESTAMPTZ,
    ended_at TIMESTAMPTZ NOT NULL,
    duration_ms BIGINT NOT NULL,
    billable_duration_ms BIGINT NOT NULL,
    status TEXT NOT NULL,
    failure_status_code INTEGER,
    failure_reason TEXT,
    caller_rtcp_loss_rate DOUBLE PRECISION,
    caller_rtcp_jitter_ms DOUBLE PRECISION,
    caller_rtcp_rtt_ms INTEGER,
    gateway_rtcp_loss_rate DOUBLE PRECISION,
    gateway_rtcp_jitter_ms DOUBLE PRECISION,
    gateway_rtcp_rtt_ms INTEGER,
    mos DOUBLE PRECISION,
    dtmf_digits TEXT,
    recording_path TEXT,
    direction VARCHAR(10) DEFAULT 'outbound',
    inserted_at TIMESTAMPTZ NOT NULL DEFAULT now()
)
"#;

pub(super) const CREATE_CALL_ID_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_call_cdrs_call_id ON call_cdrs (call_id)";
pub(super) const CREATE_STARTED_AT_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_call_cdrs_started_at ON call_cdrs (started_at)";
pub(super) const CREATE_STATUS_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_call_cdrs_status ON call_cdrs (status)";

pub(super) const CREATE_SIP_USERS_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS sip_users (
    username TEXT PRIMARY KEY,
    password TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
)
"#;

pub(super) const CREATE_SIP_GATEWAYS_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS sip_gateways (
    id TEXT PRIMARY KEY,
    host TEXT NOT NULL,
    port INTEGER,
    transport TEXT NOT NULL DEFAULT 'udp',
    max_capacity INTEGER,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
)
"#;

pub(super) const CREATE_SIP_ROUTES_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS sip_routes (
    id TEXT PRIMARY KEY,
    prefix TEXT NOT NULL,
    priority INTEGER NOT NULL DEFAULT 100,
    gateway_id TEXT NOT NULL REFERENCES sip_gateways(id) ON DELETE CASCADE,
    cost DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    weight INTEGER NOT NULL DEFAULT 100,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
)
"#;

pub(super) const MIGRATION_ADD_ROUTE_WEIGHT: &str =
    "ALTER TABLE sip_routes ADD COLUMN IF NOT EXISTS weight INTEGER NOT NULL DEFAULT 100";

pub(super) const CREATE_DTMF_EVENTS_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS dtmf_events (
    id BIGSERIAL PRIMARY KEY,
    call_id TEXT NOT NULL,
    digit TEXT NOT NULL,
    source TEXT NOT NULL,
    timestamp_ms BIGINT NOT NULL,
    rtp_timestamp BIGINT,
    duration_ms INTEGER,
    volume INTEGER,
    inserted_at TIMESTAMPTZ NOT NULL DEFAULT now()
)
"#;

pub(super) const CREATE_DTMF_CALL_ID_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_dtmf_events_call_id ON dtmf_events (call_id)";

pub(super) const CREATE_SIP_REGISTRATIONS_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS sip_registrations (
    aor TEXT NOT NULL,
    contact_uri TEXT NOT NULL,
    received_from TEXT NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    path TEXT,
    PRIMARY KEY (aor, contact_uri)
)
"#;

pub(super) const CREATE_GATEWAY_HEALTH_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS gateway_health_status (
    gateway_id TEXT PRIMARY KEY,
    circuit_open BOOLEAN NOT NULL DEFAULT FALSE,
    consecutive_failures INTEGER NOT NULL DEFAULT 0,
    state TEXT NOT NULL DEFAULT 'closed',
    last_failure_at TIMESTAMPTZ,
    half_open_successes INTEGER NOT NULL DEFAULT 0,
    last_probe_at TIMESTAMPTZ,
    active_calls INTEGER NOT NULL DEFAULT 0,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
)
"#;

pub(super) const CREATE_ANTI_FRAUD_RULES_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS anti_fraud_rules (
    id TEXT PRIMARY KEY,
    rule_type TEXT NOT NULL,
    target_value TEXT NOT NULL,
    limit_number INTEGER,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
)
"#;

/// 防盗打全局配置表。
pub(super) const CREATE_ANTI_FRAUD_CONFIG_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS anti_fraud_config (
    config_key TEXT PRIMARY KEY,
    config_value TEXT NOT NULL,
    description TEXT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
)
"#;

/// 将早期版本的反盗打规则表迁移到当前统一模型。
///
/// 早期脚本使用 `BIGSERIAL id` 和 `value` 字段，新模型使用字符串 ID、
/// `target_value` 和可选的并发限制。迁移通过 information_schema 判断字段，
/// 因此可以安全地在新旧数据库上重复执行。
pub(super) const MIGRATE_LEGACY_ANTI_FRAUD_RULES_SQL: &str = r#"
DO $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_name = 'anti_fraud_rules' AND column_name = 'value'
    ) AND NOT EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_name = 'anti_fraud_rules' AND column_name = 'target_value'
    ) THEN
        ALTER TABLE anti_fraud_rules ADD COLUMN target_value TEXT;
        UPDATE anti_fraud_rules SET target_value = value WHERE target_value IS NULL;
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_name = 'anti_fraud_rules' AND column_name = 'target_value'
    ) THEN
        ALTER TABLE anti_fraud_rules ADD COLUMN target_value TEXT;
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_name = 'anti_fraud_rules' AND column_name = 'limit_number'
    ) THEN
        ALTER TABLE anti_fraud_rules ADD COLUMN limit_number INTEGER;
    END IF;
END $$;
"#;

pub(super) const MIGRATE_LEGACY_ANTI_FRAUD_RULES_STEP2_SQL: &str = r#"
ALTER TABLE anti_fraud_rules
    ALTER COLUMN id TYPE TEXT USING id::TEXT;
"#;

pub(super) const MIGRATE_LEGACY_ANTI_FRAUD_RULES_STEP3_SQL: &str = r#"
UPDATE anti_fraud_rules
SET target_value = COALESCE(target_value, '')
WHERE target_value IS NULL;
"#;

pub(super) const MIGRATE_LEGACY_ANTI_FRAUD_RULES_STEP4_SQL: &str = r#"
ALTER TABLE anti_fraud_rules
    ALTER COLUMN target_value SET NOT NULL;
"#;

/// 防盗打默认配置，使用幂等插入保证不会覆盖运营方已有设置。
pub(super) const SEED_ANTI_FRAUD_CONFIG_SQL: &str = r#"
INSERT INTO anti_fraud_config (config_key, config_value, description) VALUES
    ('enabled', 'true', '启用防盗打'),
    ('max_concurrent_per_account', '50', '每账户最大并发呼叫数'),
    ('max_concurrent_per_ip', '20', '每 IP 最大并发呼叫数'),
    ('max_cps_per_account', '10', '每账户每秒最大呼叫数'),
    ('min_call_duration', '3', '最短通话时长（秒）'),
    ('max_call_duration', '3600', '最长通话时长（秒）'),
    ('short_call_threshold', '5', '短通话检测阈值'),
    ('short_call_window', '60', '短通话检测窗口（秒）'),
    ('block_international', 'true', '拦截国际呼叫'),
    ('block_premium', 'true', '拦截高额号码'),
    ('allow_zero_balance', 'false', '允许零余额呼叫')
ON CONFLICT (config_key) DO NOTHING
"#;

pub(super) const CREATE_BILLING_RATES_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS billing_rates (
    id TEXT PRIMARY KEY,
    prefix TEXT NOT NULL,
    rate_per_minute NUMERIC(20, 8) NOT NULL,
    description TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
)
"#;

pub(super) const CREATE_BILLING_ACCOUNTS_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS billing_accounts (
    username TEXT PRIMARY KEY,
    balance NUMERIC(20, 8) NOT NULL DEFAULT 0.0,
    credit_limit NUMERIC(20, 8) NOT NULL DEFAULT 0.0,
    currency TEXT NOT NULL DEFAULT 'CNY',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
)
"#;

pub(super) const CREATE_BILLING_LEDGER_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS billing_ledger (
    id BIGSERIAL PRIMARY KEY,
    call_id TEXT NOT NULL UNIQUE,
    username TEXT NOT NULL,
    duration_ms BIGINT NOT NULL,
    rate_per_minute NUMERIC(20, 8) NOT NULL,
    amount NUMERIC(20, 8) NOT NULL,
    balance_after NUMERIC(20, 8) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
)
"#;

pub(super) const CREATE_LEDGER_USERNAME_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_billing_ledger_username ON billing_ledger (username)";

pub(super) const CREATE_NUMBER_INVENTORY_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS number_inventory (
    number TEXT PRIMARY KEY,
    username TEXT,
    status TEXT NOT NULL DEFAULT 'available',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
)
"#;

/// 管理 API 审计日志表。
pub(super) const CREATE_AUDIT_LOGS_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS api_audit_logs (
    id BIGSERIAL PRIMARY KEY,
    request_id TEXT NOT NULL,
    username TEXT NOT NULL,
    role TEXT NOT NULL,
    method TEXT NOT NULL,
    path TEXT NOT NULL,
    status_code INTEGER NOT NULL,
    source_ip INET,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
)
"#;

pub(super) const CREATE_AUDIT_LOGS_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_api_audit_logs_created_at ON api_audit_logs (created_at DESC)";
