//! # Copilot 历史会话 REST API
//!
//! 提供 Copilot 多轮对话的会话管理：创建 / 列表 / 详情 / 更新（标题、置顶、归档）
//! / 删除；以及会话内发消息（沿用 `TelecomCopilotEngine.analyze` 真实业务数据 + LLM
//! 调用能力，并把 user / assistant 消息持久化到 PostgreSQL）。
//!
//! 所有写操作都按 JWT 中的 `sub`（操作员）做行级隔离，避免越权读写他人会话。

use axum::{
    extract::{Extension, Path, State},
    Json,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use cdr_core::{AppendCopilotMessageInput, CopilotMessage, CopilotSession};

use crate::{
    auth::Claims,
    copilot::{CopilotChatResponse, TelecomCopilotEngine},
    ApiError, AppState,
};

/// 默认会话标题：取首条用户问题的前 30 字符 + 省略号
const DEFAULT_TITLE_MAX_LEN: usize = 30;

// ============ 请求 / 响应 DTO ============

#[derive(Debug, Deserialize, Default)]
pub struct CreateSessionRequest {
    /// 可选标题；不提供则用首条消息内容生成
    pub title: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ChatInSessionRequest {
    pub query: String,
    pub model_id: Option<i64>,
}

/// 流式 chat 请求体（与 ChatInSessionRequest 相同，单独定义避免歧义）
#[derive(Debug, Deserialize)]
pub struct ChatStreamRequest {
    pub query: String,
    pub model_id: Option<i64>,
    pub images: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Default)]
pub struct UpdateSessionRequest {
    pub title: Option<String>,
    pub pinned: Option<bool>,
    pub archived: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct SessionListResponse {
    pub sessions: Vec<CopilotSession>,
}

#[derive(Debug, Serialize)]
pub struct SessionDetailResponse {
    pub session: CopilotSession,
    pub messages: Vec<CopilotMessage>,
}

/// 会话内发消息响应：返回当前 assistant 回答 + 最新会话元数据
#[derive(Debug, Serialize)]
pub struct ChatInSessionResponse {
    pub session: CopilotSession,
    pub user_message: CopilotMessage,
    pub assistant_message: CopilotMessage,
    pub analysis: CopilotChatResponse,
}

// ============ Handlers ============

/// 创建新会话。`title` 缺省时使用占位标题，首次发消息后会自动重命名为首条问题摘要。
pub async fn create_session(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    payload: Option<Json<CreateSessionRequest>>,
) -> Result<Json<CopilotSession>, ApiError> {
    let payload = payload.map(|Json(p)| p).unwrap_or_default();
    let id = format!("cp-{}", Uuid::new_v4().simple());
    let title = payload
        .title
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| "新对话".to_string());
    // 运行时从数据库读取当前启用的 LLM 配置，用于会话元数据记录
    let active_llm_meta = match state.store.get_active_llm_config().await {
        Ok(Some(rec)) => (Some(rec.provider), Some(rec.model)),
        _ => (None, None),
    };
    let session = state
        .store
        .create_copilot_session(
            &id,
            &title,
            &claims.sub,
            active_llm_meta.0.as_deref(),
            active_llm_meta.1.as_deref(),
        )
        .await
        .map_err(|e| ApiError::internal(format!("创建 Copilot 会话失败: {e}")))?;
    Ok(Json(session))
}

/// 列出当前操作员的会话（默认排除归档）。
/// 可通过 `?include_archived=true` 包含归档会话，`?limit=N` 限制条数（上限 100）。
pub async fn list_sessions(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    axum::extract::Query(params): axum::extract::Query<ListParams>,
) -> Result<Json<SessionListResponse>, ApiError> {
    let limit = params.limit.unwrap_or(50);
    let include_archived = params.include_archived.unwrap_or(false);
    let sessions = state
        .store
        .list_copilot_sessions(&claims.sub, limit, include_archived)
        .await
        .map_err(|e| ApiError::internal(format!("查询 Copilot 会话列表失败: {e}")))?;
    Ok(Json(SessionListResponse { sessions }))
}

#[derive(Debug, Deserialize)]
pub struct ListParams {
    pub limit: Option<i64>,
    pub include_archived: Option<bool>,
}

/// 获取会话详情（含全部消息）。会话不存在或不属于当前操作员返回 404。
pub async fn get_session(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(session_id): Path<String>,
) -> Result<Json<SessionDetailResponse>, ApiError> {
    let session = state
        .store
        .get_copilot_session(&session_id, &claims.sub)
        .await
        .map_err(|e| ApiError::internal(format!("查询 Copilot 会话失败: {e}")))?
        .ok_or_else(|| ApiError::not_found("会话不存在".to_string()))?;
    let messages = state
        .store
        .list_copilot_messages(&session_id, 500)
        .await
        .map_err(|e| ApiError::internal(format!("查询 Copilot 消息失败: {e}")))?;
    Ok(Json(SessionDetailResponse { session, messages }))
}

/// 更新会话元数据（标题 / 置顶 / 归档）。
pub async fn update_session(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(session_id): Path<String>,
    payload: Option<Json<UpdateSessionRequest>>,
) -> Result<Json<CopilotSession>, ApiError> {
    let payload = payload.map(|Json(p)| p).unwrap_or_default();
    let trimmed_title = payload
        .title
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let session = state
        .store
        .update_copilot_session(
            &session_id,
            &claims.sub,
            trimmed_title.as_deref(),
            payload.pinned,
            payload.archived,
        )
        .await
        .map_err(|e| ApiError::internal(format!("更新 Copilot 会话失败: {e}")))?
        .ok_or_else(|| ApiError::not_found("会话不存在".to_string()))?;
    Ok(Json(session))
}

/// 删除会话（外键级联清理消息）。
pub async fn delete_session(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(session_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let deleted = state
        .store
        .delete_copilot_session(&session_id, &claims.sub)
        .await
        .map_err(|e| ApiError::internal(format!("删除 Copilot 会话失败: {e}")))?;
    if !deleted {
        return Err(ApiError::not_found("会话不存在".to_string()));
    }
    Ok(Json(serde_json::json!({ "deleted": true, "session_id": session_id })))
}

/// 在会话内发消息：落库用户消息 → 调用引擎分析 → 落库 assistant 回答 → 返回。
///
/// 首次发消息时若会话标题仍是占位"新对话"，自动用问题摘要更新标题，
/// 让会话列表更易识别。
pub async fn chat_in_session(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(session_id): Path<String>,
    Json(payload): Json<ChatInSessionRequest>,
) -> Result<Json<ChatInSessionResponse>, ApiError> {
    let query = payload.query.trim();
    if query.is_empty() {
        return Err(ApiError::bad_request("参数无效：查询不能为空".to_string()));
    }

    // 1) 校验会话归属
    let session = state
        .store
        .get_copilot_session(&session_id, &claims.sub)
        .await
        .map_err(|e| ApiError::internal(format!("查询 Copilot 会话失败: {e}")))?
        .ok_or_else(|| ApiError::not_found("会话不存在".to_string()))?;

    // 2) 落库用户消息
    let user_message = state
        .store
        .append_copilot_message(AppendCopilotMessageInput {
            session_id: &session_id,
            role: "user",
            content: query,
            images: None,
            root_cause: None,
            suggested_action: None,
            ladder_diagram_ascii: None,
            llm_enabled: None,
            llm_status: None,
            intent: None,
        })
        .await
        .map_err(|e| ApiError::internal(format!("保存用户消息失败: {e}")))?;

    // 3) 调用引擎分析（真实业务数据采集 + LLM 调用 + 回退结构化报告）
    // 运行时从 Redis 读取当前启用的 LLM 配置
    let active_llm = match crate::llm_configs::get_llm_config_from_redis(&state, payload.model_id).await {
        Some(rec) => Some(crate::copilot::LlmConfig::from(rec)),
        None => None,
    };
    let engine = TelecomCopilotEngine::new(&state, active_llm);
    
    // 获取当前会话上下文（包含刚插入的最新消息）
    let history = state
        .store
        .list_copilot_messages(&session_id, 11)
        .await
        .unwrap_or_default();
        
    let analysis = engine.analyze(query, Some(&history)).await;
    let intent_str = TelecomCopilotEngine::classify_intent(query).as_str();

    // 4) 落库 assistant 回答
    let assistant_message = state
        .store
        .append_copilot_message(AppendCopilotMessageInput {
            session_id: &session_id,
            role: "assistant",
            content: &analysis.analysis_report,
            images: None,
            root_cause: option_str(&analysis.root_cause),
            suggested_action: option_str(&analysis.suggested_action),
            ladder_diagram_ascii: option_str(&analysis.ladder_diagram_ascii),
            llm_enabled: Some(analysis.llm_enabled),
            llm_status: option_str(&analysis.llm_status),
            intent: Some(intent_str),
        })
        .await
        .map_err(|e| ApiError::internal(format!("保存 assistant 回答失败: {e}")))?;

    // 5) 首次发消息时自动重命名会话标题
    let mut final_session = session;
    if final_session.title == "新对话" {
        let new_title = derive_title(query);
        if let Some(updated) = state
            .store
            .update_copilot_session(&session_id, &claims.sub, Some(&new_title), None, None)
            .await
            .map_err(|e| ApiError::internal(format!("更新会话标题失败: {e}")))?
        {
            final_session = updated;
        }
    } else {
        // 已经有标题，但需要拿最新的 message_count / last_message_at
        if let Some(updated) = state
            .store
            .get_copilot_session(&session_id, &claims.sub)
            .await
            .map_err(|e| ApiError::internal(format!("刷新会话状态失败: {e}")))?
        {
            final_session = updated;
        }
    }

    Ok(Json(ChatInSessionResponse {
        session: final_session,
        user_message,
        assistant_message,
        analysis,
    }))
}

/// 由用户问题生成默认会话标题：截断到 30 字符 + 省略号
pub(crate) fn derive_title(query: &str) -> String {
    let trimmed = query.trim();
    if trimmed.chars().count() <= DEFAULT_TITLE_MAX_LEN {
        return trimmed.to_string();
    }
    let truncated: String = trimmed.chars().take(DEFAULT_TITLE_MAX_LEN).collect();
    format!("{truncated}…")
}

/// 把 `&String` 转换为 `Option<&str>`：空字符串视为 `None`，避免落库空值
pub(crate) fn option_str(s: &str) -> Option<&str> {
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_title_short_query_kept_as_is() {
        assert_eq!(derive_title("查通话状态"), "查通话状态");
    }

    #[test]
    fn derive_title_long_query_truncated_with_ellipsis() {
        let long = "排查 13800138000 为什么在上午十点十五分被挂断并产生 SIP 503 错误";
        let title = derive_title(long);
        assert!(title.ends_with('…'));
        assert_eq!(title.chars().count(), DEFAULT_TITLE_MAX_LEN + 1);
    }
}
