pub(crate) const CREATE_CDR_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS call_cdrs (
    id BIGSERIAL PRIMARY KEY,
    call_id TEXT NOT NULL,
    caller TEXT,
    callee TEXT,
    started_at TIMESTAMPTZ NOT NULL,
    ringing_at TIMESTAMPTZ,
    answered_at TIMESTAMPTZ,
    ended_at TIMESTAMPTZ NOT NULL,
    duration_ms BIGINT NOT NULL,
    billable_duration_ms BIGINT NOT NULL,
    talk_duration_ms BIGINT,
    ringing_duration_ms BIGINT,
    access_billable_duration_ms BIGINT,
    access_charge_amount DOUBLE PRECISION,
    egress_billable_duration_ms BIGINT,
    egress_cost_amount DOUBLE PRECISION,
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

pub(crate) const MIGRATE_CDR_AUDIT_SQL: &str =
    "ALTER TABLE call_cdrs ADD COLUMN IF NOT EXISTS audit JSONB NOT NULL DEFAULT '{}'::jsonb";

pub(crate) const MIGRATE_CDR_TIMING_BILLING_SQL: &str = r#"
ALTER TABLE call_cdrs ADD COLUMN IF NOT EXISTS ringing_at TIMESTAMPTZ;
ALTER TABLE call_cdrs ADD COLUMN IF NOT EXISTS talk_duration_ms BIGINT;
ALTER TABLE call_cdrs ADD COLUMN IF NOT EXISTS ringing_duration_ms BIGINT;
ALTER TABLE call_cdrs ADD COLUMN IF NOT EXISTS access_billable_duration_ms BIGINT;
ALTER TABLE call_cdrs ADD COLUMN IF NOT EXISTS access_charge_amount DOUBLE PRECISION;
ALTER TABLE call_cdrs ADD COLUMN IF NOT EXISTS egress_billable_duration_ms BIGINT;
ALTER TABLE call_cdrs ADD COLUMN IF NOT EXISTS egress_cost_amount DOUBLE PRECISION;
"#;

pub(crate) const MIGRATE_CDR_TENANT_REALM_SQL: &[&str] = &[
    "ALTER TABLE call_cdrs ADD COLUMN IF NOT EXISTS tenant_id TEXT",
    "ALTER TABLE call_cdrs ADD COLUMN IF NOT EXISTS tenant_name TEXT",
    "ALTER TABLE call_cdrs ADD COLUMN IF NOT EXISTS auth_realm TEXT",
    "CREATE INDEX IF NOT EXISTS idx_call_cdrs_tenant ON call_cdrs (tenant_id)",
    "CREATE INDEX IF NOT EXISTS idx_call_cdrs_realm ON call_cdrs (auth_realm)",
];

/// 回填历史 CDR 的振铃时长和通话时长。
///
/// - `ringing_duration_ms`：振铃时间到接通时间（未接通则到结束时间）的毫秒数
/// - `talk_duration_ms`：接通时间到结束时间的毫秒数
///
/// 仅更新值为 NULL 的行，可安全重复执行。
pub(crate) const BACKFILL_CDR_TIMING_SQL: &str = r#"
UPDATE call_cdrs
SET ringing_duration_ms = GREATEST(0,
    EXTRACT(EPOCH FROM (COALESCE(answered_at, ended_at) - ringing_at)) * 1000)::BIGINT
WHERE ringing_duration_ms IS NULL
  AND ringing_at IS NOT NULL;
UPDATE call_cdrs
SET talk_duration_ms = GREATEST(0,
    EXTRACT(EPOCH FROM (ended_at - answered_at)) * 1000)::BIGINT
WHERE talk_duration_ms IS NULL
  AND answered_at IS NOT NULL;
"#;

pub(crate) const CREATE_CALL_ID_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_call_cdrs_call_id ON call_cdrs (call_id)";

pub(crate) const MIGRATE_CDR_IDEMPOTENCY_SQL: &str = r#"
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

pub(crate) const CREATE_STARTED_AT_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_call_cdrs_started_at ON call_cdrs (started_at)";

pub(crate) const CREATE_STATUS_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_call_cdrs_status ON call_cdrs (status)";

pub(crate) const CREATE_CDR_CALLER_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_call_cdrs_caller ON call_cdrs (caller)";

pub(crate) const CREATE_CDR_CALLEE_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_call_cdrs_callee ON call_cdrs (callee)";

pub(crate) const CREATE_CDR_DIRECTION_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_call_cdrs_direction ON call_cdrs (direction)";

/// 优化 CDR 列表查询：按状态筛选并按时间倒序分页。
pub(crate) const CREATE_CDR_STATUS_STARTED_AT_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_call_cdrs_status_started_at ON call_cdrs (status, started_at DESC)";

/// 优化按主叫 + 时间范围查询的报表场景。
pub(crate) const CREATE_CDR_CALLER_STARTED_AT_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_call_cdrs_caller_started_at ON call_cdrs (caller, started_at DESC)";

/// 优化按被叫 + 时间范围查询的报表场景。
pub(crate) const CREATE_CDR_CALLEE_STARTED_AT_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_call_cdrs_callee_started_at ON call_cdrs (callee, started_at DESC)";

pub(crate) const CREATE_DTMF_EVENTS_TABLE_SQL: &str = r#"
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

pub(crate) const CREATE_DTMF_CALL_ID_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_dtmf_events_call_id ON dtmf_events (call_id)";
