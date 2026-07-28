//! 媒体转接节点执行器：transfer_ext / transfer_pstn / transfer_queue / record / voicemail
//!
//! transfer 类节点返回 [`NodeExecuteResult::Transfer`]，由拓扑引擎复用
//! `ivr::execute_ivr_action_for_topology` 完成实际转接；
//! record / voicemail 节点调用媒体层录音能力。

use super::super::types::*;
use crate::{EdgeConfig, EdgeState};
use tracing::{info, warn};

/// 执行 transfer_ext / transfer_pstn / transfer_queue 节点
///
/// 读取配置：
/// - `target`：转接目标（分机号 / PSTN 号码 / 队列 ID，必填，支持模板渲染）
///
/// `transfer_type` 由节点类型决定：extension / pstn / queue。
pub fn execute_transfer(node: &TopologyNode, context: &IvrExecutionContext) -> NodeExecuteResult {
    let transfer_type = match node.node_type.as_str() {
        "transfer_ext" => "extension",
        "transfer_pstn" => "pstn",
        "transfer_queue" => "queue",
        other => {
            return NodeExecuteResult::Error {
                message: format!("execute_transfer 不支持的节点类型: {other}"),
            };
        }
    };
    let target = match node.config.get("target").and_then(|v| v.as_str()) {
        Some(s) if !s.is_empty() => context.render_template(s),
        _ => {
            return NodeExecuteResult::Error {
                message: format!("{} 节点未配置 target", node.node_type),
            };
        }
    };
    info!(
        call_id = %context.call_id,
        node_id = %node.id,
        node_type = %node.node_type,
        transfer_type,
        target = %target,
        "IVR transfer 节点发起转接"
    );
    NodeExecuteResult::Transfer {
        target,
        transfer_type: transfer_type.to_string(),
    }
}

/// 执行 record / voicemail 节点：启动媒体层录音
///
/// 读取配置：
/// - `max_duration`：最大录音时长（秒，可选，受 MediaConfig.recording_max_duration_secs 上限约束）
/// - `format`：录音格式（wav / opus / amr，可选，默认走 MediaConfig.recording_format）
///
/// 复用 [`crate::media::relay::MediaRelayState::start_call_recording`] 接口。
/// IVR 单腿场景下，A-leg 既作为 caller 又作为 gateway 端口传入，
/// 媒体层会根据端口是否已注册远端目标自动选择本地或远端录音路径。
pub async fn execute_record(
    node: &TopologyNode,
    context: &IvrExecutionContext,
    a_port: u16,
    edge_state: &EdgeState,
    edge_config: &EdgeConfig,
) -> NodeExecuteResult {
    // 录音决策：租户策略优先于全局配置
    // - 租户 recording_enabled=Some(true)  → 强制录音（即使全局未启用）
    // - 租户 recording_enabled=Some(false) → 强制不录音
    // - 租户 recording_enabled=None         → 沿用全局 media.recording_enabled
    let tenant_ctx = edge_state
        .inbound_transactions
        .get(&context.call_id)
        .and_then(|t| t.tenant.clone());
    let recording_enabled = edge_state
        .tenant_recording_override(tenant_ctx.as_ref(), edge_config.media.recording_enabled);
    if !recording_enabled {
        warn!(
            call_id = %context.call_id,
            node_id = %node.id,
            "录音功能未启用（全局或租户策略关闭）, record 节点跳过"
        );
        return NodeExecuteResult::Continue {
            port: "default".to_string(),
        };
    }

    let max_duration = node
        .config
        .get("max_duration")
        .and_then(|v| v.as_u64())
        .unwrap_or(edge_config.media.recording_max_duration_secs);
    let format = node
        .config
        .get("format")
        .and_then(|v| v.as_str())
        .unwrap_or(&edge_config.media.recording_format)
        .to_string();

    info!(
        call_id = %context.call_id,
        node_id = %node.id,
        node_type = %node.node_type,
        a_port,
        max_duration,
        format = %format,
        "IVR record 节点启动录音"
    );

    // IVR 单腿录音：caller 与 gateway 端口同为 a_port
    start_recording_or_error(edge_state, context, a_port, &edge_config.media)
}

/// 调用媒体层启动单腿录音，返回对应 NodeExecuteResult
fn start_recording_or_error(
    edge_state: &EdgeState,
    context: &IvrExecutionContext,
    a_port: u16,
    media_config: &crate::media::MediaConfig,
) -> NodeExecuteResult {
    // media_relay.start_call_recording 的第一参数需要 B2BUA session_id
    // (内部 dtmf_accumulators / dtmf_event_log 按 session_id 索引),
    // 因此先通过 wire Call-ID 查找 transaction 取出 session_id。
    let session_id = match edge_state
        .inbound_transactions
        .get(&context.call_id)
        .map(|t| t.session_id.clone())
    {
        Some(id) => id,
        None => {
            warn!(
                call_id = %context.call_id,
                "IVR record 节点启动录音失败: 未找到对应 transaction, 无法获取 session_id"
            );
            return NodeExecuteResult::Error {
                message: format!(
                    "启动录音失败: 未找到 session_id (call_id={})",
                    context.call_id
                ),
            };
        }
    };
    match edge_state
        .media_relay
        .start_call_recording(&session_id, a_port, a_port, media_config)
    {
        Ok(_) => {
            info!(call_id = %context.call_id, node_id = %context.current_node_id.as_deref().unwrap_or(""), "IVR record 节点录音已启动");
            NodeExecuteResult::Continue {
                port: "default".to_string(),
            }
        }
        Err(e) => {
            warn!(
                call_id = %context.call_id,
                error = %e,
                "IVR record 节点启动录音失败"
            );
            NodeExecuteResult::Error {
                message: format!("启动录音失败: {e}"),
            }
        }
    }
}
