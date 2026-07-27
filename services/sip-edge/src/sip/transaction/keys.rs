//! # SIP 事务匹配键
//!
//! 本模块定义了用于识别 SIP 事务的键类型：
//!
//! - [`ClientTransactionKey`]：出站事务键（Call-ID + CSeq + Method + Branch）
//! - [`RequestTransactionKey`]：入站请求事务键（Peer + Method + Branch + Call-ID + CSeq）
//! - [`InviteAckKey`]：成功 INVITE 的 ACK 匹配键（独立于 Via branch）
//!
//! ## 设计要点
//!
//! ACK 对 2xx 响应是一个独立事务，其 branch 与 INVITE 不同，
//! 因此 [`InviteAckKey`] 使用 dialog 标识符 + CSeq 序号作为稳定键。

use sip_core::Method;
use std::net::SocketAddr;

/// 出站事务匹配键。
///
/// 用于在 [`crate::sip::client_transaction`] 中识别重传的出站请求。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct ClientTransactionKey {
    pub call_id: String,
    pub cseq: String,
    pub method: String,
    pub branch: String,
}

impl ClientTransactionKey {
    /// 从出站请求中提取事务键。
    ///
    /// ACK 不创建客户端事务（RFC 3261 §17.1.1.3），返回 `None`。
    pub(crate) fn from_request(request: &sip_core::SipRequestBorrow<'_>) -> Option<Self> {
        if matches!(&request.method, Method::Ack) {
            return None;
        }
        let branch = request
            .headers
            .get("via")
            .and_then(|via| branch_param(via.as_str()))?;
        let call_id = request.headers.get("call-id")?.as_str().to_string();
        let cseq_header = request.headers.get("cseq")?.as_str();
        let cseq = cseq_header.split_whitespace().next()?.to_string();
        let method = request.method.as_str().to_string();
        Some(Self {
            call_id,
            cseq,
            method,
            branch,
        })
    }
}

/// 入站请求事务匹配键。
///
/// 用于识别入站重传请求，组合 peer 地址、method、branch、Call-ID 和 CSeq。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct RequestTransactionKey {
    pub(crate) peer: String,
    pub(crate) method: String,
    pub(crate) branch: Option<String>,
    pub(crate) call_id: Option<String>,
    pub(crate) cseq: Option<String>,
}

/// 成功 INVITE 的 ACK 匹配键。
///
/// RFC 3261 定义 ACK 对 2xx 响应为独立事务，其 Via branch 通常与 INVITE 不同，
/// 因此使用 dialog 标识符（Call-ID）和 CSeq 序号作为稳定匹配键。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct InviteAckKey {
    call_id: String,
    cseq: String,
}

impl InviteAckKey {
    /// 从 INVITE 或 ACK 请求中提取 ACK 匹配键。
    ///
    /// 仅对 INVITE 和 ACK 方法返回 `Some`，其它方法返回 `None`。
    pub(crate) fn from_request(request: &sip_core::SipRequestBorrow<'_>) -> Option<Self> {
        if !matches!(&request.method, Method::Invite | Method::Ack) {
            return None;
        }
        let call_id = request.headers.get("call-id")?.as_str().to_string();
        let cseq = request
            .headers
            .get("cseq")?
            .as_str()
            .split_whitespace()
            .next()?
            .to_string();
        Some(Self { call_id, cseq })
    }
}

impl RequestTransactionKey {
    /// 从入站请求和 peer 地址构造事务键。
    ///
    /// ACK 不创建服务端事务（其事务匹配走 [`InviteAckKey`]），返回 `None`。
    /// 当 branch/Call-ID/CSeq 全部缺失时也返回 `None`，避免误匹配。
    pub(crate) fn from_request(
        request: &sip_core::SipRequestBorrow<'_>,
        peer: SocketAddr,
    ) -> Option<Self> {
        if matches!(&request.method, Method::Ack) {
            return None;
        }

        let branch = request
            .headers
            .get("via")
            .and_then(|via| branch_param(via.as_str()));
        let call_id = request
            .headers
            .get("call-id")
            .map(|value| value.as_str().to_string());
        let cseq = request
            .headers
            .get("cseq")
            .map(|value| value.as_str().to_string());

        if branch.is_none() && call_id.is_none() && cseq.is_none() {
            return None;
        }

        Some(Self {
            peer: peer.to_string(),
            method: request.method.as_str().to_string(),
            branch,
            call_id,
            cseq,
        })
    }

    /// 将 INVITE 事务键转换为对应的 [`InviteAckKey`]，用于后续 ACK 匹配。
    ///
    /// 非 INVITE 方法返回 `None`。
    pub(crate) fn invite_ack_key(&self) -> Option<InviteAckKey> {
        if self.method != "INVITE" {
            return None;
        }
        let cseq = self.cseq.as_deref()?.split_whitespace().next()?.to_string();
        Some(InviteAckKey {
            call_id: self.call_id.clone()?,
            cseq,
        })
    }

    #[cfg(test)]
    pub(crate) fn new_manual(
        peer: String,
        method: String,
        branch: Option<String>,
        call_id: Option<String>,
        cseq: Option<String>,
    ) -> Self {
        Self {
            peer,
            method,
            branch,
            call_id,
            cseq,
        }
    }
}

/// 从 Via 头中提取 `branch` 参数值。
///
/// RFC 3261 §8.1.1.7 要求 branch 参数以 `z9hG4bK` 前缀开头作为魔术字。
/// 本函数仅做参数解析，不验证魔术字。
pub(crate) fn branch_param(via: &str) -> Option<String> {
    via.split(';').skip(1).find_map(|param| {
        let (name, value) = param.trim().split_once('=')?;
        name.trim()
            .eq_ignore_ascii_case("branch")
            .then(|| value.trim().to_string())
            .filter(|value| !value.is_empty())
    })
}
