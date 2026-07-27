//! # SIP 出站消息构建
//!
//! 本模块负责构建出站 SIP 消息，包括：
//!
//! - **INVITE**：出站呼叫邀请（含 SDP、Session Timer、Topology Hiding）
//! - **ACK**：确认响应
//! - **BYE**：终止呼叫
//! - **OPTIONS**：网关健康探测
//! - **NOTIFY**：REFER 转接进度通知
//! - **INFO**：DTMF 传递
//! - **REFER**：呼叫转接
//!
//! ## Topology Hiding
//!
//! 出站 INVITE 使用独立的 Call-ID（external_call_id），
//! 隐藏内部拓扑信息，防止外部网关探测内部网络结构。

mod helpers;
mod in_dialog;
mod invite;
#[cfg(test)]
mod tests;

pub use helpers::{is_forwardable_in_dialog_method, target_addr_for, target_addr_for_str};
pub use in_dialog::{
    build_b2bua_in_dialog_request, build_gateway_options, build_notify_sipfrag,
    build_notify_sipfrag_with_state, build_outbound_prack,
};
pub use invite::build_b2bua_outbound_invite;

pub(crate) use invite::build_transfer_invite;
