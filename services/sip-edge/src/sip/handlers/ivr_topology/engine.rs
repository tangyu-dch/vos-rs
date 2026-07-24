//! IVR 拓扑图遍历执行引擎
//!
//! 按 nodes/edges 拓扑图遍历节点，分发到 [`super::executors::dispatch`] 执行。
//!
//! 入口为 [`execute`]，接收 `&EdgeState` / `&EdgeConfig` 引用，
//! 由调用方在独立 task 中持有 `Arc` 并解引用后传入，避免引擎自身持有句柄。

use super::types::*;
use crate::{EdgeConfig, EdgeState};
use sip_core::SipRequest;
use std::net::SocketAddr;
use tracing::{error, info, warn};

/// 防止拓扑环路导致无限循环的最大步数。
const MAX_STEPS: u32 = 1000;

/// 执行 IVR 拓扑图 (每通话独立 task 调用)。
///
/// `edge_state` / `edge_config` 为引用，调用方需确保其在整个 future 生命周期内有效
/// (典型用法：在 `tokio::spawn` 的 async 块中 move `Arc` 并以引用传入)。
pub async fn execute(
    edge_state: &EdgeState,
    edge_config: &EdgeConfig,
    topology: &IvrTopology,
    context: &mut IvrExecutionContext,
    a_port: u16,
    caller_peer: SocketAddr,
    template_request: &SipRequest,
) {
    let graph = TopologyGraph::build(topology);
    let start_id = match &graph.start_node_id {
        Some(id) => id.clone(),
        None => {
            error!(call_id = %context.call_id, "IVR 拓扑无 start 节点, 终止执行");
            hangup_call(edge_state, &context.call_id, "IVR No Start Node").await;
            return;
        }
    };

    context.current_node_id = Some(start_id.clone());
    let mut current_id = start_id;
    let mut steps = 0u32;

    while let Some(node) = graph.get_node(&current_id) {
        steps += 1;
        if steps > MAX_STEPS {
            warn!(call_id = %context.call_id, steps, "IVR 执行步数超限, 强制终止");
            hangup_call(edge_state, &context.call_id, "IVR Step Limit Exceeded").await;
            return;
        }

        info!(
            call_id = %context.call_id,
            node_id = %node.id,
            node_type = %node.node_type,
            "IVR 执行节点"
        );

        let result = execute_node(
            edge_state,
            edge_config,
            node,
            &graph,
            context,
            a_port,
            caller_peer,
            template_request,
        )
        .await;

        match result {
            NodeExecuteResult::Continue { port } => match graph.next_node(&current_id, &port) {
                Some(next) => {
                    context.current_node_id = Some(next.id.clone());
                    current_id = next.id.clone();
                }
                None => {
                    warn!(call_id = %context.call_id, port = %port, "IVR 端口无后续节点, 结束执行");
                    break;
                }
            },
            NodeExecuteResult::Hangup { reason } => {
                hangup_call(edge_state, &context.call_id, &reason).await;
                return;
            }
            NodeExecuteResult::Transfer {
                target,
                transfer_type,
            } => {
                execute_transfer(
                    edge_state,
                    edge_config,
                    &context.call_id,
                    &target,
                    &transfer_type,
                    template_request,
                    a_port,
                )
                .await;
                return; // 转接后 IVR 结束
            }
            NodeExecuteResult::WaitForDtmf {
                max_digits,
                timeout_secs,
                terminator,
            } => {
                let dtmf =
                    wait_for_dtmf(edge_state, &context.call_id, a_port, max_digits, timeout_secs, terminator)
                        .await;
                context.collected_dtmf = dtmf.clone();
                let port = format!("key_{dtmf}");
                match graph.next_node(&current_id, &port) {
                    Some(next) => {
                        context.current_node_id = Some(next.id.clone());
                        current_id = next.id.clone();
                    }
                    None => match graph.next_node(&current_id, "default") {
                        Some(next) => {
                            context.current_node_id = Some(next.id.clone());
                            current_id = next.id.clone();
                        }
                        None => {
                            warn!(call_id = %context.call_id, dtmf = %dtmf, "IVR 菜单按键无匹配出口");
                            break;
                        }
                    },
                }
            }
            NodeExecuteResult::WaitForAsr { timeout_secs } => {
                let text = wait_for_asr(edge_state, &context.call_id, a_port, timeout_secs).await;
                context.set_var("asr_result", serde_json::Value::String(text));
                match graph.next_node(&current_id, "default") {
                    Some(next) => {
                        context.current_node_id = Some(next.id.clone());
                        current_id = next.id.clone();
                    }
                    None => break,
                }
            }
            NodeExecuteResult::Error { message } => {
                warn!(call_id = %context.call_id, %message, "IVR 节点执行错误");
                match graph.next_node(&current_id, "error") {
                    Some(next) => {
                        context.current_node_id = Some(next.id.clone());
                        current_id = next.id.clone();
                    }
                    None => {
                        hangup_call(edge_state, &context.call_id, "IVR Node Error").await;
                        return;
                    }
                }
            }
        }
    }

    info!(call_id = %context.call_id, steps, "IVR 拓扑执行完成");
}

/// 分发到具体节点执行器
async fn execute_node(
    edge_state: &EdgeState,
    edge_config: &EdgeConfig,
    node: &TopologyNode,
    graph: &TopologyGraph,
    context: &mut IvrExecutionContext,
    a_port: u16,
    caller_peer: SocketAddr,
    template_request: &SipRequest,
) -> NodeExecuteResult {
    match IvrNodeType::from_str(&node.node_type) {
        Some(nt) => {
            super::executors::dispatch(
                nt,
                node,
                graph,
                context,
                a_port,
                caller_peer,
                template_request,
                edge_state,
                edge_config,
            )
            .await
        }
        None => NodeExecuteResult::Error {
            message: format!("未知节点类型: {}", node.node_type),
        },
    }
}

/// 挂断指定呼叫
async fn hangup_call(edge_state: &EdgeState, call_id: &str, reason: &str) {
    edge_state
        .call_manager
        .terminate_call_with_reason(call_id, reason);
}

/// 执行转接 (复用现有 ivr.rs 转接逻辑)
async fn execute_transfer(
    edge_state: &EdgeState,
    edge_config: &EdgeConfig,
    call_id: &str,
    target: &str,
    transfer_type: &str,
    template_request: &SipRequest,
    a_port: u16,
) {
    let action = crate::edge_state::IvrAction {
        action_type: transfer_type.to_string(),
        action_target: target.to_string(),
        waiting_prompt: None,
        webhook_method: None,
    };
    super::super::ivr::execute_ivr_action_for_topology(
        edge_state,
        edge_config,
        call_id,
        a_port,
        &action,
        template_request,
    )
    .await;
}

/// 等待 DTMF 输入 (stub: 后续阶段接入媒体层 DTMF 检测)
#[allow(unused_variables)]
async fn wait_for_dtmf(
    edge_state: &EdgeState,
    call_id: &str,
    a_port: u16,
    max_digits: u8,
    timeout_secs: u32,
    terminator: Option<char>,
) -> String {
    // TODO: 接入媒体层的 DTMF 检测
    let _ = edge_state;
    String::new()
}

/// 等待 ASR 输入
///
/// 接入 [`super::voice_engine::AsrEngine`]：
/// - 若 ASR 引擎未启用 (`VOS_RS_IVR_ASR_ENABLED` 未设置或模型路径缺失)，直接返回空文本
/// - 若 ASR 引擎已启用，等待媒体层收集 PCM 音频后调用 `recognize` 识别
///
/// 当前实现：媒体层音频收集 API 尚未对接，临时返回空文本并记录 warning，
/// 后续阶段接入 RTP 监听/缓冲后即可通过 `asr_engine.recognize(&samples, sample_rate)` 完成识别。
#[allow(unused_variables)]
async fn wait_for_asr(
    edge_state: &EdgeState,
    call_id: &str,
    a_port: u16,
    timeout_secs: u32,
) -> String {
    let Some(voice_mgr) = edge_state.voice_engine() else {
        warn!(call_id, "voice_engine 未注入, ASR 返回空文本");
        return String::new();
    };
    let Some(asr_engine) = voice_mgr.asr.as_ref() else {
        warn!(
            call_id,
            "ASR 引擎未启用 (VOS_RS_IVR_ASR_ENABLED), 返回空文本"
        );
        return String::new();
    };

    // TODO: 接入媒体层收集 a_port 上的 PCM i16 samples
    // 当前媒体层尚未暴露按端口收集 PCM 的 API, 暂用空 samples 调用一次以触发模型惰性加载,
    // 并在日志中提示后续接入点。
    warn!(
        call_id,
        a_port, timeout_secs, "ASR 音频收集尚未接入媒体层, 返回空文本"
    );
    let empty_samples: Vec<i16> = Vec::new();
    match asr_engine.recognize(&empty_samples, 16000).await {
        Ok(text) => text,
        Err(e) => {
            warn!(call_id, error = %e, "ASR 识别失败, 返回空文本");
            String::new()
        }
    }
}
