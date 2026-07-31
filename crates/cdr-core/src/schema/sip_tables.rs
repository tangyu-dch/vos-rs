pub(crate) const CREATE_SIP_USERS_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS sip_users (
    username TEXT PRIMARY KEY,
    password TEXT NOT NULL,
    tenant_id TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
)
"#;

/// sip_users 添加 tenant_id 字段，引用 tenants(id)。
///
/// 可空（NULL 表示分机未关联租户，向后兼容旧数据）。
pub(crate) const MIGRATE_SIP_USERS_TENANT_SQL: &str =
    "ALTER TABLE sip_users ADD COLUMN IF NOT EXISTS tenant_id TEXT";

pub(crate) const CREATE_SIP_USERS_TENANT_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_sip_users_tenant ON sip_users (tenant_id)";

pub(crate) const CREATE_SIP_USERS_UNIQUE_TENANT_USERNAME_INDEX_SQL: &str =
    "CREATE UNIQUE INDEX IF NOT EXISTS idx_sip_users_unique_tenant_username ON sip_users (COALESCE(tenant_id, ''), username)";

pub(crate) const CREATE_SIP_GATEWAYS_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS sip_gateways (
    id TEXT PRIMARY KEY,
    host TEXT NOT NULL,
    port INTEGER,
    transport TEXT NOT NULL DEFAULT 'udp',
    max_capacity INTEGER,
    gateway_type VARCHAR(20) NOT NULL DEFAULT 'peer',
    role VARCHAR(20) NOT NULL DEFAULT 'access',
    access_auth_mode VARCHAR(20) NOT NULL DEFAULT 'none',
    access_username TEXT NOT NULL DEFAULT '',
    access_realm TEXT NOT NULL DEFAULT '',
    access_password_hash TEXT NOT NULL DEFAULT '',
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
    tenant_id TEXT,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
)
"#;

pub(crate) const MIGRATE_SIP_GATEWAYS_SQL: &[&str] = &[
    "ALTER TABLE sip_gateways ADD COLUMN IF NOT EXISTS gateway_type VARCHAR(20) NOT NULL DEFAULT 'peer'",
    "ALTER TABLE sip_gateways ADD COLUMN IF NOT EXISTS role VARCHAR(20) NOT NULL DEFAULT 'access'",
    "ALTER TABLE sip_gateways ADD COLUMN IF NOT EXISTS access_auth_mode VARCHAR(20) NOT NULL DEFAULT 'none'",
    "ALTER TABLE sip_gateways ADD COLUMN IF NOT EXISTS access_username TEXT NOT NULL DEFAULT ''",
    "ALTER TABLE sip_gateways ADD COLUMN IF NOT EXISTS access_realm TEXT NOT NULL DEFAULT ''",
    "ALTER TABLE sip_gateways ADD COLUMN IF NOT EXISTS access_password_hash TEXT NOT NULL DEFAULT ''",
    "ALTER TABLE sip_gateways ADD COLUMN IF NOT EXISTS prefix_rules TEXT NOT NULL DEFAULT ''",
    "ALTER TABLE sip_gateways ADD COLUMN IF NOT EXISTS supports_registration BOOLEAN NOT NULL DEFAULT FALSE",
    "ALTER TABLE sip_gateways ADD COLUMN IF NOT EXISTS reg_auth_type VARCHAR(20) NOT NULL DEFAULT 'none'",
    "ALTER TABLE sip_gateways ADD COLUMN IF NOT EXISTS reg_username TEXT NOT NULL DEFAULT ''",
    "ALTER TABLE sip_gateways ADD COLUMN IF NOT EXISTS reg_password TEXT NOT NULL DEFAULT ''",
    "UPDATE sip_gateways SET reg_password = '' WHERE reg_password <> ''",
    "ALTER TABLE sip_gateways ADD COLUMN IF NOT EXISTS parent_gateway_id TEXT",
    "ALTER TABLE sip_gateways ADD COLUMN IF NOT EXISTS caller_id_mode VARCHAR(20) NOT NULL DEFAULT 'passthrough'",
    "ALTER TABLE sip_gateways ADD COLUMN IF NOT EXISTS virtual_caller TEXT NOT NULL DEFAULT ''",
    "ALTER TABLE sip_gateways ADD COLUMN IF NOT EXISTS current_concurrent INTEGER NOT NULL DEFAULT 0",
    "ALTER TABLE sip_gateways ADD COLUMN IF NOT EXISTS max_concurrent INTEGER NOT NULL DEFAULT 100",
    "ALTER TABLE sip_gateways ADD COLUMN IF NOT EXISTS account_id BIGINT",
    "ALTER TABLE sip_gateways ADD COLUMN IF NOT EXISTS tenant_id TEXT",
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

pub(crate) const CREATE_GATEWAYS_TYPE_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_sip_gateways_type ON sip_gateways (gateway_type)";
pub(crate) const CREATE_GATEWAYS_PARENT_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_sip_gateways_parent ON sip_gateways (parent_gateway_id)";
pub(crate) const CREATE_GATEWAYS_ACCOUNT_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_sip_gateways_account ON sip_gateways (account_id)";
pub(crate) const CREATE_GATEWAYS_TENANT_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_sip_gateways_tenant ON sip_gateways (tenant_id)";
pub(crate) const CREATE_GATEWAYS_ENABLED_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_sip_gateways_enabled ON sip_gateways (enabled)";

pub(crate) const ADD_GATEWAY_TENANT_FOREIGN_KEY_SQL: &str = r#"
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'fk_gateway_tenant'
    ) THEN
        ALTER TABLE sip_gateways
            ADD CONSTRAINT fk_gateway_tenant
            FOREIGN KEY (tenant_id) REFERENCES tenants(id)
            ON DELETE RESTRICT NOT VALID;
    END IF;
END $$;
"#;

pub(crate) const CREATE_SIP_ROUTES_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS sip_routes (
    id TEXT PRIMARY KEY,
    prefix TEXT NOT NULL,
    priority INTEGER NOT NULL DEFAULT 100,
    gateway_id TEXT NOT NULL REFERENCES sip_gateways(id) ON DELETE CASCADE,
    cost DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    weight INTEGER NOT NULL DEFAULT 100,
    tenant_id TEXT,
    strip_prefix TEXT NOT NULL DEFAULT '',
    add_prefix TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
)
"#;

pub(crate) const MIGRATION_ADD_ROUTE_WEIGHT: &str =
    "ALTER TABLE sip_routes ADD COLUMN IF NOT EXISTS weight INTEGER NOT NULL DEFAULT 100";

pub(crate) const MIGRATION_ADD_ROUTE_TENANT_ID: &str =
    "ALTER TABLE sip_routes ADD COLUMN IF NOT EXISTS tenant_id TEXT";

pub(crate) const MIGRATION_ADD_ROUTE_STRIP_PREFIX: &str =
    "ALTER TABLE sip_routes ADD COLUMN IF NOT EXISTS strip_prefix TEXT NOT NULL DEFAULT ''";

pub(crate) const MIGRATION_ADD_ROUTE_ADD_PREFIX: &str =
    "ALTER TABLE sip_routes ADD COLUMN IF NOT EXISTS add_prefix TEXT NOT NULL DEFAULT ''";

pub(crate) const CREATE_SIP_REGISTRATIONS_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS sip_registrations (
    aor TEXT NOT NULL,
    contact_uri TEXT NOT NULL,
    received_from TEXT NOT NULL,
    user_agent TEXT,
    expires_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    path TEXT,
    PRIMARY KEY (aor, contact_uri)
)
"#;

/// 新增 user_agent 列，记录 SIP 终端的 User-Agent 头（客户端名称）。
pub(crate) const MIGRATE_REGISTRATIONS_USER_AGENT_SQL: &str =
    "ALTER TABLE sip_registrations ADD COLUMN IF NOT EXISTS user_agent TEXT";

pub(crate) const CREATE_REGISTRATIONS_EXPIRES_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_sip_registrations_expires_at ON sip_registrations (expires_at)";

pub(crate) const CREATE_GATEWAY_HEALTH_TABLE_SQL: &str = r#"
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

pub(crate) const CREATE_GATEWAY_HEALTH_STATE_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_gateway_health_state ON gateway_health_status (state)";

pub(crate) const CREATE_ROUTES_PRIORITY_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_sip_routes_priority_id ON sip_routes (priority, id)";
pub(crate) const CREATE_ROUTES_PREFIX_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_sip_routes_prefix ON sip_routes (prefix)";
pub(crate) const CREATE_ROUTES_GATEWAY_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_sip_routes_gateway ON sip_routes (gateway_id)";

/// 多租户表：按 SIP 域名映射到 tenant_id 与运行时策略。
///
/// `enabled=FALSE` 的租户不会被加载到内存注册表，等价于不存在。
/// `cross_tenant_policy` 取值：`allow` / `deny` / `allow_if_same_domain`。
pub(crate) const CREATE_TENANTS_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS tenants (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    domain TEXT UNIQUE NOT NULL,
    max_concurrent_calls INTEGER NOT NULL DEFAULT 0,
    max_cps INTEGER NOT NULL DEFAULT 0,
    cross_tenant_policy TEXT NOT NULL DEFAULT 'allow_if_same_domain',
    recording_enabled BOOLEAN,
    allowed_gateway_ids JSONB,
    billing_account_id BIGINT,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
)
"#;

pub(crate) const CREATE_TENANTS_DOMAIN_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_tenants_domain ON tenants (domain) WHERE enabled = TRUE";

pub(crate) const CREATE_TENANTS_ENABLED_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_tenants_enabled ON tenants (enabled)";
