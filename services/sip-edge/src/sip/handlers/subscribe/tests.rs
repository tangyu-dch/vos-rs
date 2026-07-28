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
