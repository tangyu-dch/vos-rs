//! Copilot 待审批工具动作的持久化与原子状态迁移。

use crate::PostgresCdrStore;
use serde_json::Value;
use time::OffsetDateTime;

/// Copilot 写操作审批记录。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, sqlx::FromRow)]
pub struct CopilotAction {
    pub id: String,
    pub session_id: String,
    pub operator: String,
    pub requested_role: String,
    pub tool_name: String,
    pub tool_arguments: Value,
    pub risk_level: String,
    pub status: String,
    pub reviewed_by: Option<String>,
    pub reviewed_role: Option<String>,
    pub review_note: Option<String>,
    pub result: Option<Value>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    pub reviewed_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub completed_at: Option<OffsetDateTime>,
}

impl PostgresCdrStore {
    /// 创建待审批动作；工具名称和参数在此固定，审批时不得由客户端覆盖。
    pub async fn create_copilot_action(
        &self,
        action: &CopilotAction,
    ) -> Result<CopilotAction, sqlx::Error> {
        sqlx::query_as::<_, CopilotAction>(
            "INSERT INTO copilot_actions \
             (id, session_id, operator, requested_role, tool_name, tool_arguments, risk_level) \
             VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING *",
        )
        .bind(&action.id)
        .bind(&action.session_id)
        .bind(&action.operator)
        .bind(&action.requested_role)
        .bind(&action.tool_name)
        .bind(&action.tool_arguments)
        .bind(&action.risk_level)
        .fetch_one(&self.pool)
        .await
    }

    /// 获取指定操作员会话中的动作，阻止跨会话、跨操作员访问。
    pub async fn get_copilot_action(
        &self,
        id: &str,
        session_id: &str,
        operator: &str,
    ) -> Result<Option<CopilotAction>, sqlx::Error> {
        sqlx::query_as::<_, CopilotAction>(
            "SELECT * FROM copilot_actions WHERE id = $1 AND session_id = $2 AND operator = $3",
        )
        .bind(id)
        .bind(session_id)
        .bind(operator)
        .fetch_optional(&self.pool)
        .await
    }

    /// 列出会话动作，供界面恢复审批卡片和查看审计结果。
    pub async fn list_copilot_actions(
        &self,
        session_id: &str,
        operator: &str,
    ) -> Result<Vec<CopilotAction>, sqlx::Error> {
        sqlx::query_as::<_, CopilotAction>(
            "SELECT * FROM copilot_actions WHERE session_id = $1 AND operator = $2 \
             ORDER BY created_at DESC LIMIT 200",
        )
        .bind(session_id)
        .bind(operator)
        .fetch_all(&self.pool)
        .await
    }

    /// 原子领取待审批动作；只有一个并发审批请求能够进入执行阶段。
    pub async fn claim_copilot_action(
        &self,
        id: &str,
        session_id: &str,
        operator: &str,
        reviewer_role: &str,
    ) -> Result<Option<CopilotAction>, sqlx::Error> {
        sqlx::query_as::<_, CopilotAction>(
            "UPDATE copilot_actions SET status = 'executing', reviewed_by = $3, \
             reviewed_role = $4, reviewed_at = now() \
             WHERE id = $1 AND session_id = $2 AND operator = $3 AND status = 'pending' \
             RETURNING *",
        )
        .bind(id)
        .bind(session_id)
        .bind(operator)
        .bind(reviewer_role)
        .fetch_optional(&self.pool)
        .await
    }

    /// 完成已领取动作，并记录完整工具结果用于审计。
    pub async fn complete_copilot_action(
        &self,
        id: &str,
        succeeded: bool,
        result: &Value,
    ) -> Result<Option<CopilotAction>, sqlx::Error> {
        let status = if succeeded { "approved" } else { "failed" };
        sqlx::query_as::<_, CopilotAction>(
            "UPDATE copilot_actions SET status = $2, result = $3, completed_at = now() \
             WHERE id = $1 AND status = 'executing' RETURNING *",
        )
        .bind(id)
        .bind(status)
        .bind(result)
        .fetch_optional(&self.pool)
        .await
    }

    /// 拒绝仍处于待审批状态的动作并记录原因。
    pub async fn reject_copilot_action(
        &self,
        id: &str,
        session_id: &str,
        operator: &str,
        reviewer_role: &str,
        note: Option<&str>,
    ) -> Result<Option<CopilotAction>, sqlx::Error> {
        sqlx::query_as::<_, CopilotAction>(
            "UPDATE copilot_actions SET status = 'rejected', reviewed_by = $3, \
             reviewed_role = $4, review_note = $5, reviewed_at = now(), completed_at = now() \
             WHERE id = $1 AND session_id = $2 AND operator = $3 AND status = 'pending' \
             RETURNING *",
        )
        .bind(id)
        .bind(session_id)
        .bind(operator)
        .bind(reviewer_role)
        .bind(note)
        .fetch_optional(&self.pool)
        .await
    }
}
