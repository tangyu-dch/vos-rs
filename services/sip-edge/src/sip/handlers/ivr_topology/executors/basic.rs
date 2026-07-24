//! 基础节点执行器：start / hangup / prompt / collect_dtmf / menu
//!
//! 这些节点不涉及外部 HTTP 调用与转接，仅与媒体层和上下文交互。

use super::super::types::*;
use crate::EdgeState;
use tracing::{info, warn};

/// 执行 start / hangup 节点
///
/// - start：读取 did 配置写入上下文，可选播放 welcome_prompt，走 default 端口
/// - hangup：读取 reason 配置，返回 Hangup 结果
pub async fn execute(node: &TopologyNode, context: &mut IvrExecutionContext) -> NodeExecuteResult {
    match node.node_type.as_str() {
        "start" => execute_start(node, context).await,
        "hangup" => execute_hangup(node),
        _ => NodeExecuteResult::Error {
            message: format!("basic::execute 不支持的节点类型: {}", node.node_type),
        },
    }
}

/// 执行 start 节点
async fn execute_start(
    node: &TopologyNode,
    context: &mut IvrExecutionContext,
) -> NodeExecuteResult {
    // 读取 did 配置（若存在则覆盖上下文中的 did）
    if let Some(did) = node.config.get("did").and_then(|v| v.as_str()) {
        if !did.is_empty() {
            context.did = did.to_string();
        }
    }
    // 读取初始变量（若 config.variables 是对象则逐项写入上下文）
    if let Some(vars) = node.config.get("variables").and_then(|v| v.as_object()) {
        for (k, v) in vars {
            context.set_var(k, v.clone());
        }
    }
    info!(
        call_id = %context.call_id,
        node_id = %node.id,
        did = %context.did,
        "IVR start 节点执行完成"
    );
    NodeExecuteResult::Continue {
        port: "default".to_string(),
    }
}

/// 执行 hangup 节点
fn execute_hangup(node: &TopologyNode) -> NodeExecuteResult {
    let reason = node
        .config
        .get("reason")
        .and_then(|v| v.as_str())
        .unwrap_or("IVR Hangup")
        .to_string();
    info!(node_id = %node.id, reason = %reason, "IVR hangup 节点");
    NodeExecuteResult::Hangup { reason }
}

/// 执行 prompt 节点：播放一段音频文件
///
/// 读取配置：
/// - `audio_file`：音频文件路径（必填）
/// - `loop`：是否循环播放（默认 false）
/// - `interruptible`：是否可被 DTMF 打断（默认 true，仅用于日志示意，实际打断由媒体层负责）
///
/// 复用 [`crate::media::relay::MediaRelayState::start_playback`] 接口，
/// 与 `ivr.rs` 中播放 welcome_prompt 的逻辑保持一致。
pub async fn execute_prompt(
    node: &TopologyNode,
    context: &IvrExecutionContext,
    a_port: u16,
    edge_state: &EdgeState,
) -> NodeExecuteResult {
    let Some(audio_file) = read_str_config(node, "audio_file") else {
        warn!(
            call_id = %context.call_id,
            node_id = %node.id,
            "prompt 节点未配置 audio_file, 跳过播放"
        );
        return NodeExecuteResult::Continue {
            port: "default".to_string(),
        };
    };
    let loop_playback = read_bool_config(node, "loop", false);
    let interruptible = read_bool_config(node, "interruptible", true);
    let rendered_path = context.render_template(&audio_file);

    info!(
        call_id = %context.call_id,
        node_id = %node.id,
        a_port,
        file = %rendered_path,
        loop_playback,
        interruptible,
        "IVR prompt 节点播放音频"
    );

    start_prompt_playback(
        edge_state,
        a_port,
        &rendered_path,
        loop_playback,
        context,
        node,
    )
    .await
}

/// 调用媒体层播放音频，返回对应的 NodeExecuteResult
async fn start_prompt_playback(
    edge_state: &EdgeState,
    a_port: u16,
    path: &str,
    loop_playback: bool,
    context: &IvrExecutionContext,
    node: &TopologyNode,
) -> NodeExecuteResult {
    let playback_mode = crate::media::relay::PlaybackMode::Exclusive;
    match edge_state
        .media_relay
        .start_playback(
            a_port,
            std::path::PathBuf::from(path),
            playback_mode,
            loop_playback,
        )
        .await
    {
        Ok(_) => NodeExecuteResult::Continue {
            port: "default".to_string(),
        },
        Err(e) => {
            warn!(
                call_id = %context.call_id,
                node_id = %node.id,
                error = %e,
                "prompt 节点播放音频失败"
            );
            NodeExecuteResult::Error {
                message: format!("播放音频失败: {e}"),
            }
        }
    }
}

/// 读取字符串型配置项，缺失/空串/类型不符时返回 None
fn read_str_config(node: &TopologyNode, key: &str) -> Option<String> {
    node.config
        .get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// 读取布尔型配置项，缺失或类型不符时返回默认值
fn read_bool_config(node: &TopologyNode, key: &str, default: bool) -> bool {
    node.config
        .get(key)
        .and_then(|v| v.as_bool())
        .unwrap_or(default)
}

/// 执行 collect_dtmf / menu 节点：等待用户 DTMF 输入
///
/// 读取配置：
/// - `max_digits`：最大收集位数（默认 1）
/// - `timeout_secs`：超时秒数（默认 5）
/// - `terminator`：终止符（默认 None）
///
/// 返回 [`NodeExecuteResult::WaitForDtmf`]，由拓扑引擎负责实际等待与端口映射。
pub async fn execute_collect_input(
    node: &TopologyNode,
    context: &IvrExecutionContext,
) -> NodeExecuteResult {
    let max_digits = node
        .config
        .get("max_digits")
        .and_then(|v| v.as_u64())
        .map(|v| v as u8)
        .unwrap_or(1)
        .clamp(1, 32);
    let timeout_secs = node
        .config
        .get("timeout_secs")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32)
        .unwrap_or(5);
    let terminator = node
        .config
        .get("terminator")
        .and_then(|v| v.as_str())
        .and_then(|s| s.chars().next())
        .filter(|c| *c != '\0');

    info!(
        call_id = %context.call_id,
        node_id = %node.id,
        node_type = %node.node_type,
        max_digits,
        timeout_secs,
        ?terminator,
        "IVR collect_input 节点等待 DTMF"
    );
    NodeExecuteResult::WaitForDtmf {
        max_digits,
        timeout_secs,
        terminator,
    }
}
