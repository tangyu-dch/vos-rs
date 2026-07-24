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
    Json(chat_req): Json<ChatStreamRequest>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let query = chat_req.query.trim().to_string();
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
    let active_llm = match crate::llm_configs::get_llm_config_from_redis(&state, chat_req.model_id).await {
        Some(rec) => Some(crate::copilot::LlmConfig::from(rec)),
        None => None,
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
    let images_clone = chat_req.images.clone();

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
            images: images_clone,
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
    images: Option<Vec<String>>,
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
        images,
    } = ctx;
    // 发送 user_message 事件
    send_event(&tx, "user_message", &user_message).await?;

    // 发送 context 事件（intent / LLM 状态，不依赖 LLM）
    send_event(&tx, "context", &context).await?;

    // 流式 LLM 调用或 fallback
    let (full_report, root_cause, suggested_action, final_llm_status) = if context.llm_enabled {
        match stream_llm_response(&tx, state, &active_llm, session_id, query, payload, ascii_ladder, &images).await {
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
            intent: Some(context.intent.as_str()),
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
    state: &AppState,
    active_llm: &Option<LlmConfig>,
    session_id: &str,
    query: &str,
    payload: &Payload,
    ascii_ladder: &str,
    images: &Option<Vec<String>>,
) -> Result<String, String> {
    let llm = active_llm
        .as_ref()
        .ok_or_else(|| "LLM 未配置".to_string())?;
    let url = format!("{}/chat/completions", llm.base_url.trim_end_matches('/'));
    let context_json = serde_json::to_string_pretty(payload).map_err(|e| e.to_string())?;

    // 构建用户消息：业务数据 JSON + 可选的 SIP 梯形图 ASCII
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

    // 获取并组装上下文历史对话（拉取最近 11 条，包含刚保存的最新 user 消息）
    let history = state
        .store
        .list_copilot_messages(session_id, 11)
        .await
        .unwrap_or_default();

    let mut messages = vec![
        serde_json::json!({
            "role": "system",
            "content": "你是 vos-rs 电信级 VoIP 软交换平台的智能运维专家 Copilot。你的任务是基于我提供的真实业务数据（JSON）和信令数据，协助用户进行高效的运维排障、性能分析或系统管理。\n\n回答要求：\n1. **排版规范与美观**：使用清晰的 Markdown 结构。必须包含以下二级标题：\n   - ## 📊 分析报告 (Analysis Report)：结合数据对当前系统状态或呼叫流程进行专业、生动的解读，避免冰冷的格式化叙述。\n   - ## 🔍 根因分析 (Root Cause)：深入剖析导致问题的底层原因（如网络延迟、信令超时、鉴权失败等），若无异常则明确告知。\n   - ## 💡 建议动作 (Suggested Action)：给出具体、可执行的操作指引（如修改路由规则、更新分机配置、核对运营商中继配置等）。\n2. **生动自然**：语气要专业、自然，像一个资深的 VoIP 架构师在与同事交流，不要让人觉得机械呆板。\n3. **梯形图输出**：如果上下文中包含 SIP 信令交互梯形图且用户问题与之相关，请在回答中合适的位置以 ```text 代码块原样输出该梯形图。\n4. **数据校验**：如果提供的业务数据为空，请礼貌地予以说明，并提示用户如何开启相应模块的持久化。"
        })
    ];

    // 排除最后一条（那是当前最新的消息，我们需要它带上当前最新的 telemetry payload 数据进行分析）
    for msg in history.iter().take(history.len().saturating_sub(1)) {
        messages.push(serde_json::json!({
            "role": if msg.role == "user" { "user" } else { "assistant" },
            "content": msg.content
        }));
    }

    let user_message_value = if let Some(ref img_list) = images {
        if !img_list.is_empty() {
            let mut parts = vec![serde_json::json!({
                "type": "text",
                "text": user_content
            })];
            for img_url in img_list {
                parts.push(serde_json::json!({
                    "type": "image_url",
                    "image_url": { "url": img_url }
                }));
            }
            serde_json::json!({
                "role": "user",
                "content": parts
            })
        } else {
            serde_json::json!({
                "role": "user",
                "content": user_content
            })
        }
    } else {
        serde_json::json!({
            "role": "user",
            "content": user_content
        })
    };

    messages.push(user_message_value);

    let body = serde_json::json!({
        "model": llm.model,
        "temperature": llm.temperature,
        "stream": true,
        "tools": crate::copilot::get_copilot_tools_schema(),
        "messages": messages
    });

    let resp = state.llm_client
        .post(&url)
        .header("Authorization", format!("Bearer {}", llm.api_key))
        .header("Content-Type", "application/json")
        .header("Accept-Encoding", "identity")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("HTTP 请求失败 (无法连接目标域名 {}, 请检查网络/代理/APIKey): {e}", llm.base_url))?;

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
        let chunk = match chunk_result {
            Ok(c) => c,
            Err(e) => {
                if !full_text.is_empty() {
                    tracing::warn!("LLM 流读取中途断开，但已成功接收部分分析内容: {e}");
                    return Ok(full_text);
                }
                return Err(format!("读取 LLM 流失败: {e}"));
            }
        };
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
