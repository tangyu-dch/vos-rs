//! # SIP SUBSCRIBE 请求处理
//!
//! 处理 RFC 6665 SUBSCRIBE 请求，维护订阅状态机并立即发送初始 NOTIFY。
//!
//! ## 处理流程
//!
//! ```text
//! SUBSCRIBE → 鉴权 → 解析 Event/Expires → upsert 订阅 → 200 OK
//!                                                          ↓
//!                                              立即发送初始 NOTIFY
//!                                              （Subscription-State: active）
//! ```
//!
//! ## 事件包支持
//!
//! 当前仅维护订阅状态本身，公告体（presence/dialog/MWI）由后续扩展填充。
//! 对于不支持的事件包返回 `489 Bad Event`。

use std::net::SocketAddr;
use std::str::FromStr;
use std::time::SystemTime;

use sip_core::SipRequest;
use tracing::{debug, warn};

use crate::config::EdgeConfig;
use crate::edge_state::{EdgeState, PendingDatagram};
use crate::sip::outbound::{header_uri, token_fragment};
use crate::sip::response;
use crate::sip::subscription::{
    expires_at_from, parse_subscribe_request, EventPackage, Subscription, SubscriptionState,
    SubscriptionStore, SubscriptionStoreError,
};

/// 处理入站 SUBSCRIBE 请求。
///
/// 返回响应 + 初始 NOTIFY 数据报列表（如有）。
pub(crate) async fn handle_subscribe_request(
    request: SipRequest,
    peer: SocketAddr,
    edge_state: &EdgeState,
    edge_config: &EdgeConfig,
) -> Vec<PendingDatagram> {
    let call_id = match request.headers.get("call-id") {
        Some(value) => value.as_str().to_string(),
        None => {
            return vec![PendingDatagram::new(
                peer.to_string(),
                bad_request(&request, "missing Call-ID"),
            )];
        }
    };

    let from_header = request
        .headers
        .get("from")
        .map(|value| value.as_str())
        .unwrap_or("");
    let from_tag = extract_tag_param(from_header).unwrap_or_default();
    if from_tag.is_empty() {
        return vec![PendingDatagram::new(
            peer.to_string(),
            bad_request(&request, "missing From tag"),
        )];
    }

    let event_header = request.headers.get("event").map(|value| value.as_str());
    let expires_header = request.headers.get("expires").map(|value| value.as_str());

    let (normalized_expires, event_package, subscription_id) =
        match parse_subscribe_request(&call_id, &from_tag, event_header, expires_header) {
            Ok(value) => value,
            Err(SubscriptionStoreError::UnsupportedEventPackage) => {
                return vec![PendingDatagram::new(
                    peer.to_string(),
                    bad_event_response(&request),
                )];
            }
        };

    // 解析订阅目标 AOR：取 To 头 URI
    let to_header = request
        .headers
        .get("to")
        .map(|value| value.as_str())
        .unwrap_or("");
    let aor = header_uri(to_header).unwrap_or_else(|| "sip:unknown@localhost".to_string());

    // 解析 Contact URI
    let contact_header = request
        .headers
        .get("contact")
        .map(|value| value.as_str())
        .unwrap_or("");
    let contact_uri = match header_uri(contact_header) {
        Some(uri) => uri,
        None => {
            return vec![PendingDatagram::new(
                peer.to_string(),
                bad_request(&request, "missing or invalid Contact"),
            )];
        }
    };

    // 处理 Expires=0（终止订阅）
    let requested_expires = expires_header
        .and_then(|value| value.trim().parse::<u32>().ok())
        .unwrap_or_else(crate::sip::subscription::default_expires_seconds);
    if requested_expires == 0 {
        let removed = edge_state.subscription_store.remove(&subscription_id);
        if let Some(subscription) = removed {
            let notify = build_notify(
                &subscription,
                "",
                &SubscriptionState::Terminated { reason: None },
                edge_config,
            );
            let datagram = PendingDatagram::new(subscription.peer.to_string(), notify);
            return vec![
                PendingDatagram::new(peer.to_string(), ok_response(&request, normalized_expires)),
                datagram,
            ];
        }
        return vec![PendingDatagram::new(
            peer.to_string(),
            ok_response(&request, normalized_expires),
        )];
    }

    // 构造新订阅
    let now = SystemTime::now();
    let subscription = Subscription {
        id: subscription_id.clone(),
        aor: aor.clone(),
        event_package,
        contact_uri: contact_uri.clone(),
        peer,
        from_uri: extract_uri(from_header).unwrap_or_default(),
        to_uri: extract_uri(to_header).unwrap_or_default(),
        dialog_call_id: call_id.clone(),
        local_tag: generate_local_tag(&call_id, &from_tag),
        remote_tag: Some(from_tag.clone()),
        route_set: extract_record_routes(&request),
        last_cseq: 0,
        expires_at: expires_at_from(now, normalized_expires),
    };

    let previous = edge_state.subscription_store.upsert(subscription.clone());

    debug!(
        call_id = %call_id,
        aor = %aor,
        event = %event_package.as_str(),
        expires = normalized_expires,
        refreshed = previous.is_some(),
        "SUBSCRIBE accepted"
    );

    let initial_state = SubscriptionState::Active {
        expires: normalized_expires,
    };
    let body = initial_body_for(event_package, &aor);
    let notify = build_notify(&subscription, &body, &initial_state, edge_config);
    let notify_datagram = PendingDatagram::new(peer.to_string(), notify);

    vec![
        PendingDatagram::new(peer.to_string(), ok_response(&request, normalized_expires)),
        notify_datagram,
    ]
}

/// 生成订阅响应（200 OK）。
fn ok_response(request: &SipRequest, expires: u32) -> Vec<u8> {
    response::build_response_with_owned_headers(
        request,
        200,
        "OK",
        &[("Expires".to_string(), expires.to_string())],
        "",
    )
}

/// 489 Bad Event — 不支持的事件包。
fn bad_event_response(request: &SipRequest) -> Vec<u8> {
    response::build_response_with_owned_headers(
        request,
        489,
        "Bad Event",
        &[(
            "Allow-Events".to_string(),
            "presence,dialog,message-summary".to_string(),
        )],
        "",
    )
}

fn bad_request(request: &SipRequest, reason: &str) -> Vec<u8> {
    response::build_response_with_owned_headers(
        request,
        400,
        "Bad Request",
        &[("X-VOS-RS-Error".to_string(), reason.to_string())],
        "",
    )
}

/// 构造 NOTIFY 请求体（初始状态）。
///
/// 当前为最简公告体；后续可扩展为 PIDF/XML、dialog-info+xml 等。
fn initial_body_for(event_package: EventPackage, aor: &str) -> String {
    match event_package {
        EventPackage::Presence => format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <presence xmlns=\"urn:ietf:params:xml:ns:pidf\"\n\
             entity=\"{aor}\">\n\
             <tuple id=\"vosrs\">\n\
             <status><basic>open</basic></status>\n\
             </tuple>\n\
             </presence>"
        ),
        EventPackage::Dialog => format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <dialog-info xmlns=\"urn:ietf:params:xml:ns:dialog-info\"\n\
             version=\"1\" state=\"full\" entity=\"{aor}\">\n\
             </dialog-info>"
        ),
        EventPackage::MessageSummary => {
            "Messages-Waiting: no\r\nMessage-Account: sip:vm@localhost\r\nVoice-Message: 0/0 (0/0)"
                .to_string()
        }
    }
}

/// 构建一个 NOTIFY 请求并发送给订阅者。
pub(crate) fn build_notify(
    subscription: &Subscription,
    body: &str,
    state: &SubscriptionState,
    edge_config: &EdgeConfig,
) -> Vec<u8> {
    let cseq = subscription.next_cseq();
    let branch = format!(
        "z9hG4bK-notify-{}-{}",
        token_fragment(&subscription.dialog_call_id),
        cseq
    );
    let target_uri = subscription
        .contact_uri
        .parse::<sip_core::SipUri>()
        .unwrap_or_else(|_| {
            sip_core::SipUri::from_str(&format!("sip:subscriber@{}", edge_config.advertised_addr))
                .expect("fallback URI is well-formed")
        });

    let mut request = format!(
        "NOTIFY {target_uri} SIP/2.0\r\n\
         Via: SIP/2.0/UDP {addr};branch={branch}\r\n\
         Max-Forwards: 70\r\n\
         From: <{from_uri}>;tag={local_tag}\r\n\
         To: <{to_uri}>",
        target_uri = target_uri,
        addr = edge_config.advertised_addr,
        branch = branch,
        from_uri = subscription.to_uri,
        local_tag = subscription.local_tag,
        to_uri = subscription.from_uri,
    );
    if let Some(remote_tag) = &subscription.remote_tag {
        request.push_str(";tag=");
        request.push_str(remote_tag);
    }
    request.push_str("\r\n");
    request.push_str(&format!("Call-ID: {}\r\n", subscription.dialog_call_id));
    request.push_str(&format!("CSeq: {cseq} NOTIFY\r\n"));
    request.push_str(&format!(
        "Contact: <sip:vosrs@{}>\r\n",
        edge_config.advertised_addr
    ));
    request.push_str(&format!(
        "Event: {}\r\n",
        subscription.event_package.as_str()
    ));
    request.push_str(&format!(
        "Subscription-State: {}\r\n",
        state.to_header_value()
    ));
    let content_type = content_type_for(subscription.event_package);
    request.push_str(&format!("Content-Type: {content_type}\r\n"));
    request.push_str(&format!("Content-Length: {}\r\n\r\n", body.len()));
    request.push_str(body);

    request.into_bytes()
}

fn content_type_for(event_package: EventPackage) -> &'static str {
    match event_package {
        EventPackage::Presence => "application/pidf+xml",
        EventPackage::Dialog => "application/dialog-info+xml",
        EventPackage::MessageSummary => "application/simple-message-summary",
    }
}

fn extract_tag_param(header: &str) -> Option<String> {
    header.split(';').skip(1).find_map(|parameter| {
        let (key, value) = parameter.trim().split_once('=')?;
        key.trim()
            .eq_ignore_ascii_case("tag")
            .then(|| value.trim().trim_end_matches('>').to_string())
    })
}

fn extract_uri(header: &str) -> Option<String> {
    header_uri(header)
}

fn extract_record_routes(request: &SipRequest) -> Vec<String> {
    request
        .headers
        .get_all("record-route")
        .map(|value| value.as_str().to_string())
        .collect()
}

fn generate_local_tag(call_id: &str, from_tag: &str) -> String {
    // 简单的稳定哈希，确保同一订阅刷新时返回相同 tag
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in call_id.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    for byte in from_tag.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("vosrs-sub-{hash:x}")
}

/// 触发对某 AOR 的状态变更通知，向所有订阅者发送 NOTIFY。
///
/// 用于呼叫状态变化时广播 BLF 状态。调用方提供公告体，函数负责为每个订阅者构造
/// NOTIFY 数据报。
#[allow(dead_code)]
pub(crate) fn notify_subscribers(
    store: &SubscriptionStore,
    event_package: EventPackage,
    aor: &str,
    body: &str,
    edge_config: &EdgeConfig,
    now: SystemTime,
) -> Vec<PendingDatagram> {
    let subscribers = store.subscribers_for(event_package, aor);
    if subscribers.is_empty() {
        return Vec::new();
    }

    let mut datagrams = Vec::with_capacity(subscribers.len());
    for subscription in subscribers {
        let remaining = subscription.remaining_seconds(now);
        let state = if remaining == 0 {
            SubscriptionState::Terminated {
                reason: Some("timeout"),
            }
        } else {
            SubscriptionState::Active { expires: remaining }
        };
        let notify = build_notify(&subscription, body, &state, edge_config);
        datagrams.push(PendingDatagram::new(subscription.peer.to_string(), notify));
    }
    datagrams
}

/// 处理入站 NOTIFY（订阅者侧）。
///
/// 当前作为 B2BUA 我们主要扮演 notifier 角色；作为 subscriber 的场景仅在 REFER
/// 转接中由 [`super::in_dialog::refer`] 处理，因此这里仅回 200 OK。
pub(crate) fn handle_notify_request(request: SipRequest, peer: SocketAddr) -> Vec<PendingDatagram> {
    if request.headers.get("subscription-state").is_none() {
        warn!(
            call_id = request.headers.get("call-id").map(|v| v.as_str()),
            "received NOTIFY without Subscription-State header"
        );
    }
    vec![PendingDatagram::new(
        peer.to_string(),
        response::build_response_with_owned_headers(&request, 200, "OK", &[], ""),
    )]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sip::subscription::SubscriptionId;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    #[test]
    fn extract_tag_param_finds_tag() {
        assert_eq!(
            extract_tag_param("<sip:a@b>;tag=abc123"),
            Some("abc123".to_string())
        );
        assert_eq!(extract_tag_param("<sip:a@b>"), None);
    }

    #[test]
    fn generate_local_tag_is_stable() {
        let tag1 = generate_local_tag("call-1", "from-tag");
        let tag2 = generate_local_tag("call-1", "from-tag");
        assert_eq!(tag1, tag2);

        let tag3 = generate_local_tag("call-2", "from-tag");
        assert_ne!(tag1, tag3);
    }

    #[test]
    fn initial_body_for_presence_is_valid_xml() {
        let body = initial_body_for(EventPackage::Presence, "sip:1001@example.com");
        assert!(body.contains("urn:ietf:params:xml:ns:pidf"));
        assert!(body.contains("sip:1001@example.com"));
    }

    #[test]
    fn initial_body_for_dialog_includes_entity() {
        let body = initial_body_for(EventPackage::Dialog, "sip:1001@example.com");
        assert!(body.contains("dialog-info"));
        assert!(body.contains("entity=\"sip:1001@example.com\""));
    }

    #[test]
    fn initial_body_for_mwi_has_voice_message_line() {
        let body = initial_body_for(EventPackage::MessageSummary, "sip:1001@example.com");
        assert!(body.starts_with("Messages-Waiting: no"));
        assert!(body.contains("Voice-Message:"));
    }

    #[test]
    fn content_type_for_matches_event_package() {
        assert_eq!(
            content_type_for(EventPackage::Presence),
            "application/pidf+xml"
        );
        assert_eq!(
            content_type_for(EventPackage::Dialog),
            "application/dialog-info+xml"
        );
        assert_eq!(
            content_type_for(EventPackage::MessageSummary),
            "application/simple-message-summary"
        );
    }

    #[test]
    fn build_notify_includes_required_headers() {
        let subscription = Subscription {
            id: SubscriptionId::new("test"),
            aor: "sip:1001@example.com".to_string(),
            event_package: EventPackage::Presence,
            contact_uri: "sip:watcher@10.0.0.1:5060".to_string(),
            peer: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)), 5060),
            from_uri: "sip:watcher@example.com".to_string(),
            to_uri: "sip:1001@example.com".to_string(),
            dialog_call_id: "call-1".to_string(),
            local_tag: "local-tag".to_string(),
            remote_tag: Some("remote-tag".to_string()),
            route_set: Vec::new(),
            last_cseq: 5,
            expires_at: SystemTime::now() + std::time::Duration::from_secs(600),
        };
        let edge_config = EdgeConfig {
            advertised_addr: "127.0.0.1:5060".to_string(),
            ..EdgeConfig::default()
        };
        let body = initial_body_for(EventPackage::Presence, &subscription.aor);
        let notify = build_notify(
            &subscription,
            &body,
            &SubscriptionState::Active { expires: 600 },
            &edge_config,
        );
        let notify_str = String::from_utf8(notify).unwrap();
        assert!(notify_str.starts_with("NOTIFY sip:watcher@10.0.0.1:5060 SIP/2.0"));
        assert!(notify_str.contains("CSeq: 6 NOTIFY"));
        assert!(notify_str.contains("Event: presence"));
        assert!(notify_str.contains("Subscription-State: active;expires=600"));
        assert!(notify_str.contains("Content-Type: application/pidf+xml"));
        assert!(notify_str.contains("To: <sip:watcher@example.com>;tag=remote-tag"));
        assert!(notify_str.contains("From: <sip:1001@example.com>;tag=local-tag"));
    }

    #[test]
    fn notify_subscribers_returns_empty_when_no_subscribers() {
        let store = SubscriptionStore::new();
        let edge_config = EdgeConfig {
            advertised_addr: "127.0.0.1:5060".to_string(),
            ..EdgeConfig::default()
        };
        let datagrams = notify_subscribers(
            &store,
            EventPackage::Presence,
            "sip:unknown@example.com",
            "body",
            &edge_config,
            SystemTime::now(),
        );
        assert!(datagrams.is_empty());
    }
}
