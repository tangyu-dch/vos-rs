//! # EdgeState 辅助类型
//!
//! 本模块定义 [`EdgeState`][super::EdgeState] 周边的辅助数据类型：
//!
//! - [`IvrMenu`] / [`IvrAction`]：IVR 菜单配置（扁平 DTMF 表 + 可视化拓扑）
//! - [`OutboundRegState`]：出站注册状态（gateway 注册客户端）
//! - [`CdrSinks`]：CDR 输出端聚合（PostgreSQL + NATS）
//! - [`ReferSubscription`]：REFER 转接订阅状态
//! - [`ParkedCall`]：呼叫停泊槽位

use std::collections::HashMap;

use cdr_core::PostgresCdrStore;

use crate::sip::handlers::ivr_topology::IvrTopology;

/// IVR 菜单配置。
///
/// 当 `topology` 存在且节点非空时走拓扑引擎；否则回退到扁平 DTMF 表 `actions`。
#[derive(Debug, Clone)]
pub(crate) struct IvrMenu {
    pub(crate) welcome_prompt: String,
    pub(crate) timeout_secs: i32,
    pub(crate) actions: HashMap<String, IvrAction>,
    /// 可视化拓扑画布（存在且 nodes 非空时走拓扑引擎，否则走扁平 DTMF 表）
    pub(crate) topology: Option<IvrTopology>,
}

#[derive(Debug, Clone)]
pub(crate) struct IvrAction {
    pub(crate) action_type: String,
    pub(crate) action_target: String,
    pub(crate) waiting_prompt: Option<String>,
    pub(crate) webhook_method: Option<String>,
}

/// 出站网关注册状态。
///
/// 由 `outbound_reg` 模块维护，用于周期性向远端网关发起 REGISTER。
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) struct OutboundRegState {
    pub(crate) gateway_id: String,
    pub(crate) host: String,
    pub(crate) port: Option<u16>,
    pub(crate) transport: String,
    pub(crate) username: String,
    pub(crate) password: String,
    pub(crate) call_id: String,
    pub(crate) cseq: u32,
    pub(crate) from_tag: String,
    pub(crate) expires: u32,
    pub(crate) last_reg_sent: Option<std::time::Instant>,
    pub(crate) last_reg_success: Option<std::time::Instant>,
    pub(crate) challenge: Option<HashMap<String, String>>,
}

/// CDR 输出端聚合。
///
/// `postgres` 与 `nats` 任一可选，运行时按可用 sink 分别写入。
#[derive(Debug, Clone, Default)]
pub(crate) struct CdrSinks {
    pub(crate) postgres: Option<PostgresCdrStore>,
    pub(crate) nats: Option<crate::cdr::NatsCdrPublisher>,
}

/// REFER 转接订阅状态。
///
/// 跟踪原 REFER 请求的 From/To 头、NOTIFY CSeq、referrer peer 地址以及
/// 目标 relay 端口（用于媒体切换）。
#[derive(Debug, Clone)]
pub(crate) struct ReferSubscription {
    pub(crate) from_header: String,
    pub(crate) to_header: String,
    pub(crate) notify_cseq: u32,
    pub(crate) referrer_peer: String,
    pub(crate) target_relay_port: Option<u16>,
}

/// 呼叫停泊槽位。
///
/// 用于 IVR 等场景临时保存入站 INVITE 上下文，等待后续转接或恢复。
#[derive(Clone)]
pub(crate) struct ParkedCall {
    pub(crate) session_id: String,
    pub(crate) invite_request: sip_core::SipRequest,
    pub(crate) peer_addr: std::net::SocketAddr,
    pub(crate) caller_relay_port: u16,
}
