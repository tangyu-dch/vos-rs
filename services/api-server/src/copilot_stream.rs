//! # Copilot 流式对话（SSE）
//!
//! 提供与 `copilot_history::chat_in_session` 相同的业务逻辑（落库用户消息 →
//! 采集真实业务数据 → LLM 分析 → 落库 assistant 回答），但改为 Server-Sent
//! Events 流式输出，让前端逐 token 渲染 LLM 回答，类似豆包的打字机效果。
//!
//! SSE 事件协议：
//! - `event: user_message`  data: CopilotMessage       — 用户消息已落库
//! - `event: context`       data: ContextPayload        — 梯形图等结构化上下文
//! - `event: delta`         data: {"text":"chunk"}      — LLM 逐 chunk 输出
//! - `event: done`          data: DonePayload           — 完成信号 + 最终元数据
//! - `event: error`         data: {"error":"msg"}       — 错误（流不中断，后续发 done）

use axum::{
    extract::{Extension, Path, State},
    response::sse::{Event, KeepAlive, Sse},
    Json,
};
use futures::stream::{self, Stream};
use futures::StreamExt;
use serde::Serialize;
use std::convert::Infallible;
use tokio::sync::mpsc;

use cdr_core::{AppendCopilotMessageInput, CopilotMessage, CopilotSession};

use crate::{
    auth::Claims,
    copilot::{CopilotIntent, LlmConfig, Payload, TelecomCopilotEngine},
    copilot_history::{derive_title, option_str, ChatStreamRequest},
    ApiError, AppState,
};

/// SSE 通道缓冲区大小：LLM chunk 通常很小且频繁，64 足够平滑
const SSE_CHANNEL_BUFFER: usize = 64;

/// 结构化上下文（梯形图等），在 LLM 输出前先发给前端
#[derive(Debug, Serialize)]
struct ContextPayload {
    intent: String,
    llm_enabled: bool,
    llm_status: String,
}

/// 完成信号：包含最新会话元数据 + 落库的 assistant 消息
#[derive(Debug, Serialize)]
struct DonePayload {
    session: CopilotSession,
    assistant_message: CopilotMessage,
}

/// 流式 chat handler：SSE 逐 token 推送 LLM 回答。
///
/// 流程：校验会话 → 落库用户消息 → 采集业务数据 → 流式 LLM → 落库 assistant → done。
pub async fn chat_in_session_stream(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(session_id): Path<String>,
    Json(payload): Json<ChatStreamRequest>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let query = payload.query.trim().to_string();
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
            content: &query,
            root_cause: None,
            suggested_action: None,
            ladder_diagram_ascii: None,
            llm_enabled: None,
            llm_status: None,
            intent: None,
        })
        .await
        .map_err(|e| ApiError::internal(format!("保存用户消息失败: {e}")))?;

    // 3) 采集业务数据 + 生成梯形图（不依赖 LLM，可先完成）
    // 运行时从数据库读取当前启用的 LLM 配置（is_active=true）
    let active_llm = match state.store.get_active_llm_config().await {
        Ok(Some(rec)) => Some(crate::copilot::LlmConfig::from(rec)),
        Ok(None) => None,
        Err(e) => {
            tracing::warn!(error = %e, "读取当前 LLM 配置失败，回退到无 LLM 模式");
            None
        }
    };
    let engine = TelecomCopilotEngine::new(&state, active_llm.clone());
    let intent = TelecomCopilotEngine::classify_intent(&query);
    let payload_data = engine.collect_payload(&query, intent).await;
    let steps = TelecomCopilotEngine::build_ladder_steps(&payload_data);
    let ascii_ladder = TelecomCopilotEngine::generate_ascii_ladder(&steps);
    let llm_enabled = active_llm.as_ref().is_some_and(|l| l.is_configured());
    let llm_status = if llm_enabled {
        let l = active_llm.as_ref().expect("llm_enabled 为 true 时 active_llm 必存在");
        format!("LLM 已启用 (provider={}, model={})", l.provider, l.model)
    } else {
        "LLM 未配置（数据库无启用配置），以下为结构化真实业务数据".to_string()
    };

    // 4) 创建 SSE channel，spawn 任务流式推送
    let (tx, rx) = mpsc::channel::<Event>(SSE_CHANNEL_BUFFER);
    let context = ContextPayload {
        intent: intent.as_str().to_string(),
        llm_enabled,
        llm_status: llm_status.clone(),
    };

    let state_clone = state.clone();
    let session_clone = session.clone();
    let claims_sub = claims.sub.clone();
    let session_id_clone = session_id.clone();
    let query_clone = query.clone();
    let ascii_ladder_clone = ascii_ladder.clone();
    let active_llm_clone = active_llm.clone();
    tokio::spawn(async move {
        let ctx = StreamContext {
            state: &state_clone,
            session_id: &session_id_clone,
            operator: &claims_sub,
            session: &session_clone,
            user_message,
            context,
            query: &query_clone,
            intent,
            payload: &payload_data,
            ascii_ladder: &ascii_ladder_clone,
            active_llm: active_llm_clone,
        };
        let result = run_stream_loop(tx, ctx).await;
        if let Err(e) = result {
            tracing::warn!(error = %e, "Copilot 流式推送异常");
        }
    });

    // 5) 返回 SSE stream（把 mpsc::Receiver 转为 Stream）
    let stream = stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|event| (Ok(event), rx))
    });
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

/// 流式推送所需的上下文（避免函数参数过多）
struct StreamContext<'a> {
    state: &'a AppState,
    session_id: &'a str,
    operator: &'a str,
    session: &'a CopilotSession,
    user_message: CopilotMessage,
    context: ContextPayload,
    query: &'a str,
    intent: CopilotIntent,
    payload: &'a Payload,
    ascii_ladder: &'a str,
    active_llm: Option<LlmConfig>,
}

/// 流式推送主循环：发送 user_message → context → delta* → done
async fn run_stream_loop(
    tx: mpsc::Sender<Event>,
    ctx: StreamContext<'_>,
) -> Result<(), String> {
    let StreamContext {
        state,
        session_id,
        operator,
        session,
        user_message,
        context,
        query,
        intent,
        payload,
        ascii_ladder,
        active_llm,
    } = ctx;
    // 发送 user_message 事件
    send_event(&tx, "user_message", &user_message).await?;

    // 发送 context 事件（intent / LLM 状态，不依赖 LLM）
    send_event(&tx, "context", &context).await?;

    // 流式 LLM 调用或 fallback
    let (full_report, root_cause, suggested_action, final_llm_status) = if context.llm_enabled {
        match stream_llm_response(&tx, &state.llm_client, &active_llm, query, payload, ascii_ladder).await {
            Ok(text) => (text, String::new(), String::new(), context.llm_status.clone()),
            Err(e) => {
                tracing::warn!(error = %e, "LLM 流式调用失败，回退到结构化报告");
                let (r, rc, sa) =
                    TelecomCopilotEngine::build_fallback_report(query, intent, payload);
                let fallback_status = format!("LLM 调用失败：{e}；以下为结构化真实业务数据");
                send_event(&tx, "delta", &serde_json::json!({ "text": &r })).await?;
                (r, rc, sa, fallback_status)
            }
        }
    } else {
        let (r, rc, sa) = TelecomCopilotEngine::build_fallback_report(query, intent, payload);
        send_event(&tx, "delta", &serde_json::json!({ "text": &r })).await?;
        (r, rc, sa, context.llm_status.clone())
    };

    // 落库 assistant 消息（梯形图已内嵌到 markdown content，不再单独存储）
    let assistant_message = state
        .store
        .append_copilot_message(AppendCopilotMessageInput {
            session_id,
            role: "assistant",
            content: &full_report,
            root_cause: option_str(&root_cause),
            suggested_action: option_str(&suggested_action),
            ladder_diagram_ascii: None,
            llm_enabled: Some(context.llm_enabled),
            llm_status: option_str(&final_llm_status),
            intent: Some(intent.as_str()),
        })
        .await
        .map_err(|e| format!("保存 assistant 回答失败: {e}"))?;

    // 首次发消息自动重命名标题
    let mut final_session = session.clone();
    if final_session.title == "新对话" {
        let new_title = derive_title(query);
        if let Some(updated) = state
            .store
            .update_copilot_session(session_id, operator, Some(&new_title), None, None)
            .await
            .map_err(|e| format!("更新会话标题失败: {e}"))?
        {
            final_session = updated;
        }
    } else if let Some(updated) = state
        .store
        .get_copilot_session(session_id, operator)
        .await
        .map_err(|e| format!("刷新会话状态失败: {e}"))?
    {
        final_session = updated;
    }

    // 发送 done 事件
    let done = DonePayload {
        session: final_session,
        assistant_message,
    };
    send_event(&tx, "done", &done).await?;

    Ok(())
}

/// 流式调用 LLM（OpenAI 兼容协议 stream=true），逐 chunk 通过 SSE delta 推送。
///
/// 返回完整文本。如果 HTTP 请求本身失败或响应解析失败，返回 Err。
async fn stream_llm_response(
    tx: &mpsc::Sender<Event>,
    client: &reqwest::Client,
    active_llm: &Option<LlmConfig>,
    query: &str,
    payload: &Payload,
    ascii_ladder: &str,
) -> Result<String, String> {
    let llm = active_llm
        .as_ref()
        .ok_or_else(|| "LLM 未配置".to_string())?;
    let url = format!("{}/chat/completions", llm.base_url.trim_end_matches('/'));
    let context_json = serde_json::to_string_pretty(payload).map_err(|e| e.to_string())?;

    // 构建用户消息：业务数据 JSON + 可选的 SIP 梯形图 ASCII
    // LLM 自行判断是否在回答中包含梯形图（用 ```text 代码块包裹）
    let ladder_section = if ascii_ladder.trim().is_empty() {
        String::new()
    } else {
        format!(
            "\n\n## SIP 信令交互梯形图（参考数据，如用户问及请在回答中用 ```text 代码块原样输出）\n```text\n{ascii_ladder}\n```"
        )
    };
    let user_content = format!(
        "用户问题：{query}\n\n当前真实业务数据（JSON）：\n{context_json}{ladder_section}"
    );

    let body = serde_json::json!({
        "model": llm.model,
        "temperature": llm.temperature,
        "stream": true,
        "messages": [
            {
                "role": "system",
                "content": "你是 vos-rs 电信级 VoIP 软交换平台的运维助手 Copilot。基于真实业务数据回答用户的运维排障问题。回答需简洁、专业、可执行，包含：1) 分析报告；2) 根因定位；3) 建议动作。若数据为空请明确告知。如果上下文中包含 SIP 信令交互梯形图且用户问题与之相关（如排查信令、分析呼叫流程），请在回答中用 ```text 代码块原样输出梯形图。"
            },
            {
                "role": "user",
                "content": user_content
            }
        ]
    });

    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", llm.api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("HTTP 请求失败: {e}"))?;

    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("LLM HTTP {status}: {}", truncate(&text, 300)));
    }

    // 逐 chunk 解析 SSE：`data: {json}\n\n`，以 `data: [DONE]` 结束
    let mut byte_stream = resp.bytes_stream();
    let mut buf = String::new();
    let mut full_text = String::new();

    while let Some(chunk_result) = byte_stream.next().await {
        let chunk = chunk_result.map_err(|e| format!("读取 LLM 流失败: {e}"))?;
        buf.push_str(&String::from_utf8_lossy(&chunk));

        // 按换行符分割，处理完整的行
        while let Some(newline_pos) = buf.find('\n') {
            let line = buf[..newline_pos].trim().to_string();
            buf = buf[newline_pos + 1..].to_string();

            if line.is_empty() || line.starts_with(':') {
                continue;
            }
            if let Some(json_str) = line.strip_prefix("data: ") {
                if json_str.trim() == "[DONE]" {
                    return Ok(full_text);
                }
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(json_str) {
                    if let Some(content) = val
                        .get("choices")
                        .and_then(|c| c.get(0))
                        .and_then(|c| c.get("delta"))
                        .and_then(|d| d.get("content"))
                        .and_then(|c| c.as_str())
                    {
                        if !content.is_empty() {
                            full_text.push_str(content);
                            send_event(tx, "delta", &serde_json::json!({ "text": content }))
                                .await?;
                        }
                    }
                }
            }
        }
    }

    Ok(full_text)
}

/// 发送一个 SSE 事件
async fn send_event(
    tx: &mpsc::Sender<Event>,
    event_name: &str,
    data: &impl Serialize,
) -> Result<(), String> {
    let json = serde_json::to_string(data).map_err(|e| e.to_string())?;
    let event = Event::default().event(event_name).data(json);
    tx.send(event)
        .await
        .map_err(|_| "SSE channel closed".to_string())?;
    Ok(())
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max])
    }
}
