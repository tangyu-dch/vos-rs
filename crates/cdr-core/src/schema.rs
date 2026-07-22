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
    audit JSONB NOT NULL DEFAULT '{}'::jsonb,
    inserted_at TIMESTAMPTZ NOT NULL DEFAULT now()
)
"#;

pub(super) const MIGRATE_CDR_AUDIT_SQL: &str =
    "ALTER TABLE call_cdrs ADD COLUMN IF NOT EXISTS audit JSONB NOT NULL DEFAULT '{}'::jsonb";

pub(super) const CREATE_CALL_ID_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_call_cdrs_call_id ON call_cdrs (call_id)";
/// 一次性修复历史重复 CDR，并创建幂等唯一索引。
///
/// 通过检查索引是否存在避免每次服务启动都扫描整张 CDR 表。
pub(super) const MIGRATE_CDR_IDEMPOTENCY_SQL: &str = r#"
DO $$
BEGIN
    IF to_regclass('public.idx_call_cdrs_call_id_unique') IS NULL THEN
        DELETE FROM call_cdrs older
        USING call_cdrs newer
        WHERE older.call_id = newer.call_id
          AND older.id < newer.id;
        CREATE UNIQUE INDEX idx_call_cdrs_call_id_unique ON call_cdrs (call_id);
    END IF;
END $$;
"#;
pub(super) const CREATE_STARTED_AT_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_call_cdrs_started_at ON call_cdrs (started_at)";
pub(super) const CREATE_STATUS_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_call_cdrs_status ON call_cdrs (status)";
pub(super) const CREATE_CDR_CALLER_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_call_cdrs_caller ON call_cdrs (caller)";
pub(super) const CREATE_CDR_CALLEE_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_call_cdrs_callee ON call_cdrs (callee)";

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
    gateway_type VARCHAR(20) NOT NULL DEFAULT 'peer',
    prefix_rules TEXT NOT NULL DEFAULT '',
    supports_registration BOOLEAN NOT NULL DEFAULT FALSE,
    reg_auth_type VARCHAR(20) NOT NULL DEFAULT 'none',
    reg_username TEXT NOT NULL DEFAULT '',
    reg_password TEXT NOT NULL DEFAULT '',
    parent_gateway_id TEXT,
    caller_id_mode VARCHAR(20) NOT NULL DEFAULT 'passthrough',
    virtual_caller TEXT NOT NULL DEFAULT '',
    current_concurrent INTEGER NOT NULL DEFAULT 0,
    max_concurrent INTEGER NOT NULL DEFAULT 100,
    account_id BIGINT,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
)
"#;

/// 将历史网关表增量升级到管理面和 SIP 路由共同使用的规范结构。
pub(super) const MIGRATE_SIP_GATEWAYS_SQL: &[&str] = &[
    "ALTER TABLE sip_gateways ADD COLUMN IF NOT EXISTS gateway_type VARCHAR(20) NOT NULL DEFAULT 'peer'",
    "ALTER TABLE sip_gateways ADD COLUMN IF NOT EXISTS prefix_rules TEXT NOT NULL DEFAULT ''",
    "ALTER TABLE sip_gateways ADD COLUMN IF NOT EXISTS supports_registration BOOLEAN NOT NULL DEFAULT FALSE",
    "ALTER TABLE sip_gateways ADD COLUMN IF NOT EXISTS reg_auth_type VARCHAR(20) NOT NULL DEFAULT 'none'",
    "ALTER TABLE sip_gateways ADD COLUMN IF NOT EXISTS reg_username TEXT NOT NULL DEFAULT ''",
    "ALTER TABLE sip_gateways ADD COLUMN IF NOT EXISTS reg_password TEXT NOT NULL DEFAULT ''",
    // 旧版曾将上游注册密码写入该字段。主动注册尚未启用，迁移时清除明文凭据。
    "UPDATE sip_gateways SET reg_password = '' WHERE reg_password <> ''",
    "ALTER TABLE sip_gateways ADD COLUMN IF NOT EXISTS parent_gateway_id TEXT",
    "ALTER TABLE sip_gateways ADD COLUMN IF NOT EXISTS caller_id_mode VARCHAR(20) NOT NULL DEFAULT 'passthrough'",
    "ALTER TABLE sip_gateways ADD COLUMN IF NOT EXISTS virtual_caller TEXT NOT NULL DEFAULT ''",
    "ALTER TABLE sip_gateways ADD COLUMN IF NOT EXISTS current_concurrent INTEGER NOT NULL DEFAULT 0",
    "ALTER TABLE sip_gateways ADD COLUMN IF NOT EXISTS max_concurrent INTEGER NOT NULL DEFAULT 100",
    "ALTER TABLE sip_gateways ADD COLUMN IF NOT EXISTS account_id BIGINT",
    r#"DO $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_schema = 'public'
          AND table_name = 'sip_gateways'
          AND column_name = 'account_id'
          AND data_type = 'integer'
    ) THEN
        ALTER TABLE sip_gateways DROP CONSTRAINT IF EXISTS fk_gateway_account;
        ALTER TABLE sip_gateways ALTER COLUMN account_id TYPE BIGINT USING account_id::BIGINT;
    END IF;
END $$;
"#,
    "ALTER TABLE sip_gateways ADD COLUMN IF NOT EXISTS enabled BOOLEAN NOT NULL DEFAULT TRUE",
];

pub(super) const CREATE_GATEWAYS_TYPE_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_sip_gateways_type ON sip_gateways (gateway_type)";
pub(super) const CREATE_GATEWAYS_PARENT_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_sip_gateways_parent ON sip_gateways (parent_gateway_id)";
pub(super) const CREATE_GATEWAYS_ACCOUNT_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_sip_gateways_account ON sip_gateways (account_id)";
pub(super) const CREATE_GATEWAYS_ENABLED_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_sip_gateways_enabled ON sip_gateways (enabled)";

pub(super) const CREATE_SIP_ROUTES_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS sip_routes (
    id TEXT PRIMARY KEY,
    prefix TEXT NOT NULL,
    priority INTEGER NOT NULL DEFAULT 100,
    gateway_id TEXT NOT NULL REFERENCES sip_gateways(id) ON DELETE CASCADE,
    cost DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    weight INTEGER NOT NULL DEFAULT 100,
    topology JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
)
"#;

pub(super) const MIGRATION_ADD_ROUTE_WEIGHT: &str =
    "ALTER TABLE sip_routes ADD COLUMN IF NOT EXISTS weight INTEGER NOT NULL DEFAULT 100";

/// 旧库升级: 为已存在的 sip_routes 补充 topology JSONB 字段, 用于持久化可视化拓扑编排
pub(super) const MIGRATION_ADD_ROUTE_TOPOLOGY: &str =
    "ALTER TABLE sip_routes ADD COLUMN IF NOT EXISTS topology JSONB NOT NULL DEFAULT '{}'::jsonb";

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

pub(super) const CREATE_REGISTRATIONS_EXPIRES_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_sip_registrations_expires_at ON sip_registrations (expires_at)";

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

pub(super) const CREATE_GATEWAY_HEALTH_STATE_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_gateway_health_state ON gateway_health_status (state)";

pub(super) const CREATE_ROUTES_PRIORITY_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_sip_routes_priority_id ON sip_routes (priority, id)";
pub(super) const CREATE_ROUTES_PREFIX_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_sip_routes_prefix ON sip_routes (prefix)";
pub(super) const CREATE_ROUTES_GATEWAY_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_sip_routes_gateway ON sip_routes (gateway_id)";

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

    IF EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_name = 'anti_fraud_rules' AND column_name = 'value'
    ) THEN
        -- New writes use target_value; retain the legacy column for compatibility
        -- without forcing callers to populate both representations.
        ALTER TABLE anti_fraud_rules ALTER COLUMN value DROP NOT NULL;
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
    billing_interval_secs INTEGER NOT NULL DEFAULT 60 CHECK (billing_interval_secs > 0),
    price_per_interval NUMERIC(20, 8) NOT NULL DEFAULT 0,
    description TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
)
"#;

pub(super) const CREATE_BILLING_ACCOUNTS_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS billing_accounts (
    id BIGSERIAL UNIQUE,
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
    billing_interval_secs INTEGER NOT NULL DEFAULT 60,
    price_per_interval NUMERIC(20, 8) NOT NULL DEFAULT 0,
    amount NUMERIC(20, 8) NOT NULL,
    balance_after NUMERIC(20, 8) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
)
"#;

pub(super) const CREATE_BILLING_CREDITS_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS billing_credits (
    idempotency_key TEXT PRIMARY KEY,
    username TEXT NOT NULL,
    amount NUMERIC(20, 8) NOT NULL CHECK (amount > 0),
    balance_after NUMERIC(20, 8) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
)
"#;

pub(super) const CREATE_BILLING_CREDITS_USERNAME_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_billing_credits_username ON billing_credits (username, created_at DESC)";

pub(super) const CREATE_LEDGER_USERNAME_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_billing_ledger_username ON billing_ledger (username)";
pub(super) const CREATE_LEDGER_CREATED_AT_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_billing_ledger_created_at ON billing_ledger (created_at DESC)";

pub(super) const MIGRATE_BILLING_INTERVALS_SQL: &str = r#"
ALTER TABLE billing_rates ADD COLUMN IF NOT EXISTS billing_interval_secs INTEGER;
ALTER TABLE billing_rates ADD COLUMN IF NOT EXISTS price_per_interval NUMERIC(20, 8);
UPDATE billing_rates SET billing_interval_secs = 60 WHERE billing_interval_secs IS NULL;
UPDATE billing_rates SET price_per_interval = rate_per_minute WHERE price_per_interval IS NULL;
ALTER TABLE billing_rates ALTER COLUMN billing_interval_secs SET DEFAULT 60;
ALTER TABLE billing_rates ALTER COLUMN billing_interval_secs SET NOT NULL;
ALTER TABLE billing_rates ALTER COLUMN price_per_interval SET DEFAULT 0;
ALTER TABLE billing_rates ALTER COLUMN price_per_interval SET NOT NULL;
ALTER TABLE billing_ledger ADD COLUMN IF NOT EXISTS billing_interval_secs INTEGER;
ALTER TABLE billing_ledger ADD COLUMN IF NOT EXISTS price_per_interval NUMERIC(20, 8);
UPDATE billing_ledger SET billing_interval_secs = 60 WHERE billing_interval_secs IS NULL;
UPDATE billing_ledger SET price_per_interval = rate_per_minute WHERE price_per_interval IS NULL;
ALTER TABLE billing_ledger ALTER COLUMN billing_interval_secs SET DEFAULT 60;
ALTER TABLE billing_ledger ALTER COLUMN billing_interval_secs SET NOT NULL;
ALTER TABLE billing_ledger ALTER COLUMN price_per_interval SET DEFAULT 0;
ALTER TABLE billing_ledger ALTER COLUMN price_per_interval SET NOT NULL;
"#;

pub(super) const CREATE_NUMBER_INVENTORY_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS number_inventory (
    number TEXT PRIMARY KEY,
    username TEXT,
    gateway_id TEXT,
    direction VARCHAR(20) NOT NULL DEFAULT 'bidirectional',
    max_concurrent INTEGER NOT NULL DEFAULT 10,
    current_concurrent INTEGER NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'available',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
)
"#;

pub(super) const MIGRATE_BILLING_ACCOUNTS_SQL: &[&str] = &[
    "ALTER TABLE billing_accounts ADD COLUMN IF NOT EXISTS id BIGSERIAL",
    "ALTER TABLE billing_accounts ADD COLUMN IF NOT EXISTS credit_limit NUMERIC(20, 8) NOT NULL DEFAULT 0.0",
    "ALTER TABLE billing_accounts ADD COLUMN IF NOT EXISTS currency TEXT NOT NULL DEFAULT 'CNY'",
    "CREATE UNIQUE INDEX IF NOT EXISTS idx_billing_accounts_id ON billing_accounts (id)",
];

pub(super) const MIGRATE_NUMBER_INVENTORY_SQL: &[&str] = &[
    "ALTER TABLE number_inventory ADD COLUMN IF NOT EXISTS gateway_id TEXT",
    "ALTER TABLE number_inventory ADD COLUMN IF NOT EXISTS direction VARCHAR(20) NOT NULL DEFAULT 'bidirectional'",
    "ALTER TABLE number_inventory ADD COLUMN IF NOT EXISTS max_concurrent INTEGER NOT NULL DEFAULT 10",
    "ALTER TABLE number_inventory ADD COLUMN IF NOT EXISTS current_concurrent INTEGER NOT NULL DEFAULT 0",
    "ALTER TABLE number_inventory ADD COLUMN IF NOT EXISTS updated_at TIMESTAMPTZ NOT NULL DEFAULT now()",
];

pub(super) const CREATE_NUMBERS_GATEWAY_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_number_inventory_gateway ON number_inventory (gateway_id)";
pub(super) const CREATE_NUMBERS_STATUS_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_number_inventory_status ON number_inventory (status)";
pub(super) const CREATE_NUMBERS_USERNAME_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_number_inventory_username ON number_inventory (username)";

pub(super) const CREATE_GATEWAY_NUMBER_ASSIGNMENTS_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS gateway_number_assignments (
    id BIGSERIAL PRIMARY KEY,
    gateway_id TEXT NOT NULL REFERENCES sip_gateways(id) ON DELETE CASCADE,
    number TEXT NOT NULL REFERENCES number_inventory(number) ON DELETE CASCADE,
    direction VARCHAR(20) NOT NULL DEFAULT 'both',
    max_concurrent INTEGER NOT NULL DEFAULT 10,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (gateway_id, number)
)
"#;

pub(super) const CREATE_GATEWAY_PEER_LINKS_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS gateway_peer_links (
    id BIGSERIAL PRIMARY KEY,
    gateway_id TEXT NOT NULL REFERENCES sip_gateways(id) ON DELETE CASCADE,
    peer_gateway_id TEXT NOT NULL REFERENCES sip_gateways(id) ON DELETE CASCADE,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (gateway_id, peer_gateway_id),
    CHECK (gateway_id <> peer_gateway_id)
)
"#;

pub(super) const CREATE_GATEWAY_ASSIGNMENT_INDEXES_SQL: &[&str] = &[
    "CREATE INDEX IF NOT EXISTS idx_gna_gateway ON gateway_number_assignments (gateway_id)",
    "CREATE INDEX IF NOT EXISTS idx_gna_number ON gateway_number_assignments (number)",
    "CREATE INDEX IF NOT EXISTS idx_gpl_gateway ON gateway_peer_links (gateway_id)",
    "CREATE INDEX IF NOT EXISTS idx_gpl_peer_gateway ON gateway_peer_links (peer_gateway_id)",
];

pub(super) const ADD_GATEWAY_ACCOUNT_FOREIGN_KEY_SQL: &str = r#"
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'fk_gateway_account'
    ) THEN
        ALTER TABLE sip_gateways
            ADD CONSTRAINT fk_gateway_account
            FOREIGN KEY (account_id) REFERENCES billing_accounts(id)
            ON DELETE SET NULL NOT VALID;
    END IF;
END $$;
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
    query_params TEXT,
    request_body TEXT,
    status_code INTEGER NOT NULL,
    source_ip INET,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
)
"#;

pub(super) const CREATE_AUDIT_LOGS_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_api_audit_logs_created_at ON api_audit_logs (created_at DESC)";

pub(super) const CREATE_SIP_FLOWS_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS sip_flows (
    id BIGSERIAL,
    call_id TEXT NOT NULL,
    method TEXT NOT NULL,
    direction TEXT NOT NULL,
    from_addr TEXT NOT NULL,
    to_addr TEXT NOT NULL,
    raw_message TEXT NOT NULL,
    timestamp TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (id, timestamp)
) PARTITION BY RANGE (timestamp)
"#;

pub(super) const CREATE_SIP_FLOWS_CALL_ID_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_sip_flows_call_id ON sip_flows (call_id)";

pub(super) const CREATE_SIP_FLOWS_TIMESTAMP_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_sip_flows_timestamp ON sip_flows (timestamp)";

pub const CREATE_SYSTEM_CONFIGS_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS system_configs (
    config_key TEXT PRIMARY KEY,
    config_value TEXT NOT NULL,
    description TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
)
"#;

pub const SEED_SYSTEM_CONFIGS_SQL: &str = r#"
INSERT INTO system_configs (config_key, config_value, description) VALUES
    ('session_expires_gateway', '600', '网关会话超时时长'),
    ('session_expires_caller', '1800', '呼叫方会话超时时长'),
    ('database_routes_enabled', 'true', '启用数据库动态路由'),
    ('default_gateway', '', '无数据库路由时的默认网关'),
    ('gateway_health_checks_enabled', 'true', '启用网关健康检查'),
    ('cluster_enabled', 'false', '启用 SIP 节点集群'),
    ('cluster_heartbeat_interval_secs', '3', 'SIP 节点心跳间隔'),
    ('cluster_node_timeout_secs', '10', 'SIP 节点离线判定时间'),
    ('cluster_dialog_ttl_secs', '86400', '集群对话快照保留时间'),
    ('sbc_rate_limit_enabled', 'true', '启用 SBC 来源限速'),
    ('sbc_rate_limit_capacity', '2000.0', 'SBC 限速令牌桶容量'),
    ('sbc_rate_limit_fill_rate', '500.0', 'SBC 限速令牌填充速率'),
    ('sbc_max_concurrency', '2000', '每个分机最大并发数'),
    ('tls_allow_test_certificate', 'false', '允许自签名/测试证书'),
    ('tls_insecure_skip_verify', 'false', '跳过 TLS 校验'),
    ('udp_workers', '4', 'UDP工作线程数'),
    ('udp_workers_auto', 'false', '自动调整工作线程数'),
    ('udp_receive_buffer_bytes', '4194304', 'UDP接收缓冲区字节数'),
    ('udp_send_buffer_bytes', '4194304', 'UDP发送缓冲区字节数'),
    ('cdr_queue_capacity', '4096', 'CDR 内存有界队列容量'),
    ('cdr_persistence_enabled', 'true', '启用 CDR 持久化'),
    ('rtp_symmetric_learning', 'true', '启用对称 RTP 学习'),
    ('rtp_anti_spoofing', 'true', 'RTP 源地址欺骗防护'),
    ('rtp_source_relearn_secs', '30', 'RTP 重新学习周期'),
    ('media_metrics_log', 'false', '通话结束时输出媒体指标明细'),
    ('recording_enabled', 'false', '全局录音开关'),
    ('recording_dir', 'target/recordings', '本地录音保存路径'),
    ('recording_workers', '4', '录音独立线程数'),
    ('recording_queue_capacity', '4096', '录音管道深度'),
    ('recording_retention_secs', '604800', '本地录音留存期'),
    ('recording_min_free_bytes', '536870912', '录音磁盘保护大小阀值'),
    ('recording_max_file_bytes', '134217728', '单 WAV 录音文件最大字节数'),
    ('recording_max_duration_secs', '3600', '单录音最长时长限制'),
    ('balance_enforcement_enabled', 'true', '启用实时余额校验'),
    ('billing_settlement_enabled', 'true', '启用通话结束计费结算'),
    ('storage_backend', 'local', '录音存储后端类型 (local/oss/dual)'),
    ('tls_bind_addr', '', 'SIP TLS 监听地址'),
    ('tls_cert_path', '', 'SIP TLS 证书路径'),
    ('tls_key_path', '', 'SIP TLS 私钥路径'),
    ('tls_ca_path', '', '上游 TLS CA 证书路径'),
    ('tls_server_name', '', '上游 TLS Server Name'),
    ('realm', 'vos-rs', 'SIP 挑战认证 Realm'),
    ('nonce', 'vos-rs-dev-nonce', 'SIP 静态 Nonce'),
    ('secret_key', 'default-fallback-secret-key-12345', 'SIP 鉴权密钥'),
    ('sipflow_enabled', 'true', '启用 SipFlow 信令抓包'),
    ('sipflow_whitelist', '1001,1002', 'SipFlow 抓包白名单（分机号/号码/网关，逗号分隔）'),
    ('sipflow_retention_days', '7', 'SipFlow 信令数据留存天数')
ON CONFLICT (config_key) DO NOTHING
"#;

#[cfg(test)]
mod tests {
    use super::SEED_SYSTEM_CONFIGS_SQL;

    #[test]
    fn system_config_seed_covers_high_frequency_domains() {
        for key in [
            "session_expires_gateway",
            "database_routes_enabled",
            "gateway_health_checks_enabled",
            "rtp_symmetric_learning",
            "recording_enabled",
            "balance_enforcement_enabled",
            "billing_settlement_enabled",
            "sbc_rate_limit_enabled",
            "cluster_heartbeat_interval_secs",
            "cluster_node_timeout_secs",
            "cdr_persistence_enabled",
        ] {
            assert!(
                SEED_SYSTEM_CONFIGS_SQL.contains(&format!("('{key}',")),
                "missing default for {key}"
            );
        }
        assert!(SEED_SYSTEM_CONFIGS_SQL.contains("ON CONFLICT (config_key) DO NOTHING"));
    }
}
