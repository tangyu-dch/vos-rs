//! command_listener 模块单元测试。

use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;

use call_core::{CallId, CallManager, CallState, CdrStatus, RouteTable};
use sip_core::{parse_message, SipMessage, SipUri};

use crate::config::EdgeConfig;
use crate::edge_state::{EdgeState, ParkedCall};

use super::hangup_handler::finalize_vci_hangup;
use super::{handle_command, CallCommand, CommandAction, DialParams, HangupParams};

async fn make_test_state() -> Arc<EdgeState> {
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let cm = CallManager::new(RouteTable::default(), tx);
    let config = EdgeConfig::default();
    let state =
        EdgeState::with_media_relay_and_db(cm, crate::media::MediaRelayState::new(), None, &config);
    let socket = std::sync::Arc::new(tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap());
    state.set_socket(socket);
    Arc::new(state)
}

fn make_parked_request() -> sip_core::SipRequest {
    sip_core::SipRequest {
        method: sip_core::Method::Invite,
        uri: SipUri {
            secure: false,
            user: Some("1001".into()),
            host: "192.168.1.1".into(),
            port: Some(5060),
            params: Vec::new(),
        },
        version: std::borrow::Cow::Borrowed("SIP/2.0"),
        headers: sip_core::HeaderMap::new(),
        body: std::borrow::Cow::Borrowed(&[]),
    }
}

#[tokio::test]
async fn test_hangup_removes_parked_call_and_sends_response() {
    let state = make_test_state().await;
    let config = Arc::new(EdgeConfig::default());

    let request = make_parked_request();
    let peer: SocketAddr = "192.168.1.100:5060".parse().unwrap();

    state.parked_calls.insert(
        "test-call-1".to_string(),
        ParkedCall {
            session_id: "test-session-1".to_string(),
            invite_request: request,
            peer_addr: peer,
            caller_relay_port: 40000,
        },
    );

    assert!(state.parked_calls.contains_key("test-call-1"));

    let cmd = CallCommand {
        call_id: "test-call-1".to_string(),
        action: CommandAction::Hangup {
            params: HangupParams {
                sip_cause: Some(603),
            },
        },
    };

    handle_command(cmd, &state, &config).await;

    assert!(!state.parked_calls.contains_key("test-call-1"));
}

#[tokio::test]
async fn test_hangup_parked_call_with_default_cause() {
    let state = make_test_state().await;
    let config = Arc::new(EdgeConfig::default());

    let request = make_parked_request();
    let peer: SocketAddr = "192.168.1.101:5060".parse().unwrap();

    state.parked_calls.insert(
        "test-call-2".to_string(),
        ParkedCall {
            session_id: "test-session-2".to_string(),
            invite_request: request,
            peer_addr: peer,
            caller_relay_port: 40001,
        },
    );

    let cmd = CallCommand {
        call_id: "test-call-2".to_string(),
        action: CommandAction::Hangup {
            params: HangupParams { sip_cause: None },
        },
    };

    handle_command(cmd, &state, &config).await;

    assert!(!state.parked_calls.contains_key("test-call-2"));
}

#[test]
fn test_hangup_finalizes_managed_call_only_once() {
    let (cdr_tx, mut cdr_rx) = tokio::sync::mpsc::unbounded_channel();
    let manager = CallManager::new(RouteTable::default(), cdr_tx);
    let config = Arc::new(EdgeConfig::default());
    let state = EdgeState::with_media_relay_and_db(
        manager,
        crate::media::MediaRelayState::new(),
        None,
        &config,
    );
    let state = Arc::new(state);

    let raw_invite = b"INVITE sip:1002@example.com SIP/2.0\r\n\
Via: SIP/2.0/UDP 127.0.0.1:5060;branch=z9hG4bK-vci-finalize\r\n\
From: <sip:1001@example.com>;tag=from-vci\r\n\
To: <sip:1002@example.com>\r\n\
Call-ID: vci-finalize@example.com\r\n\
CSeq: 1 INVITE\r\n\
Content-Length: 0\r\n\r\n";
    let SipMessage::Request(invite) = parse_message(raw_invite).unwrap() else {
        panic!("expected INVITE request");
    };
    state
        .call_manager
        .handle_inbound_invite_to_uri(
            &invite,
            SipUri::from_str("sip:1002@gateway.example.com:5060").unwrap(),
        )
        .unwrap();

    finalize_vci_hangup(&state, "vci-finalize@example.com", "VCI Hangup (487)");

    let call_id = CallId::new("vci-finalize@example.com");
    assert_eq!(
        state.call_manager.get(&call_id).map(|call| call.state),
        Some(CallState::Failed)
    );
    let cdr = cdr_rx.try_recv().expect("VCI Hangup should emit a CDR");
    assert_eq!(cdr.status, CdrStatus::Failed);
    assert_eq!(
        cdr.failure_cause
            .as_ref()
            .map(|cause| cause.reason.as_str()),
        Some("VCI Hangup (487)")
    );

    finalize_vci_hangup(&state, "vci-finalize@example.com", "VCI Hangup (487)");
    assert!(cdr_rx.try_recv().is_err(), "duplicate Hangup emitted a CDR");
}

#[tokio::test]
async fn test_dial_missing_parked_call_returns_early() {
    let state = make_test_state().await;
    let config = Arc::new(EdgeConfig::default());

    let cmd = CallCommand {
        call_id: "nonexistent".to_string(),
        action: CommandAction::Dial {
            params: DialParams {
                target_gateway: None,
                target_uri: None,
                caller_id: None,
                timeout_secs: None,
            },
        },
    };

    handle_command(cmd, &state, &config).await;

    assert!(!state.parked_calls.contains_key("nonexistent"));
}

#[test]
fn test_call_command_deserialize_dial() {
    let json = r#"{"call_id":"abc","action":"dial","target_gateway":"gw1","caller_id":"1001"}"#;
    let cmd: CallCommand = serde_json::from_str(json).unwrap();
    assert_eq!(cmd.call_id, "abc");
    match cmd.action {
        CommandAction::Dial { params } => {
            assert_eq!(params.target_gateway.as_deref(), Some("gw1"));
            assert_eq!(params.caller_id.as_deref(), Some("1001"));
        }
        _ => panic!("expected Dial"),
    }
}

#[test]
fn test_call_command_deserialize_hangup() {
    let json = r#"{"call_id":"xyz","action":"hangup","sip_cause":486}"#;
    let cmd: CallCommand = serde_json::from_str(json).unwrap();
    assert_eq!(cmd.call_id, "xyz");
    match cmd.action {
        CommandAction::Hangup { params } => {
            assert_eq!(params.sip_cause, Some(486));
        }
        _ => panic!("expected Hangup"),
    }
}

#[test]
fn test_call_command_deserialize_play() {
    let json = r#"{"call_id":"p1","action":"play","url":"/audio/welcome.wav","loop_count":2}"#;
    let cmd: CallCommand = serde_json::from_str(json).unwrap();
    match cmd.action {
        CommandAction::Play { params } => {
            assert_eq!(params.url, "/audio/welcome.wav");
            assert_eq!(params.loop_count, Some(2));
        }
        _ => panic!("expected Play"),
    }
}

#[test]
fn test_call_command_deserialize_gather() {
    let json = r#"{"call_id":"g1","action":"gather","play_url":"/audio/prompt.wav","max_digits":4,"timeout_ms":5000}"#;
    let cmd: CallCommand = serde_json::from_str(json).unwrap();
    match cmd.action {
        CommandAction::Gather { params } => {
            assert_eq!(params.max_digits, 4);
            assert_eq!(params.timeout_ms, 5000);
            assert_eq!(params.play_url.as_deref(), Some("/audio/prompt.wav"));
        }
        _ => panic!("expected Gather"),
    }
}
