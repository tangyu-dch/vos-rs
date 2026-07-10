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
