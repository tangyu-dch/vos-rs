use call_core::{
    CallError, CallEvent, CallId, CallManager, CallState, CdrStatus, Route, RouteTable, RouteTarget,
};
use sip_core::{parse_message, SipUri};
use std::str::FromStr;

#[test]
fn route_table_selects_longest_prefix() {
    let table = RouteTable::new(vec![
        Route::new(
            "default",
            "",
            100,
            RouteTarget::new("gw-default", "default-gw.example.com", Some(5060)),
        ),
        Route::new(
            "mobile-1380",
            "1380",
            10,
            RouteTarget::new("gw-mobile", "mobile-gw.example.com", Some(5070)),
        ),
    ]);
    let destination = SipUri::from_str("sip:13800138000@example.com").unwrap();

    let selected = table.select(&destination).expect("route should match");

    assert_eq!(selected.route_id, "mobile-1380");
    assert_eq!(selected.target.gateway_id.as_str(), "gw-mobile");
    assert_eq!(
        selected.outbound_uri.to_string(),
        "sip:13800138000@mobile-gw.example.com:5070;transport=udp"
    );
}

#[test]
fn call_manager_accepts_invite_and_stores_routed_call() {
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let manager = CallManager::new(test_routes(), tx);
    let request = invite_request("call-1@example.com", "13800138000");

    let outcome = manager
        .handle_inbound_invite(&request)
        .expect("invite should be accepted");

    assert_eq!(outcome.call_id.as_str(), "call-1@example.com");
    assert_eq!(outcome.state, CallState::Routing);
    assert_eq!(
        outcome.outbound_uri.to_string(),
        "sip:13800138000@gw1.example.com:5060;transport=udp"
    );

    let call = manager
        .get(&CallId::new("call-1@example.com"))
        .expect("call should be stored");
    assert_eq!(call.state, CallState::Routing);
    assert!(call.outbound.is_some());
}

#[test]
fn call_manager_emits_ordered_lifecycle_webhook_events() {
    let (cdr_tx, _cdr_rx) = tokio::sync::mpsc::unbounded_channel();
    let (event_tx, mut event_rx) = tokio::sync::mpsc::channel(8);
    let manager = CallManager::new_with_event_sink(test_routes(), cdr_tx, event_tx);
    let call_id = "call-webhook@example.com";

    manager
        .handle_inbound_invite(&invite_request(call_id, "13800138000"))
        .expect("呼叫应进入路由状态");
    manager
        .handle_outbound_response(&outbound_response(180, "Ringing", call_id))
        .expect("呼叫应进入振铃状态");
    manager
        .handle_outbound_response(&outbound_response(200, "OK", call_id))
        .expect("呼叫应进入接通状态");
    manager
        .handle_inbound_termination(&bye_request(call_id), None, None)
        .expect("呼叫应正常结束");

    let events = std::iter::from_fn(|| event_rx.try_recv().ok()).collect::<Vec<_>>();
    assert_eq!(events.len(), 4);
    assert!(matches!(events[0].event, CallEvent::CallInitiated { .. }));
    assert!(matches!(events[1].event, CallEvent::CallRinging { .. }));
    assert!(matches!(events[2].event, CallEvent::CallAnswered { .. }));
    assert!(matches!(events[3].event, CallEvent::CallFinished { .. }));
    assert!(events
        .windows(2)
        .all(|pair| pair[0].sequence < pair[1].sequence));
    assert!(events.iter().all(|event| event.call_id == call_id));
}

#[test]
fn call_manager_accepts_preselected_outbound_uri() {
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let manager = CallManager::new(RouteTable::default(), tx);
    let request = invite_request("call-direct@example.com", "1002");
    let outbound_uri = SipUri::from_str("sip:1002@192.0.2.20:5062;transport=udp").unwrap();

    let outcome = manager
        .handle_inbound_invite_to_uri(&request, outbound_uri)
        .expect("invite should use preselected outbound URI");

    assert_eq!(outcome.call_id.as_str(), "call-direct@example.com");
    assert_eq!(
        outcome.outbound_uri.to_string(),
        "sip:1002@192.0.2.20:5062;transport=udp"
    );

    let call = manager
        .get(&CallId::new("call-direct@example.com"))
        .expect("call should be stored");
    assert_eq!(call.state, CallState::Routing);
    assert_eq!(
        call.outbound.as_ref().unwrap().remote_uri.to_string(),
        "sip:1002@192.0.2.20:5062;transport=udp"
    );
}

#[test]
fn call_manager_fails_invite_when_no_route_matches() {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let manager = CallManager::new(RouteTable::default(), tx);
    let request = invite_request("call-2@example.com", "13900139000");

    let error = manager
        .handle_inbound_invite(&request)
        .expect_err("invite should fail without a route");

    assert_eq!(
        error,
        CallError::NoRouteForDestination("13900139000".to_string())
    );

    let call = manager
        .get(&CallId::new("call-2@example.com"))
        .expect("failed call should be stored for later CDR work");
    assert_eq!(call.state, CallState::Failed);
    assert!(call.failure_cause.is_some());

    let cdr = rx.try_recv().expect("CDR should exist");
    assert_eq!(cdr.call_id.as_str(), "call-2@example.com");
    assert_eq!(cdr.status, CdrStatus::Failed);
    assert_eq!(cdr.callee.as_deref(), Some("13900139000"));
    assert!(cdr.failure_cause.is_some());
}

#[test]
fn outbound_ringing_response_moves_call_to_ringing() {
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let manager = routed_manager_with_call("call-3@example.com", tx);
    let response = outbound_response(180, "Ringing", "call-3@example.com");

    let outcome = manager
        .handle_outbound_response(&response)
        .expect("response should update call");

    assert_eq!(outcome.state, CallState::Ringing);
    assert_eq!(
        manager
            .get(&CallId::new("call-3@example.com"))
            .unwrap()
            .state,
        CallState::Ringing
    );
}

#[test]
fn outbound_success_response_establishes_call() {
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let manager = routed_manager_with_call("call-4@example.com", tx);
    let response = outbound_response(200, "OK", "call-4@example.com");

    let outcome = manager
        .handle_outbound_response(&response)
        .expect("response should update call");

    assert_eq!(outcome.state, CallState::Established);
    assert_eq!(
        manager
            .get(&CallId::new("call-4@example.com"))
            .unwrap()
            .state,
        CallState::Established
    );
}

#[test]
fn outbound_failure_response_fails_call() {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let manager = routed_manager_with_call("call-5@example.com", tx);
    let response = outbound_response(486, "Busy Here", "call-5@example.com");

    let outcome = manager
        .handle_outbound_response(&response)
        .expect("response should update call");

    let call = manager.get(&CallId::new("call-5@example.com")).unwrap();
    assert_eq!(outcome.state, CallState::Failed);
    assert_eq!(call.state, CallState::Failed);
    assert_eq!(call.failure_cause.as_ref().unwrap().status_code, Some(486));

    let cdr = rx.try_recv().expect("CDR should exist");
    assert_eq!(cdr.status, CdrStatus::Failed);
    assert_eq!(cdr.failure_cause.as_ref().unwrap().status_code, Some(486));
}

#[test]
fn inbound_termination_request_terminates_call() {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let manager = routed_manager_with_call("call-6@example.com", tx);
    let response = outbound_response(200, "OK", "call-6@example.com");
    manager.handle_outbound_response(&response).unwrap();
    let bye = bye_request("call-6@example.com");

    let outcome = manager
        .handle_inbound_termination(&bye, None, None)
        .expect("BYE should terminate call");

    assert_eq!(outcome.state, CallState::Terminated);
    assert_eq!(
        manager
            .get(&CallId::new("call-6@example.com"))
            .unwrap()
            .state,
        CallState::Terminated
    );

    let cdr = rx.try_recv().expect("CDR should exist");
    assert_eq!(cdr.status, CdrStatus::Answered);
    assert!(cdr.answered_at.is_some());
    assert!(cdr.duration >= cdr.billable_duration);
}

#[test]
fn inbound_cancel_before_answer_generates_canceled_cdr() {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let manager = routed_manager_with_call("call-7@example.com", tx);
    let cancel = cancel_request("call-7@example.com");

    let outcome = manager
        .handle_inbound_termination(&cancel, None, None)
        .expect("CANCEL should terminate call");

    assert_eq!(outcome.state, CallState::Terminated);
    let cdr = rx.try_recv().expect("CDR should exist");
    assert_eq!(cdr.status, CdrStatus::Canceled);
    assert!(cdr.answered_at.is_none());
    assert!(cdr.billable_duration.is_zero());
}

#[test]
fn watchdog_termination_after_answer_generates_answered_cdr() {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let manager = routed_manager_with_call("call-watchdog-answered@example.com", tx);
    let response = outbound_response(200, "OK", "call-watchdog-answered@example.com");
    manager
        .handle_outbound_response(&response)
        .expect("response should establish call");

    manager.terminate_call_with_reason(
        "call-watchdog-answered@example.com",
        "session timer expired",
    );

    let call = manager
        .get(&CallId::new("call-watchdog-answered@example.com"))
        .expect("call should remain available for inspection");
    assert_eq!(call.state, CallState::Terminated);
    let cdr = rx.try_recv().expect("CDR should exist");
    assert_eq!(cdr.status, CdrStatus::Answered);
    assert!(cdr.answered_at.is_some());
}

#[test]
fn watchdog_termination_before_answer_generates_failed_cdr() {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let manager = routed_manager_with_call("call-watchdog-failed@example.com", tx);

    manager.terminate_call_with_reason("call-watchdog-failed@example.com", "session timer expired");

    let call = manager
        .get(&CallId::new("call-watchdog-failed@example.com"))
        .expect("call should remain available for inspection");
    assert_eq!(call.state, CallState::Failed);
    let cdr = rx.try_recv().expect("CDR should exist");
    assert_eq!(cdr.status, CdrStatus::Failed);
    assert_eq!(
        cdr.failure_cause
            .as_ref()
            .map(|cause| cause.reason.as_str()),
        Some("session timer expired")
    );
}

#[test]
fn failover_outcome_distinguishes_failed_and_replacement_gateways() {
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let routes = RouteTable::new(vec![
        Route::new(
            "primary",
            "",
            200,
            RouteTarget::new("gw-primary", "primary.example.com", Some(5060)),
        ),
        Route::new(
            "backup",
            "",
            100,
            RouteTarget::new("gw-backup", "backup.example.com", Some(5060)),
        ),
    ]);
    let manager = CallManager::new(routes, tx);
    let call_id = "call-failover@example.com";
    manager
        .handle_inbound_invite(&invite_request(call_id, "13800138000"))
        .expect("call should select the primary gateway");

    let outcome = manager
        .handle_outbound_response(&outbound_response(503, "Service Unavailable", call_id))
        .expect("retryable response should select the backup gateway");

    assert_eq!(outcome.gateway_id, "gw-primary");
    assert_eq!(outcome.failover_gateway_id.as_deref(), Some("gw-backup"));
    assert_eq!(
        outcome.failover_uri.as_ref().map(|uri| uri.host.as_ref()),
        Some("backup.example.com")
    );
}

fn test_routes() -> RouteTable {
    RouteTable::new(vec![Route::new(
        "default",
        "",
        100,
        RouteTarget::new("gw1", "gw1.example.com", Some(5060)),
    )])
}

fn routed_manager_with_call(
    call_id: &str,
    tx: tokio::sync::mpsc::UnboundedSender<call_core::CallCdr>,
) -> CallManager {
    let manager = CallManager::new(test_routes(), tx);
    let request = invite_request(call_id, "13800138000");
    manager.handle_inbound_invite(&request).unwrap();
    manager
}

fn invite_request(call_id: &str, destination: &str) -> sip_core::SipRequest {
    let raw = format!(
        concat!(
            "INVITE sip:{destination}@example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-1\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:{destination}@example.com>\r\n",
            "Call-ID: {call_id}\r\n",
            "CSeq: 1 INVITE\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        ),
        call_id = call_id,
        destination = destination
    );

    let sip_core::SipMessageBorrow::Request(request) = parse_message(raw.as_bytes()).unwrap()
    else {
        panic!("expected request");
    };
    request.into_owned()
}

fn outbound_response(
    status_code: u16,
    reason_phrase: &str,
    call_id: &str,
) -> sip_core::SipResponse {
    let raw = format!(
        concat!(
            "SIP/2.0 {status_code} {reason_phrase}\r\n",
            "Via: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-vosrs\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:13800138000@example.com>;tag=gw-tag\r\n",
            "Call-ID: {call_id}\r\n",
            "CSeq: 1 INVITE\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        ),
        status_code = status_code,
        reason_phrase = reason_phrase,
        call_id = call_id
    );

    let sip_core::SipMessageBorrow::Response(response) = parse_message(raw.as_bytes()).unwrap()
    else {
        panic!("expected response");
    };
    response.into_owned()
}

fn bye_request(call_id: &str) -> sip_core::SipRequest {
    let raw = format!(
        concat!(
            "BYE sip:13800138000@example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-bye\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:13800138000@example.com>;tag=gw-tag\r\n",
            "Call-ID: {call_id}\r\n",
            "CSeq: 2 BYE\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        ),
        call_id = call_id
    );

    let sip_core::SipMessageBorrow::Request(request) = parse_message(raw.as_bytes()).unwrap()
    else {
        panic!("expected request");
    };
    request.into_owned()
}

fn cancel_request(call_id: &str) -> sip_core::SipRequest {
    let raw = format!(
        concat!(
            "CANCEL sip:13800138000@example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-cancel\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:13800138000@example.com>\r\n",
            "Call-ID: {call_id}\r\n",
            "CSeq: 1 CANCEL\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        ),
        call_id = call_id
    );

    let sip_core::SipMessageBorrow::Request(request) = parse_message(raw.as_bytes()).unwrap()
    else {
        panic!("expected request");
    };
    request.into_owned()
}
