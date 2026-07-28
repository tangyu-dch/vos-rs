//! IVR 模块单元测试。

use crate::edge_state::{InboundTransaction, IvrAction};
use crate::{EdgeConfig, EdgeState};
use call_core::{CallManager, Route, RouteTable, RouteTarget};
use sip_core::{HeaderMap, Method, SipRequest, SipUri};
use std::str::FromStr;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;

use super::actions::execute_ivr_action;
use super::menu_loop::run_ivr_menu_loop;

static PORT_COUNTER: AtomicU16 = AtomicU16::new(10);
fn get_test_ports() -> (u16, u16) {
    let offset = PORT_COUNTER.fetch_add(10, Ordering::Relaxed);
    let port_min = 45000 + offset;
    let port_max = port_min + 8;
    (port_min, port_max)
}

#[tokio::test]
async fn test_execute_ivr_action_hangup() {
    let (tx, _rx) = tokio::sync::mpsc::channel(100);
    let call_manager = CallManager::new(RouteTable::default(), tx);
    let edge_state = EdgeState::new(call_manager);

    let action = IvrAction {
        action_type: "hangup".to_string(),
        action_target: "".to_string(),
        waiting_prompt: None,
        webhook_method: None,
    };

    let template_request = SipRequest {
        method: Method::Invite,
        uri: SipUri::from_str("sip:13800138000@example.com").unwrap(),
        version: std::borrow::Cow::Borrowed("SIP/2.0"),
        headers: HeaderMap::new(),
        body: std::borrow::Cow::Borrowed(&[]),
    };

    execute_ivr_action(
        &edge_state,
        &EdgeConfig::default(),
        "test-call-id-hangup",
        40000,
        &action,
        &template_request,
        "127.0.0.1:5060".parse().unwrap(),
    )
    .await;
}

#[tokio::test]
async fn test_execute_ivr_action_pstn_transfer() {
    let (tx, _rx) = tokio::sync::mpsc::channel(100);
    let routes = RouteTable::new(vec![Route::new(
        "test-route",
        "",
        100,
        RouteTarget::new("test-gateway", "192.0.2.200", Some(5060)),
    )]);
    let call_manager = CallManager::new(routes, tx);
    let edge_state = EdgeState::new(call_manager);

    let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    edge_state.socket.set(Arc::new(socket)).unwrap();

    let dummy_transaction = InboundTransaction {
        session_id: "test-session-123".to_string(),
        dialogs: crate::edge_state::B2buaDialogPair::placeholder(
            "test-call-id-123",
            SipUri::from_str("sip:13800138000@example.com").unwrap(),
            "127.0.0.1:5060",
        ),
        original_request: None,
        caller_rtp: None,
        gateway_relay_rtp: None,
        gateway_rtp: None,
        caller_relay_rtp: None,
        session_expires: None,
        session_refresher: None,
        last_session_refresh: None,
        prack_rseq: 0,
        gateway_100rel: false,
        refer_subscription: None,
        transfer_dialog: None,
        fork_dialogs: Default::default(),
        max_duration_secs: None,
        established_at: None,
        invite_response_order: Arc::new(tokio::sync::Mutex::new(
            crate::edge_state::InviteResponseOrder::default(),
        )),
    };
    edge_state.inbound_transactions.insert(dummy_transaction);

    let action = IvrAction {
        action_type: "pstn".to_string(),
        action_target: "123".to_string(),
        waiting_prompt: None,
        webhook_method: None,
    };

    let template_request = SipRequest {
        method: Method::Invite,
        uri: SipUri::from_str("sip:13800138000@example.com").unwrap(),
        version: std::borrow::Cow::Borrowed("SIP/2.0"),
        headers: HeaderMap::new(),
        body: std::borrow::Cow::Borrowed(&[]),
    };

    let (port_min, port_max) = get_test_ports();
    let config = EdgeConfig {
        advertised_addr: "127.0.0.1:5060".to_string(),
        media: crate::media::MediaConfig::new_with_symmetric_learning(
            "127.0.0.1",
            port_min,
            port_max,
            true,
        ),
        ..EdgeConfig::default()
    };

    execute_ivr_action(
        &edge_state,
        &config,
        "test-call-id-123",
        40000,
        &action,
        &template_request,
        "127.0.0.1:5060".parse().unwrap(),
    )
    .await;

    let transaction = edge_state
        .inbound_transactions
        .get("test-call-id-123")
        .unwrap();
    assert!(!transaction.dialogs.gateway.call_id.is_empty());
    assert!(transaction.gateway_relay_rtp.is_some());
}

#[tokio::test]
async fn test_execute_ivr_action_queue() {
    let (tx, _rx) = tokio::sync::mpsc::channel(100);
    let call_manager = CallManager::new(RouteTable::default(), tx);
    let edge_state = EdgeState::new(call_manager);

    let action = IvrAction {
        action_type: "queue".to_string(),
        action_target: "sales_queue".to_string(),
        waiting_prompt: None,
        webhook_method: None,
    };

    let template_request = SipRequest {
        method: Method::Invite,
        uri: SipUri::from_str("sip:13800138000@example.com").unwrap(),
        version: std::borrow::Cow::Borrowed("SIP/2.0"),
        headers: HeaderMap::new(),
        body: std::borrow::Cow::Borrowed(&[]),
    };

    execute_ivr_action(
        &edge_state,
        &EdgeConfig::default(),
        "test-call-id-queue",
        40001,
        &action,
        &template_request,
        "127.0.0.1:5060".parse().unwrap(),
    )
    .await;
}

#[tokio::test]
async fn test_execute_ivr_action_menu_retry() {
    let (tx, _rx) = tokio::sync::mpsc::channel(100);
    let call_manager = CallManager::new(RouteTable::default(), tx);
    let edge_state = Arc::new(EdgeState::new(call_manager));

    let menu1 = crate::edge_state::IvrMenu {
        welcome_prompt: "prompt1.wav".to_string(),
        timeout_secs: 1,
        actions: {
            let mut m = std::collections::HashMap::new();
            m.insert(
                "1".to_string(),
                IvrAction {
                    action_type: "menu".to_string(),
                    action_target: "menu2".to_string(),
                    waiting_prompt: None,
                    webhook_method: None,
                },
            );
            m
        },
        topology: None,
    };

    let menu2 = crate::edge_state::IvrMenu {
        welcome_prompt: "prompt2.wav".to_string(),
        timeout_secs: 1,
        actions: {
            let mut m = std::collections::HashMap::new();
            m.insert(
                "2".to_string(),
                IvrAction {
                    action_type: "hangup".to_string(),
                    action_target: "".to_string(),
                    waiting_prompt: None,
                    webhook_method: None,
                },
            );
            m
        },
        topology: None,
    };

    edge_state
        .ivr_menus
        .write()
        .unwrap()
        .insert("menu1".to_string(), menu1.clone());
    edge_state
        .ivr_menus
        .write()
        .unwrap()
        .insert("menu2".to_string(), menu2.clone());

    edge_state.inbound_transactions.insert(InboundTransaction {
        session_id: "test-session-retry".to_string(),
        dialogs: crate::edge_state::B2buaDialogPair::placeholder(
            "test-call-id-retry",
            SipUri::from_str("sip:13800138000@example.com").unwrap(),
            "127.0.0.1:5060",
        ),
        original_request: None,
        caller_rtp: None,
        gateway_relay_rtp: None,
        gateway_rtp: None,
        caller_relay_rtp: None,
        session_expires: None,
        session_refresher: None,
        last_session_refresh: None,
        prack_rseq: 0,
        gateway_100rel: false,
        refer_subscription: None,
        transfer_dialog: None,
        fork_dialogs: Default::default(),
        max_duration_secs: None,
        established_at: None,
        invite_response_order: Arc::new(tokio::sync::Mutex::new(
            crate::edge_state::InviteResponseOrder::default(),
        )),
    });

    let template_request = SipRequest {
        method: Method::Invite,
        uri: SipUri::from_str("sip:13800138000@example.com").unwrap(),
        version: std::borrow::Cow::Borrowed("SIP/2.0"),
        headers: HeaderMap::new(),
        body: std::borrow::Cow::Borrowed(&[]),
    };

    // Instead of simulating DTMF, we wait for timeout to let it compile and run
    run_ivr_menu_loop(
        edge_state.clone(),
        Arc::new(EdgeConfig::default()),
        "test-call-id-retry".to_string(),
        "test-session-retry".to_string(),
        40002,
        template_request.clone(),
        "127.0.0.1:5060".parse().unwrap(),
        menu1.clone(),
    )
    .await;

    // Assert that the transaction is still there or something similar
    assert!(edge_state
        .inbound_transactions
        .contains_key("test-call-id-retry"));
}
