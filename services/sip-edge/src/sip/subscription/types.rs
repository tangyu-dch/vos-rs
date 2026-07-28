//! 订阅数据模型与事件包定义。

use std::net::SocketAddr;
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

/// 默认订阅有效期（秒）。RFC 6665 §3.1.1 建议值。
pub const DEFAULT_EXPIRES_SECONDS: u32 = 3600;
/// 最大订阅有效期（秒），与 REGISTER 一致以避免长期占用。
pub const MAX_EXPIRES_SECONDS: u32 = 86_400;

/// 当前支持的事件包类型。
///
/// 扩展新事件包时，仅需在此枚举中追加并实现 `from_header` 解析即可。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EventPackage {
    /// RFC 3856 — 在线状态公告（BLF）。
    Presence,
    /// RFC 4235 — 对话状态公告（BLF）。
    Dialog,
    /// RFC 3842 — 语音留言计数（MWI）。
    MessageSummary,
}

impl EventPackage {
    /// 从 `Event` 头解析事件包类型，支持带参数形式（如 `dialog;call-id=...`）。
    pub fn from_header(value: &str) -> Option<Self> {
        let token = value.split(';').next()?.trim().to_ascii_lowercase();
        match token.as_str() {
            "presence" => Some(Self::Presence),
            "dialog" => Some(Self::Dialog),
            "message-summary" => Some(Self::MessageSummary),
            _ => None,
        }
    }

    /// 返回写入 `Event` 头的规范名称。
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Presence => "presence",
            Self::Dialog => "dialog",
            Self::MessageSummary => "message-summary",
        }
    }
}

/// 订阅唯一标识，用于路由后续 NOTIFY 与刷新请求。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SubscriptionId(String);

impl SubscriptionId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    #[allow(dead_code)]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SubscriptionId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// NOTIFY 中的 `Subscription-State` 取值。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SubscriptionState {
    /// 订阅活跃，附带剩余有效期（秒）。
    Active { expires: u32 },
    /// 订阅已终止，附带原因（可选）。
    Terminated { reason: Option<&'static str> },
    /// 订阅等待中（首次状态尚未就绪）。
    Pending { expires: u32 },
}

impl SubscriptionState {
    /// 序列化为 `Subscription-State` 头值。
    pub fn to_header_value(self) -> String {
        match self {
            Self::Active { expires } => format!("active;expires={expires}"),
            Self::Pending { expires } => format!("pending;expires={expires}"),
            Self::Terminated { reason } => match reason {
                Some(value) => format!("terminated;reason={value}"),
                None => "terminated".to_string(),
            },
        }
    }
}

/// 一条活跃订阅。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Subscription {
    /// 订阅 ID（通常为 dialog 的 Call-ID + From-tag 组合）。
    pub id: SubscriptionId,
    /// 订阅的目标 AOR（如 `sip:1001@example.com`）。
    pub aor: String,
    /// 事件包类型。
    pub event_package: EventPackage,
    /// 订阅者 Contact URI（用于发送 NOTIFY）。
    pub contact_uri: String,
    /// 订阅者传输地址。
    pub peer: SocketAddr,
    /// 订阅者 From URI（NOTIFY 的 To 头）。
    pub from_uri: String,
    /// 订阅者 To URI（NOTIFY 的 From 头）。
    pub to_uri: String,
    /// 订阅对话框 Call-ID（NOTIFY 使用相同 Call-ID）。
    pub dialog_call_id: String,
    /// 订阅者本地 tag（NOTIFY 的 To tag）。
    pub local_tag: String,
    /// 订阅者远端 tag（NOTIFY 的 From tag）。
    pub remote_tag: Option<String>,
    /// 订阅创建时使用的 Record-Route 集合（向订阅者回送时反向）。
    pub route_set: Vec<String>,
    /// 上一次 NOTIFY 的 CSeq 序号。
    pub last_cseq: u32,
    /// 订阅到期时间戳。
    pub expires_at: SystemTime,
}

impl Subscription {
    /// 计算剩余有效期（秒），已过期返回 0。
    #[allow(dead_code)]
    pub fn remaining_seconds(&self, now: SystemTime) -> u32 {
        match self.expires_at.duration_since(now) {
            Ok(duration) => duration.as_secs().min(u32::MAX as u64) as u32,
            Err(_) => 0,
        }
    }

    /// 是否已过期。
    pub fn is_expired(&self, now: SystemTime) -> bool {
        self.expires_at <= now
    }

    /// 构造下一次 NOTIFY 的 CSeq（递增 1）。
    pub fn next_cseq(&self) -> u32 {
        self.last_cseq.saturating_add(1)
    }
}

/// 默认有效期（秒），当 SUBSCRIBE 未携带 Expires 头时使用。
pub fn default_expires_seconds() -> u32 {
    DEFAULT_EXPIRES_SECONDS
}

/// 将请求的 Expires 值规范化到 `[1, MAX_EXPIRES_SECONDS]` 范围。
///
/// - `0` 视为立即终止订阅，由调用方处理（不会进入此函数）。
/// - 超过上限的值被截断为 `MAX_EXPIRES_SECONDS`。
pub fn normalize_expires(requested: u32) -> u32 {
    if requested == 0 {
        return DEFAULT_EXPIRES_SECONDS;
    }
    requested.min(MAX_EXPIRES_SECONDS)
}

/// 根据规范化后的有效期生成到期时间戳。
pub fn expires_at_from(now: SystemTime, expires_secs: u32) -> SystemTime {
    now + Duration::from_secs(expires_secs as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    fn dummy_subscription(expires_secs: u32) -> Subscription {
        Subscription {
            id: SubscriptionId::new("call-123-fromtag"),
            aor: "sip:1001@example.com".to_string(),
            event_package: EventPackage::Dialog,
            contact_uri: "sip:1001@10.0.0.1:5060".to_string(),
            peer: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)), 5060),
            from_uri: "sip:watcher@example.com".to_string(),
            to_uri: "sip:1001@example.com".to_string(),
            dialog_call_id: "call-123".to_string(),
            local_tag: "local-tag".to_string(),
            remote_tag: Some("remote-tag".to_string()),
            route_set: Vec::new(),
            last_cseq: 0,
            expires_at: SystemTime::now() + Duration::from_secs(expires_secs as u64),
        }
    }

    #[test]
    fn event_package_parsing_supports_parameters() {
        assert_eq!(
            EventPackage::from_header("presence"),
            Some(EventPackage::Presence)
        );
        assert_eq!(
            EventPackage::from_header("dialog;call-id=abc@host"),
            Some(EventPackage::Dialog)
        );
        assert_eq!(
            EventPackage::from_header("message-summary"),
            Some(EventPackage::MessageSummary)
        );
        assert_eq!(EventPackage::from_header("unknown-pkg"), None);
    }

    #[test]
    fn subscription_state_header_serialization() {
        assert_eq!(
            SubscriptionState::Active { expires: 600 }.to_header_value(),
            "active;expires=600"
        );
        assert_eq!(
            SubscriptionState::Terminated {
                reason: Some("timeout")
            }
            .to_header_value(),
            "terminated;reason=timeout"
        );
        assert_eq!(
            SubscriptionState::Terminated { reason: None }.to_header_value(),
            "terminated"
        );
    }

    #[test]
    fn subscription_remaining_seconds_never_negative() {
        let sub = dummy_subscription(100);
        let now = sub.expires_at - Duration::from_secs(40);
        assert_eq!(sub.remaining_seconds(now), 40);
    }

    #[test]
    fn subscription_expired_when_past_expiry() {
        let sub = dummy_subscription(0);
        let later = sub.expires_at + Duration::from_secs(10);
        assert!(sub.is_expired(later));
    }

    #[test]
    fn normalize_expires_clamps_to_maximum() {
        assert_eq!(normalize_expires(0), DEFAULT_EXPIRES_SECONDS);
        assert_eq!(normalize_expires(1), 1);
        assert_eq!(
            normalize_expires(MAX_EXPIRES_SECONDS * 2),
            MAX_EXPIRES_SECONDS
        );
    }
}
