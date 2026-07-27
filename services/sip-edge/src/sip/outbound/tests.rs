use super::helpers::{caller_identity_headers, target_addr_for};
use super::in_dialog::{
    build_b2bua_in_dialog_request, build_gateway_options, build_notify_sipfrag,
};
use call_core::{CallerIdentity, CallerIdentityMode, GatewayId};
use sip_core::{parse_message, SipUri};
use std::str::FromStr;

#[test]
fn caller_identity_headers_rebuild_from_and_pai_without_forwarding_display_name() {
    let inbound = invite_request();
    let identity = CallerIdentity {
        original_number: "1001".to_string(),
        presented_number: "13800138000".to_string(),
        owner_gateway_id: GatewayId::new("gw1"),
        mode: CallerIdentityMode::Fixed,
        max_concurrent: 1,
    };
    let headers = caller_identity_headers(&inbound, "edge.example.com:5060", &identity)
        .expect("valid identity headers");
    assert_eq!(
        headers,
        "From: <sip:13800138000@edge.example.com:5060>;tag=from-tag\r\n\
         P-Asserted-Identity: <sip:13800138000@edge.example.com:5060>\r\n"
    );
    assert!(!headers.contains("1001"));
}

#[test]
fn caller_identity_headers_reject_header_injection() {
    let inbound = invite_request();
    let identity = CallerIdentity {
        original_number: "1001".to_string(),
        presented_number: "13800138000\r\nX-Evil: yes".to_string(),
        owner_gateway_id: GatewayId::new("gw1"),
        mode: CallerIdentityMode::Fixed,
        max_concurrent: 1,
    };
    assert!(caller_identity_headers(&inbound, "edge.example.com", &identity).is_none());
}

#[test]
fn builds_gateway_options_probe() {
    let target = SipUri::from_str("sip:health-check@gw1.example.com:5060").unwrap();
    let options = String::from_utf8(build_gateway_options(
        &target,
        "edge.example.com:5060",
        "health-probe-gw1-1",
        1,
    ))
    .unwrap();

    assert!(options.starts_with("OPTIONS sip:health-check@gw1.example.com:5060 SIP/2.0\r\n"));
    assert!(options.contains("CSeq: 1 OPTIONS\r\n"));
    assert!(options.contains("Call-ID: health-probe-gw1-1\r\n"));
    assert!(options.contains("Content-Length: 0\r\n\r\n"));
}

#[test]
fn b2bua_in_dialog_request_uses_only_target_leg_dialog_identifiers() {
    let inbound = request(concat!(
        "BYE sip:edge@example.com SIP/2.0\r\n",
        "Via: SIP/2.0/UDP caller.example.com;branch=caller-branch\r\n",
        "From: <sip:caller@example.com>;tag=caller-tag\r\n",
        "To: <sip:callee@example.com>;tag=edge-a-tag\r\n",
        "Call-ID: caller-leg-id\r\n",
        "CSeq: 44 BYE\r\n",
        "Reason: SIP;cause=200;text=normal\r\n",
        "Content-Length: 0\r\n\r\n"
    ));
    let target = SipUri::from_str("sip:callee@callee.example.com:5070").unwrap();
    let local = SipUri::from_str("sip:caller@edge.example.com").unwrap();
    let remote = SipUri::from_str("sip:callee@callee.example.com").unwrap();

    let outbound = String::from_utf8(build_b2bua_in_dialog_request(
        &inbound,
        &target,
        "edge.example.com:5060",
        &["<sip:proxy.example.com;lr>".to_string()],
        "gateway-leg-id",
        &local,
        "edge-b-tag",
        &remote,
        Some("callee-tag"),
        9,
        &[],
    ))
    .unwrap();

    assert!(outbound.starts_with("BYE sip:callee@callee.example.com:5070 SIP/2.0\r\n"));
    assert!(outbound.contains("Route: <sip:proxy.example.com;lr>\r\n"));
    assert!(outbound.contains("From: <sip:caller@edge.example.com>;tag=edge-b-tag\r\n"));
    assert!(outbound.contains("To: <sip:callee@callee.example.com>;tag=callee-tag\r\n"));
    assert!(outbound.contains("Call-ID: gateway-leg-id\r\n"));
    assert!(outbound.contains("CSeq: 9 BYE\r\n"));
    assert!(!outbound.contains("caller-leg-id"));
    assert!(!outbound.contains("caller-tag"));
    assert!(!outbound.contains("caller-branch"));
}

#[test]
fn target_addr_defaults_to_5060() {
    let uri = SipUri::from_str("sip:13800138000@gw1.example.com").unwrap();

    assert_eq!(target_addr_for(&uri), "gw1.example.com:5060");
}

#[test]
fn builds_notify_sipfrag_for_refer_progress() {
    let notify = build_notify_sipfrag(
        "refer-call@example.com",
        "<sip:1001@example.com>;tag=from-tag",
        "<sip:13800138000@example.com>;tag=to-tag",
        52,
        "edge.example.com:5060",
        "SIP/2.0 100 Trying\r\n",
    );
    let notify = String::from_utf8(notify).expect("NOTIFY should be UTF-8");

    assert!(notify.starts_with("NOTIFY sip:1001@example.com SIP/2.0\r\n"));
    assert!(notify.contains(
        "Via: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-notify-refer-call-example-com-52\r\n"
    ));
    assert!(notify.contains("From: <sip:13800138000@example.com>;tag=to-tag\r\n"));
    assert!(notify.contains("To: <sip:1001@example.com>;tag=from-tag\r\n"));
    assert!(notify.contains("Call-ID: refer-call@example.com\r\n"));
    assert!(notify.contains("CSeq: 52 NOTIFY\r\n"));
    assert!(notify.contains("Event: refer\r\n"));
    assert!(notify.contains("Subscription-State: active;expires=60\r\n"));
    assert!(notify.contains("Content-Type: message/sipfrag;version=2.0\r\n"));
    assert!(notify.ends_with("Content-Length: 20\r\n\r\nSIP/2.0 100 Trying\r\n"));
}

fn invite_request() -> sip_core::SipRequest {
    let raw = concat!(
        "INVITE sip:13800138000@example.com SIP/2.0\r\n",
        "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-1\r\n",
        "Max-Forwards: 70\r\n",
        "From: <sip:1001@example.com>;tag=from-tag\r\n",
        "To: <sip:13800138000@example.com>\r\n",
        "Call-ID: call-1@example.com\r\n",
        "CSeq: 1 INVITE\r\n",
        "Content-Type: application/sdp\r\n",
        "Content-Length: 5\r\n",
        "\r\n",
        "v=0\r\n"
    );

    request(raw)
}

fn request(raw: &str) -> sip_core::SipRequest {
    let sip_core::SipMessageBorrow::Request(request) = parse_message(raw.as_bytes()).unwrap()
    else {
        panic!("expected request");
    };
    request.into_owned()
}
