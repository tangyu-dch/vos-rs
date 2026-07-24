//! IVR 节点执行器分发入口
//!
//! 按 [`IvrNodeType`] 分发到对应的执行器子模块：
//! - [`basic`]：start / hangup / prompt / collect_dtmf / menu
//! - [`flow`]：condition / route / set_var / loop / http_webhook
//! - [`media`]：transfer_ext / transfer_pstn / transfer_queue / record / voicemail
//! - [`voice`]：tts / asr（当前为 stub，TTS/ASR 引擎后续阶段接入）
//! - [`ai`]：ai_agent（当前为 stub，LLM 接入后续阶段实现）

#![allow(unused_variables)]

use super::types::*;
use crate::{EdgeConfig, EdgeState};
use sip_core::SipRequest;
use std::net::SocketAddr;

pub mod ai;
pub mod basic;
pub mod flow;
pub mod media;
pub mod voice;

/// 按节点类型分发到具体执行器
///
/// # Arguments
/// * `node_type` - 节点类型枚举
/// * `node` - 拓扑节点（含 config JSON）
/// * `graph` - 拓扑图索引（用于查询后续节点）
/// * `context` - 当前通话的执行上下文
/// * `a_port` - A-leg 本地媒体端口
/// * `caller_peer` - 主叫 socket 地址
/// * `template_request` - 触发 IVR 的原始 SIP 请求（用于转接模板）
/// * `edge_state` - 边缘节点共享状态
/// * `edge_config` - 边缘节点配置
//
// 参数数量由现有 [`crate::sip::handlers::ivr_topology::engine::TopologyEngine`] 调用约定决定，
// 暂不允许拆分；与原 stub 实现保持一致。
#[allow(clippy::too_many_arguments)]
pub async fn dispatch(
    node_type: IvrNodeType,
    node: &TopologyNode,
    graph: &TopologyGraph,
    context: &mut IvrExecutionContext,
    a_port: u16,
    caller_peer: SocketAddr,
    template_request: &SipRequest,
    edge_state: &EdgeState,
    edge_config: &EdgeConfig,
) -> NodeExecuteResult {
    match node_type {
        IvrNodeType::Start | IvrNodeType::Hangup => basic::execute(node, context).await,
        IvrNodeType::Prompt => basic::execute_prompt(node, context, a_port, edge_state).await,
        IvrNodeType::CollectDtmf | IvrNodeType::Menu => {
            basic::execute_collect_input(node, context).await
        }
        IvrNodeType::Condition => flow::execute_condition(node, context),
        IvrNodeType::Route => flow::execute_route(node, context),
        IvrNodeType::SetVar => flow::execute_set_var(node, context),
        IvrNodeType::Loop => flow::execute_loop(node, context),
        IvrNodeType::HttpWebhook => flow::execute_webhook(node, context).await,
        IvrNodeType::Tts => voice::execute_tts(node, context, a_port, edge_state).await,
        IvrNodeType::Asr => voice::execute_asr(node, context).await,
        IvrNodeType::AiAgent => ai::execute_ai_agent(node, context, edge_config).await,
        IvrNodeType::TransferExt | IvrNodeType::TransferPstn | IvrNodeType::TransferQueue => {
            media::execute_transfer(node, context)
        }
        IvrNodeType::Voicemail | IvrNodeType::Record => {
            media::execute_record(node, context, a_port, edge_state, edge_config).await
        }
    }
}
