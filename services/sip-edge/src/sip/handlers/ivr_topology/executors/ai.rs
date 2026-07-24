//! AI Agent 节点执行器
//!
//! 通过 HTTP 调用 api-server 的 LLM 推理接口 (`/api/v1/copilot/chat`)，
//! 复用 llm_configs 表中维护的多厂商 LLM 配置。
//!
//! 流程：读取 `prompt` / `system_prompt` -> 模板渲染 -> POST 到 api-server ->
//! 解析响应 -> 把回复文本写入 `context.variables["ai_reply"]` -> 走 default 端口。

use super::super::types::*;
use crate::EdgeConfig;
use tracing::{info, warn};

/// 默认 AI Agent 最大对话轮数 (当前实现为单轮调用, 仅用于日志)
const DEFAULT_AI_MAX_TURNS: u32 = 5;
/// 默认 LLM 调用超时秒数
const DEFAULT_LLM_TIMEOUT_SECS: u64 = 30;

/// 执行 ai_agent 节点：与 LLM 进行对话
///
/// 读取配置：
/// - `prompt`：用户提示词（必填，支持 `{{var}}` 模板渲染）
/// - `system_prompt`：系统提示词（可选，默认 "你是一个电话客服助手"）
/// - `max_turns`：最大对话轮数（默认 5，仅用于日志）
/// - `model`：LLM 模型标识（可选，记录到日志，实际由 api-server 决定）
/// - `timeout_secs`：HTTP 调用超时秒数（默认 30）
///
/// 调用成功后将回复写入 `context.variables["ai_reply"]`，并走 default 端口继续。
pub async fn execute_ai_agent(
    node: &TopologyNode,
    context: &mut IvrExecutionContext,
    _edge_config: &EdgeConfig,
) -> NodeExecuteResult {
    let user_prompt = match node.config.get("prompt").and_then(|v| v.as_str()) {
        Some(s) if !s.is_empty() => context.render_template(s),
        _ => {
            return NodeExecuteResult::Error {
                message: "ai_agent 节点未配置 prompt".to_string(),
            };
        }
    };
    let system_prompt = node
        .config
        .get("system_prompt")
        .and_then(|v| v.as_str())
        .unwrap_or("你是一个电话客服助手")
        .to_string();
    let max_turns = node
        .config
        .get("max_turns")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32)
        .unwrap_or(DEFAULT_AI_MAX_TURNS);
    let model = node
        .config
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("default");
    let timeout_secs = node
        .config
        .get("timeout_secs")
        .and_then(|v| v.as_u64())
        .unwrap_or(DEFAULT_LLM_TIMEOUT_SECS);

    let api_server_url = std::env::var("VOS_RS_API_SERVER_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:8080".to_string());
    let llm_endpoint = format!("{api_server_url}/api/v1/copilot/chat");

    info!(
        call_id = %context.call_id,
        node_id = %node.id,
        endpoint = %llm_endpoint,
        model,
        max_turns,
        prompt_len = user_prompt.len(),
        "IVR ai_agent 节点调用 LLM"
    );

    // 复用 api-server copilot chat 接口的请求体格式
    let request_body = serde_json::json!({
        "message": user_prompt,
        "system_prompt": system_prompt,
        "stream": false,
    });

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            warn!(
                call_id = %context.call_id,
                node_id = %node.id,
                error = %e,
                "ai_agent 构建 HTTP client 失败"
            );
            return NodeExecuteResult::Error {
                message: format!("LLM client 构建失败: {e}"),
            };
        }
    };

    match client.post(&llm_endpoint).json(&request_body).send().await {
        Ok(response) => {
            if !response.status().is_success() {
                let status = response.status();
                warn!(
                    call_id = %context.call_id,
                    node_id = %node.id,
                    %status,
                    "ai_agent LLM 调用失败"
                );
                return NodeExecuteResult::Error {
                    message: format!("LLM 调用失败: {status}"),
                };
            }
            match response.json::<serde_json::Value>().await {
                Ok(json) => {
                    // 兼容多种常见响应字段名
                    let reply = json
                        .get("reply")
                        .or_else(|| json.get("message"))
                        .or_else(|| json.get("content"))
                        .or_else(|| json.get("analysis_report"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    info!(
                        call_id = %context.call_id,
                        node_id = %node.id,
                        reply_len = reply.len(),
                        "ai_agent 收到 LLM 回复"
                    );
                    context.set_var("ai_reply", serde_json::Value::String(reply));
                    NodeExecuteResult::Continue {
                        port: "default".to_string(),
                    }
                }
                Err(e) => {
                    warn!(
                        call_id = %context.call_id,
                        node_id = %node.id,
                        error = %e,
                        "ai_agent 解析 LLM 响应失败"
                    );
                    NodeExecuteResult::Error {
                        message: format!("LLM 响应解析失败: {e}"),
                    }
                }
            }
        }
        Err(e) => {
            warn!(
                call_id = %context.call_id,
                node_id = %node.id,
                error = %e,
                "ai_agent LLM 请求失败"
            );
            NodeExecuteResult::Error {
                message: format!("LLM 请求失败: {e}"),
            }
        }
    }
}
