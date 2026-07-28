pub(crate) const CREATE_ANTI_FRAUD_RULES_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS anti_fraud_rules (
    id TEXT PRIMARY KEY,
    rule_type TEXT NOT NULL,
    target_value TEXT NOT NULL,
    limit_number INTEGER,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
)
"#;

pub(crate) const CREATE_ANTI_FRAUD_CONFIG_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS anti_fraud_config (
    config_key TEXT PRIMARY KEY,
    config_value TEXT NOT NULL,
    description TEXT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
)
"#;

pub(crate) const MIGRATE_LEGACY_ANTI_FRAUD_RULES_SQL: &str = r#"
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
        ALTER TABLE anti_fraud_rules ALTER COLUMN value DROP NOT NULL;
    END IF;
END $$;
"#;

pub(crate) const MIGRATE_LEGACY_ANTI_FRAUD_RULES_STEP2_SQL: &str = r#"
ALTER TABLE anti_fraud_rules
    ALTER COLUMN id TYPE TEXT USING id::TEXT;
"#;

pub(crate) const MIGRATE_LEGACY_ANTI_FRAUD_RULES_STEP3_SQL: &str = r#"
UPDATE anti_fraud_rules
SET target_value = COALESCE(target_value, '')
WHERE target_value IS NULL;
"#;

pub(crate) const MIGRATE_LEGACY_ANTI_FRAUD_RULES_STEP4_SQL: &str = r#"
ALTER TABLE anti_fraud_rules
    ALTER COLUMN target_value SET NOT NULL;
"#;

pub(crate) const SEED_ANTI_FRAUD_CONFIG_SQL: &str = r#"
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

pub(crate) const CREATE_BILLING_RATES_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS billing_rates (
    id TEXT PRIMARY KEY,
    prefix TEXT NOT NULL,
    rate_per_minute NUMERIC(20, 8) NOT NULL,
    billing_interval_secs INTEGER NOT NULL DEFAULT 60 CHECK (billing_interval_secs > 0),
    price_per_interval NUMERIC(20, 8) NOT NULL DEFAULT 0,
    description TEXT,
    tenant_id TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
)
"#;

/// billing_rates 添加 tenant_id 字段。
///
/// 可空（NULL 表示全局费率，对所有租户生效）。
pub(crate) const MIGRATE_BILLING_RATES_TENANT_SQL: &str =
    "ALTER TABLE billing_rates ADD COLUMN IF NOT EXISTS tenant_id TEXT";

pub(crate) const CREATE_BILLING_RATES_TENANT_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_billing_rates_tenant ON billing_rates (tenant_id)";

pub(crate) const CREATE_BILLING_ACCOUNTS_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS billing_accounts (
    id BIGSERIAL UNIQUE,
    username TEXT PRIMARY KEY,
    balance NUMERIC(20, 8) NOT NULL DEFAULT 0.0,
    credit_limit NUMERIC(20, 8) NOT NULL DEFAULT 0.0,
    currency TEXT NOT NULL DEFAULT 'CNY',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
)
"#;

pub(crate) const CREATE_BILLING_LEDGER_TABLE_SQL: &str = r#"
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

pub(crate) const CREATE_BILLING_CREDITS_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS billing_credits (
    idempotency_key TEXT PRIMARY KEY,
    username TEXT NOT NULL,
    amount NUMERIC(20, 8) NOT NULL CHECK (amount > 0),
    balance_after NUMERIC(20, 8) NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
)
"#;

pub(crate) const CREATE_BILLING_CREDITS_USERNAME_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_billing_credits_username ON billing_credits (username, created_at DESC)";

pub(crate) const CREATE_LEDGER_USERNAME_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_billing_ledger_username ON billing_ledger (username)";
pub(crate) const CREATE_LEDGER_CREATED_AT_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_billing_ledger_created_at ON billing_ledger (created_at DESC)";

pub(crate) const MIGRATE_BILLING_INTERVALS_SQL: &str = r#"
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

pub(crate) const CREATE_NUMBER_INVENTORY_TABLE_SQL: &str = r#"
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

pub(crate) const MIGRATE_BILLING_ACCOUNTS_SQL: &[&str] = &[
    "ALTER TABLE billing_accounts ADD COLUMN IF NOT EXISTS id BIGSERIAL",
    "ALTER TABLE billing_accounts ADD COLUMN IF NOT EXISTS credit_limit NUMERIC(20, 8) NOT NULL DEFAULT 0.0",
    "ALTER TABLE billing_accounts ADD COLUMN IF NOT EXISTS currency TEXT NOT NULL DEFAULT 'CNY'",
    "CREATE UNIQUE INDEX IF NOT EXISTS idx_billing_accounts_id ON billing_accounts (id)",
];

pub(crate) const MIGRATE_NUMBER_INVENTORY_SQL: &[&str] = &[
    "ALTER TABLE number_inventory ADD COLUMN IF NOT EXISTS gateway_id TEXT",
    "ALTER TABLE number_inventory ADD COLUMN IF NOT EXISTS direction VARCHAR(20) NOT NULL DEFAULT 'bidirectional'",
    "ALTER TABLE number_inventory ADD COLUMN IF NOT EXISTS max_concurrent INTEGER NOT NULL DEFAULT 10",
    "ALTER TABLE number_inventory ADD COLUMN IF NOT EXISTS current_concurrent INTEGER NOT NULL DEFAULT 0",
    "ALTER TABLE number_inventory ADD COLUMN IF NOT EXISTS updated_at TIMESTAMPTZ NOT NULL DEFAULT now()",
];

pub(crate) const CREATE_NUMBERS_GATEWAY_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_number_inventory_gateway ON number_inventory (gateway_id)";
pub(crate) const CREATE_NUMBERS_STATUS_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_number_inventory_status ON number_inventory (status)";
pub(crate) const CREATE_NUMBERS_USERNAME_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_number_inventory_username ON number_inventory (username)";

pub(crate) const CREATE_GATEWAY_NUMBER_ASSIGNMENTS_TABLE_SQL: &str = r#"
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

pub(crate) const CREATE_GATEWAY_PEER_LINKS_TABLE_SQL: &str = r#"
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

pub(crate) const CREATE_GATEWAY_ASSIGNMENT_INDEXES_SQL: &[&str] = &[
    "CREATE INDEX IF NOT EXISTS idx_gna_gateway ON gateway_number_assignments (gateway_id)",
    "CREATE INDEX IF NOT EXISTS idx_gna_number ON gateway_number_assignments (number)",
    "CREATE INDEX IF NOT EXISTS idx_gpl_gateway ON gateway_peer_links (gateway_id)",
    "CREATE INDEX IF NOT EXISTS idx_gpl_peer_gateway ON gateway_peer_links (peer_gateway_id)",
];

pub(crate) const ADD_GATEWAY_ACCOUNT_FOREIGN_KEY_SQL: &str = r#"
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

pub(crate) const CREATE_AUDIT_LOGS_TABLE_SQL: &str = r#"
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

pub(crate) const CREATE_AUDIT_LOGS_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_api_audit_logs_created_at ON api_audit_logs (created_at DESC)";

pub(crate) const CREATE_SIP_FLOWS_TABLE_SQL: &str = r#"
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

pub(crate) const CREATE_SIP_FLOWS_CALL_ID_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_sip_flows_call_id ON sip_flows (call_id)";

pub(crate) const CREATE_SIP_FLOWS_TIMESTAMP_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_sip_flows_timestamp ON sip_flows (timestamp)";

pub(crate) const CREATE_SYSTEM_CONFIGS_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS system_configs (
    config_key TEXT PRIMARY KEY,
    config_value TEXT NOT NULL,
    description TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
)
"#;

pub(crate) const SEED_SYSTEM_CONFIGS_SQL: &str = r#"
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

pub(crate) const CREATE_COPILOT_SESSIONS_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS copilot_sessions (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    operator TEXT NOT NULL,
    llm_provider TEXT,
    llm_model TEXT,
    pinned BOOLEAN NOT NULL DEFAULT FALSE,
    archived BOOLEAN NOT NULL DEFAULT FALSE,
    message_count INTEGER NOT NULL DEFAULT 0,
    last_message_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
)
"#;

pub(crate) const CREATE_COPILOT_SESSIONS_OPERATOR_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_copilot_sessions_operator ON copilot_sessions (operator, last_message_at DESC NULLS LAST)";

pub(crate) const CREATE_COPILOT_MESSAGES_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS copilot_messages (
    id BIGSERIAL PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES copilot_sessions(id) ON DELETE CASCADE,
    role TEXT NOT NULL,
    content TEXT NOT NULL,
    images TEXT[],
    root_cause TEXT,
    suggested_action TEXT,
    ladder_diagram_ascii TEXT,
    llm_enabled BOOLEAN,
    llm_status TEXT,
    intent TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
)
"#;

pub(crate) const MIGRATE_COPILOT_MESSAGES_IMAGES_SQL: &str =
    "ALTER TABLE copilot_messages ADD COLUMN IF NOT EXISTS images TEXT[];";

pub(crate) const CREATE_COPILOT_MESSAGES_SESSION_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_copilot_messages_session ON copilot_messages (session_id, created_at)";

pub(crate) const CREATE_LLM_CONFIGS_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS llm_configs (
    id BIGSERIAL PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    provider TEXT NOT NULL,
    api_key TEXT NOT NULL,
    base_url TEXT NOT NULL,
    model TEXT NOT NULL,
    temperature REAL NOT NULL DEFAULT 0.3,
    is_active BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
)
"#;

pub(crate) const CREATE_LLM_CONFIGS_ACTIVE_INDEX_SQL: &str =
    "CREATE UNIQUE INDEX IF NOT EXISTS idx_llm_configs_active_singleton ON llm_configs (is_active) WHERE is_active = true";

pub(crate) const SEED_DEFAULT_LLM_CONFIG_SQL: &str = r#"
INSERT INTO llm_configs (name, provider, api_key, base_url, model, temperature, is_active)
SELECT '智谱 GLM-4.7-flash (默认)', 'zhipu',
       '6f86ed5fe1c04366918e12e5170f4660.CRsePLgiumNbWmh0',
       'https://open.bigmodel.cn/api/paas/v4', 'glm-4.7-flash', 0.3, true
WHERE NOT EXISTS (SELECT 1 FROM llm_configs)
"#;
