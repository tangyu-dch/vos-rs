//! 公告内容、投放范围与每用户阅读回执表结构。

pub(crate) const CREATE_ANNOUNCEMENTS_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS announcements (
    id BIGSERIAL PRIMARY KEY,
    title TEXT NOT NULL,
    category TEXT NOT NULL,
    content TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'draft' CHECK (status IN ('draft', 'published')),
    audience TEXT NOT NULL DEFAULT 'all' CHECK (audience IN ('all', 'specified')),
    audience_users TEXT[] NOT NULL DEFAULT '{}'::TEXT[],
    delivery_methods TEXT[] NOT NULL DEFAULT ARRAY['system']::TEXT[],
    scheduled_at TIMESTAMPTZ,
    published_at TIMESTAMPTZ,
    pinned BOOLEAN NOT NULL DEFAULT FALSE,
    publisher TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (audience = 'all' OR cardinality(audience_users) > 0),
    CHECK (cardinality(delivery_methods) > 0),
    CHECK (delivery_methods <@ ARRAY['system', 'popup']::TEXT[])
)
"#;

pub(crate) const CREATE_ANNOUNCEMENTS_STATUS_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_announcements_status_time ON announcements (status, scheduled_at, created_at DESC)";
pub(crate) const CREATE_ANNOUNCEMENTS_CATEGORY_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_announcements_category ON announcements (category, created_at DESC)";

pub(crate) const CREATE_ANNOUNCEMENT_READS_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS announcement_reads (
    announcement_id BIGINT NOT NULL REFERENCES announcements(id) ON DELETE CASCADE,
    operator TEXT NOT NULL,
    read_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (announcement_id, operator)
)
"#;

pub(crate) const CREATE_ANNOUNCEMENT_READS_OPERATOR_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_announcement_reads_operator ON announcement_reads (operator, read_at DESC)";
