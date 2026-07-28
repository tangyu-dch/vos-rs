//! SDP 工具适配层（media-edge 独立服务版）。
//!
//! media-edge 的 SDP 处理主要通过 UDS/HTTP 端点对外暴露，与 sip-edge 中
//! 直接调用 `media_core::sdp` 的方式互补。本模块仅保留 media-edge 自身需要
//! 的 SocketAddr 解析工具；完整的 SDP 协商/改写/解析由 sip-edge 或调用方
//! 直接使用 `media_core::sdp` 完成。

use crate::media::recording::MediaError;
use sdp_core::RtpEndpoint;
use std::net::{SocketAddr, ToSocketAddrs};

/// 将 `RtpEndpoint` 解析为 `SocketAddr`，用于 RTP 中继目标地址设置。
///
/// 支持 IPv4 与 IPv6（含方括号）格式，解析失败时返回 `MediaError::InvalidEndpoint`。
pub fn socket_addr_for_endpoint(endpoint: &RtpEndpoint) -> Result<SocketAddr, MediaError> {
    let target = if endpoint.address.contains(':') {
        format!("[{}]:{}", endpoint.address, endpoint.port)
    } else {
        format!("{}:{}", endpoint.address, endpoint.port)
    };

    target
        .to_socket_addrs()
        .map_err(|_| MediaError::InvalidEndpoint(target.clone()))?
        .next()
        .ok_or(MediaError::InvalidEndpoint(target))
}
