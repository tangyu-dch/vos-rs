use call_core::{Call, CallCdr, CallDirection, CallState, CdrAuditSnapshot, LegState};
use sip_core::{parse_message, SipMessage, SipUri};
use std::str::FromStr;

#[test]
fn creates_call_from_inbound_invite() {
    let request = invite_request();
    let call = Call::from_inbound_invite(&request).expect("call should be created");

    assert_eq!(call.id.as_str(), "call-1@example.com");
    assert_eq!(call.state, CallState::Routing);
    assert_eq!(call.inbound.state, LegState::Inviting);
    assert!(call.outbound.is_none());
    assert_eq!(call.direction, "outbound");
}

#[test]
fn creates_call_with_trusted_inbound_direction() {
    let request = invite_request();
    let call = Call::from_inbound_invite_with_direction(&request, CallDirection::Inbound)
        .expect("call should be created");

    assert_eq!(call.direction, "inbound");
}

#[test]
fn selecting_route_adds_outbound_leg() {
    let request = invite_request();
    let mut call = Call::from_inbound_invite(&request).expect("call should be created");
    let route = SipUri::from_str("sip:13800138000@gw1.example.com:5060").unwrap();

    call.select_route(route.clone())
        .expect("route should be selected");

    let outbound = call.outbound.expect("outbound leg should exist");
    assert_eq!(outbound.remote_uri, route);
    assert_eq!(outbound.state, LegState::Inviting);
}

#[test]
fn provisional_response_moves_call_to_ringing() {
    let mut call = routed_call();

    call.mark_ringing().expect("call should ring");

    assert_eq!(call.state, CallState::Ringing);
    assert_eq!(call.inbound.state, LegState::Ringing);
    assert_eq!(call.outbound.unwrap().state, LegState::Ringing);
}

#[test]
fn repeated_provisional_response_keeps_call_ringing() {
    let mut call = routed_call();

    call.mark_ringing()
        .expect("first provisional response should ring");
    call.mark_ringing()
        .expect("early media after 180 should remain valid");

    assert_eq!(call.state, CallState::Ringing);
    assert_eq!(call.inbound.state, LegState::Ringing);
    assert_eq!(
        call.outbound.expect("outbound leg should exist").state,
        LegState::Ringing
    );
}

#[test]
fn answered_call_becomes_established() {
    let mut call = routed_call();

    call.mark_answered().expect("call should be answered");

    assert_eq!(call.state, CallState::Established);
    assert_eq!(call.inbound.state, LegState::Answered);
    assert_eq!(call.outbound.unwrap().state, LegState::Answered);
}

#[test]
fn established_call_can_terminate() {
    let mut call = routed_call();
    call.mark_answered().unwrap();

    call.terminate().expect("call should terminate");

    assert_eq!(call.state, CallState::Terminated);
    assert_eq!(call.inbound.state, LegState::Terminated);
    assert_eq!(call.outbound.unwrap().state, LegState::Terminated);
}

#[test]
fn completed_cdr_preserves_frozen_audit_context() {
    let mut call = routed_call();
    call.audit = CdrAuditSnapshot {
        source_type: Some("trunk".to_string()),
        source_id: Some("access-a".to_string()),
        ingress_trunk_id: Some("access-a".to_string()),
        caller_mode: Some("virtual_pool".to_string()),
        caller_pool_id: Some("pool-a".to_string()),
        caller_selection: Some("round_robin".to_string()),
        billing_interval_secs: Some(6),
        price_per_interval: Some(0.05),
        ..CdrAuditSnapshot::default()
    };
    call.billing_account = Some("account-a".to_string());
    call.mark_answered().expect("call should be answered");
    call.terminate().expect("call should terminate");

    let cdr = CallCdr::from_completed_call(&call).expect("terminated call should create CDR");
    assert_eq!(cdr.audit.source_id.as_deref(), Some("access-a"));
    assert_eq!(cdr.audit.billing_account.as_deref(), Some("account-a"));
    assert_eq!(cdr.audit.billing_interval_secs, Some(6));
    assert_eq!(cdr.audit.price_per_interval, Some(0.05));
}

#[test]
fn routing_failure_marks_call_failed() {
    let request = invite_request();
    let mut call = Call::from_inbound_invite(&request).expect("call should be created");

    call.fail(Some(404), "no route").expect("call should fail");

    assert_eq!(call.state, CallState::Failed);
    assert_eq!(call.inbound.state, LegState::Failed);
    assert_eq!(call.failure_cause.unwrap().status_code, Some(404));
}

fn routed_call() -> Call {
    let request = invite_request();
    let mut call = Call::from_inbound_invite(&request).expect("call should be created");
    let route = SipUri::from_str("sip:13800138000@gw1.example.com:5060").unwrap();
    call.select_route(route).unwrap();
    call
}

fn invite_request() -> sip_core::SipRequest {
    let raw = concat!(
        "INVITE sip:13800138000@example.com SIP/2.0\r\n",
        "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-1\r\n",
        "From: <sip:1001@example.com>;tag=from-tag\r\n",
        "To: <sip:13800138000@example.com>\r\n",
        "Call-ID: call-1@example.com\r\n",
        "CSeq: 1 INVITE\r\n",
        "Content-Length: 0\r\n",
        "\r\n"
    );

    let SipMessage::Request(request) = parse_message(raw.as_bytes()).unwrap() else {
        panic!("expected request");
    };
    request
}
