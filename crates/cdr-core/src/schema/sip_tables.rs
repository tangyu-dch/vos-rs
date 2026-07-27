pub(crate) const CREATE_SIP_USERS_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS sip_users (
    username TEXT PRIMARY KEY,
    password TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
)
"#;

pub(crate) const CREATE_SIP_GATEWAYS_TABLE_SQL: &str = r#"
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

pub(crate) const MIGRATE_SIP_GATEWAYS_SQL: &[&str] = &[
    "ALTER TABLE sip_gateways ADD COLUMN IF NOT EXISTS gateway_type VARCHAR(20) NOT NULL DEFAULT 'peer'",
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
pub(crate) const CREATE_GATEWAYS_ENABLED_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_sip_gateways_enabled ON sip_gateways (enabled)";

pub(crate) const CREATE_SIP_ROUTES_TABLE_SQL: &str = r#"
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

pub(crate) const MIGRATION_ADD_ROUTE_WEIGHT: &str =
    "ALTER TABLE sip_routes ADD COLUMN IF NOT EXISTS weight INTEGER NOT NULL DEFAULT 100";

pub(crate) const MIGRATION_ADD_ROUTE_TOPOLOGY: &str =
    "ALTER TABLE sip_routes ADD COLUMN IF NOT EXISTS topology JSONB NOT NULL DEFAULT '{}'::jsonb";

pub(crate) const CREATE_SIP_REGISTRATIONS_TABLE_SQL: &str = r#"
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
