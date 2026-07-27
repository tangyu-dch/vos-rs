pub(crate) const CREATE_CDR_TABLE_SQL: &str = r#"
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

pub(crate) const MIGRATE_CDR_AUDIT_SQL: &str =
    "ALTER TABLE call_cdrs ADD COLUMN IF NOT EXISTS audit JSONB NOT NULL DEFAULT '{}'::jsonb";

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
