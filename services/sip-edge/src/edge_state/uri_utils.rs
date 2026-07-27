//! # SIP URI 工具函数
//!
//! 提供 Contact/Route 头解析与 peer 地址到 [`SipUri`] 的转换。

use sip_core::SipUri;
use std::net::SocketAddr;
use std::str::FromStr;

/// 从 Contact 头中提取 URI。
///
/// 支持 `<sip:user@host:port;params>` 与裸 URI 两种形式。
pub(crate) fn extract_uri_from_contact(contact: &str) -> Option<SipUri> {
    let contact = contact.trim();
    let uri_str = if let Some(start) = contact.find('<') {
        let end = contact.find('>')?;
        &contact[start + 1..end]
    } else {
        contact.split(';').next()?
    };
    SipUri::from_str(uri_str.trim()).ok()
}

/// 将 peer 地址字符串转换为 [`SipUri`]。
///
/// 优先解析为 `SocketAddr`；失败时退化为字符串切分。
pub(crate) fn sip_uri_from_peer(peer: &str) -> SipUri {
    match peer.parse::<SocketAddr>() {
        Ok(addr) => SipUri {
            secure: false,
            user: None,
            host: match addr.ip() {
                std::net::IpAddr::V4(ip) => ip.to_string().into(),
                std::net::IpAddr::V6(ip) => format!("[{ip}]").into(),
            },
            port: Some(addr.port()),
            params: Vec::new(),
        },
        Err(_) => SipUri {
            secure: false,
            user: None,
            host: peer
                .split(':')
                .next()
                .filter(|host| !host.is_empty())
                .unwrap_or(peer)
                .to_string()
                .into(),
            port: None,
            params: Vec::new(),
        },
    }
}

/// 从 Route 头中解析出 `host:port`。
///
/// 缺省端口补全为 5060。
pub(crate) fn parse_target_addr_from_route(route: &str) -> Option<String> {
    let route = route.trim();
    let uri_str = if let Some(start) = route.find('<') {
        let end = route.find('>')?;
        &route[start + 1..end]
    } else {
        route.split(';').next()?
    };
    let uri = SipUri::from_str(uri_str.trim()).ok()?;
    Some(format!("{}:{}", uri.host, uri.port.unwrap_or(5060)))
}
