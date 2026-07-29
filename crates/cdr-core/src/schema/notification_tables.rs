//! 站内通知与每用户阅读回执表结构。

pub(crate) const CREATE_NOTIFICATIONS_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS notifications (
    id BIGSERIAL PRIMARY KEY,
    category TEXT NOT NULL CHECK (category IN ('server', 'trunk', 'registration', 'billing', 'call_quality', 'risk_control', 'security', 'system')),
    severity TEXT NOT NULL CHECK (severity IN ('info', 'warning', 'critical')),
    title TEXT NOT NULL,
    content TEXT NOT NULL,
    source TEXT NOT NULL,
    dedup_key TEXT NOT NULL,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    resolved_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
)
"#;

pub(crate) const CREATE_NOTIFICATIONS_ACTIVE_DEDUP_INDEX_SQL: &str =
    "CREATE UNIQUE INDEX IF NOT EXISTS idx_notifications_active_dedup ON notifications (dedup_key) WHERE resolved_at IS NULL";
pub(crate) const CREATE_NOTIFICATIONS_CREATED_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_notifications_created ON notifications (created_at DESC)";

pub(crate) const CREATE_NOTIFICATION_READS_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS notification_reads (
    notification_id BIGINT NOT NULL REFERENCES notifications(id) ON DELETE CASCADE,
    operator TEXT NOT NULL,
    read_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (notification_id, operator)
)
"#;

pub(crate) const CREATE_NOTIFICATION_READS_OPERATOR_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_notification_reads_operator ON notification_reads (operator, read_at DESC)";
