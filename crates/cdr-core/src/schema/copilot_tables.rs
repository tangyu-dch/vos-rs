//! Copilot 安全工具执行相关表结构。

pub(crate) const CREATE_COPILOT_ACTIONS_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS copilot_actions (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES copilot_sessions(id) ON DELETE CASCADE,
    operator TEXT NOT NULL,
    requested_role TEXT NOT NULL,
    tool_name TEXT NOT NULL,
    tool_arguments JSONB NOT NULL,
    risk_level TEXT NOT NULL CHECK (risk_level IN ('write', 'high_risk')),
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'executing', 'approved', 'rejected', 'failed')),
    reviewed_by TEXT,
    reviewed_role TEXT,
    review_note TEXT,
    result JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    reviewed_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ
)
"#;

pub(crate) const CREATE_COPILOT_ACTIONS_SESSION_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_copilot_actions_session ON copilot_actions (session_id, created_at DESC)";
