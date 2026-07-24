//! # Copilot 历史会话存储
//!
//! 提供 Copilot 运维助手多轮对话的持久化能力：
//! - 会话元数据（标题、操作员、置顶/归档标记、消息计数、最后消息时间）
//! - 逐条消息（含 LLM 返回的根因、建议动作、SIP 梯形图、LLM 状态等）
//!
//! 所有方法返回 `Result`，不使用 panic。会话删除时通过外键 `ON DELETE CASCADE`
//! 自动级联清理消息。

use crate::PostgresCdrStore;
use sqlx::Row;
use time::OffsetDateTime;

/// Copilot 会话元数据
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
pub struct CopilotSession {
    pub id: String,
    pub title: String,
    pub operator: String,
    pub llm_provider: Option<String>,
    pub llm_model: Option<String>,
    pub pinned: bool,
    pub archived: bool,
    pub message_count: i32,
    #[serde(with = "time::serde::rfc3339::option")]
    pub last_message_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

/// Copilot 单条消息
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
pub struct CopilotMessage {
    pub id: i64,
    pub session_id: String,
    /// 角色：`user` | `assistant`
    pub role: String,
    pub content: String,
    pub images: Option<Vec<String>>,
    pub root_cause: Option<String>,
    pub suggested_action: Option<String>,
    pub ladder_diagram_ascii: Option<String>,
    pub llm_enabled: Option<bool>,
    pub llm_status: Option<String>,
    /// 意图分类（仅 assistant 消息携带）：SystemHealth/CallFailure/...
    pub intent: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

/// 追加消息时的输入参数，避免方法签名过长
#[derive(Debug, Clone)]
pub struct AppendCopilotMessageInput<'a> {
    pub session_id: &'a str,
    pub role: &'a str,
    pub content: &'a str,
    pub images: Option<&'a [String]>,
    pub root_cause: Option<&'a str>,
    pub suggested_action: Option<&'a str>,
    pub ladder_diagram_ascii: Option<&'a str>,
    pub llm_enabled: Option<bool>,
    pub llm_status: Option<&'a str>,
    pub intent: Option<&'a str>,
}

impl PostgresCdrStore {
    /// 创建新的 Copilot 会话。`id` 由调用方生成（建议 UUID v4）。
    pub async fn create_copilot_session(
        &self,
        id: &str,
        title: &str,
        operator: &str,
        llm_provider: Option<&str>,
        llm_model: Option<&str>,
    ) -> Result<CopilotSession, sqlx::Error> {
        let row = sqlx::query_as::<_, CopilotSession>(
            "INSERT INTO copilot_sessions (id, title, operator, llm_provider, llm_model) \
             VALUES ($1, $2, $3, $4, $5) \
             RETURNING id, title, operator, llm_provider, llm_model, pinned, archived, \
                       message_count, last_message_at, created_at, updated_at",
        )
        .bind(id)
        .bind(title)
        .bind(operator)
        .bind(llm_provider)
        .bind(llm_model)
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }

    /// 列出指定操作员的会话（默认按更新时间倒序）。
    pub async fn list_copilot_sessions(
        &self,
        operator: &str,
        limit: i64,
        include_archived: bool,
    ) -> Result<Vec<CopilotSession>, sqlx::Error> {
        let limit = limit.clamp(1, 100);
        let rows = if include_archived {
            sqlx::query_as::<_, CopilotSession>(
                "SELECT id, title, operator, llm_provider, llm_model, pinned, archived, \
                        message_count, last_message_at, created_at, updated_at \
                 FROM copilot_sessions WHERE operator = $1 \
                 ORDER BY pinned DESC, updated_at DESC LIMIT $2",
            )
            .bind(operator)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, CopilotSession>(
                "SELECT id, title, operator, llm_provider, llm_model, pinned, archived, \
                        message_count, last_message_at, created_at, updated_at \
                 FROM copilot_sessions WHERE operator = $1 AND archived = false \
                 ORDER BY pinned DESC, updated_at DESC LIMIT $2",
            )
            .bind(operator)
            .bind(limit)
            .fetch_all(&self.pool)
            .await?
        };
        Ok(rows)
    }

    /// 获取单个会话详情。确保 `operator` 匹配，隔离越权访问。
    pub async fn get_copilot_session(
        &self,
        id: &str,
        operator: &str,
    ) -> Result<Option<CopilotSession>, sqlx::Error> {
        let row = sqlx::query_as::<_, CopilotSession>(
            "SELECT id, title, operator, llm_provider, llm_model, pinned, archived, \
                    message_count, last_message_at, created_at, updated_at \
             FROM copilot_sessions WHERE id = $1 AND operator = $2",
        )
        .bind(id)
        .bind(operator)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// 更新会话属性（标题 / 置顶 / 归档）。仅修改传入 `Some` 的字段。
    pub async fn update_copilot_session(
        &self,
        id: &str,
        operator: &str,
        title: Option<&str>,
        pinned: Option<bool>,
        archived: Option<bool>,
    ) -> Result<Option<CopilotSession>, sqlx::Error> {
        let row = sqlx::query_as::<_, CopilotSession>(
            "UPDATE copilot_sessions \
             SET title = COALESCE($3, title), \
                 pinned = COALESCE($4, pinned), \
                 archived = COALESCE($5, archived), \
                 updated_at = now() \
             WHERE id = $1 AND operator = $2 \
             RETURNING id, title, operator, llm_provider, llm_model, pinned, archived, \
                       message_count, last_message_at, created_at, updated_at",
        )
        .bind(id)
        .bind(operator)
        .bind(title)
        .bind(pinned)
        .bind(archived)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// 删除指定会话（触发外键级联清理消息）。隔离操作员权限。
    pub async fn delete_copilot_session(
        &self,
        id: &str,
        operator: &str,
    ) -> Result<bool, sqlx::Error> {
        let affected = sqlx::query("DELETE FROM copilot_sessions WHERE id = $1 AND operator = $2")
            .bind(id)
            .bind(operator)
            .execute(&self.pool)
            .await?
            .rows_affected();
        Ok(affected > 0)
    }

    /// 追加一条消息并联动更新会话的 `message_count` / `last_message_at` / `updated_at`。
    /// 全部在单事务内完成，避免半写状态。
    pub async fn append_copilot_message(
        &self,
        input: AppendCopilotMessageInput<'_>,
    ) -> Result<CopilotMessage, sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query(
            "INSERT INTO copilot_messages \
                (session_id, role, content, images, root_cause, suggested_action, \
                 ladder_diagram_ascii, llm_enabled, llm_status, intent) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) \
             RETURNING id, session_id, role, content, images, root_cause, suggested_action, \
                       ladder_diagram_ascii, llm_enabled, llm_status, intent, created_at",
        )
        .bind(input.session_id)
        .bind(input.role)
        .bind(input.content)
        .bind(input.images)
        .bind(input.root_cause)
        .bind(input.suggested_action)
        .bind(input.ladder_diagram_ascii)
        .bind(input.llm_enabled)
        .bind(input.llm_status)
        .bind(input.intent)
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE copilot_sessions \
             SET message_count = message_count + 1, \
                 last_message_at = now(), \
                 updated_at = now() \
             WHERE id = $1",
        )
        .bind(input.session_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(CopilotMessage {
            id: row.get(0),
            session_id: row.get(1),
            role: row.get(2),
            content: row.get(3),
            images: row.get(4),
            root_cause: row.get(5),
            suggested_action: row.get(6),
            ladder_diagram_ascii: row.get(7),
            llm_enabled: row.get(8),
            llm_status: row.get(9),
            intent: row.get(10),
            created_at: row.get(11),
        })
    }

    /// 列出指定会话的消息（按时间正序，便于聊天 UI 渲染）。
    /// 上限 500 条，避免历史过长时一次拉取过多。
    pub async fn list_copilot_messages(
        &self,
        session_id: &str,
        limit: i64,
    ) -> Result<Vec<CopilotMessage>, sqlx::Error> {
        let limit = limit.clamp(1, 500);
        let rows = sqlx::query_as::<_, CopilotMessage>(
            "SELECT id, session_id, role, content, images, root_cause, suggested_action, \
                    ladder_diagram_ascii, llm_enabled, llm_status, intent, created_at \
             FROM copilot_messages WHERE session_id = $1 \
             ORDER BY created_at ASC, id ASC LIMIT $2",
        )
        .bind(session_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }
}
