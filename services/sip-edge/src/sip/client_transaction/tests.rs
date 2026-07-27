use sip_core::{parse_message, SipMessage};

use super::manager::ClientTransactionManager;
use super::state::{
    ClientTransactionControl, ClientTransactionMachine, ClientTransactionState, ResponseAction,
};
use crate::sip::transaction::ClientTransactionKey;

#[test]
fn invite_provisional_responses_stop_timer_a() {
    for status_code in [100, 180, 183] {
        let mut machine = ClientTransactionMachine::new("INVITE");
        assert!(machine.should_retransmit());
        assert_eq!(
            machine.on_response(status_code),
            ResponseAction::StopRetransmissions
        );
        assert_eq!(machine.state(), ClientTransactionState::Proceeding);
        assert!(!machine.should_retransmit());
    }
}

#[test]
fn invite_provisional_response_disables_timer_b() {
    let control = ClientTransactionControl::new("INVITE");

    assert!(control.should_timeout());
    assert_eq!(
        control.on_response(180),
        ResponseAction::StopRetransmissions
    );
    assert!(!control.should_timeout());
    assert!(!control.should_retransmit());
}

#[test]
fn repeated_invite_provisional_response_is_idempotent() {
    let control = ClientTransactionControl::new("INVITE");

    assert_eq!(
        control.on_response(180),
        ResponseAction::StopRetransmissions
    );
    assert_eq!(control.on_response(180), ResponseAction::Ignore);
    assert_eq!(control.state(), ClientTransactionState::Proceeding);
}

#[test]
fn invite_final_response_completes_transaction() {
    let mut machine = ClientTransactionMachine::new("INVITE");
    assert_eq!(machine.on_response(486), ResponseAction::Complete);
    assert_eq!(machine.state(), ClientTransactionState::Completed);
    assert!(!machine.should_retransmit());
}

#[test]
fn manager_applies_response_by_via_branch() {
    let manager = ClientTransactionManager::new();
    let key = client_key("external-call", "z9hG4bK-outbound");
    let registration = manager.register(key).expect("transaction should register");
    let response = response("different-call-id", "z9hG4bK-outbound", 180);

    assert_eq!(manager.observe_response(&response), 1);
    assert_eq!(
        registration.control.state(),
        ClientTransactionState::Proceeding
    );
    assert!(!registration.control.should_retransmit());
}

#[test]
fn response_before_registration_is_replayed_to_transaction() {
    let manager = ClientTransactionManager::new();
    let response = response("external-call", "z9hG4bK-early", 100);
    assert_eq!(manager.observe_response(&response), 0);

    let registration = manager
        .register(client_key("external-call", "z9hG4bK-early"))
        .expect("transaction should register");
    assert_eq!(
        registration.control.state(),
        ClientTransactionState::Proceeding
    );
    assert!(!registration.control.should_retransmit());
}

#[test]
fn forked_branches_are_isolated() {
    let manager = ClientTransactionManager::new();
    let first_key = client_key("shared-call", "z9hG4bK-first");
    let second_key = client_key("shared-call", "z9hG4bK-second");
    let first = manager
        .register(first_key)
        .expect("first transaction should register");
    let second = manager
        .register(second_key)
        .expect("second transaction should register");
    let response = response("shared-call", "z9hG4bK-first", 180);

    assert_eq!(manager.observe_response(&response), 1);
    assert_eq!(first.control.state(), ClientTransactionState::Proceeding);
    assert_eq!(second.control.state(), ClientTransactionState::Calling);
    assert_eq!(manager.active_len(), 2);
}

#[test]
fn duplicate_transaction_registration_is_rejected() {
    let manager = ClientTransactionManager::new();
    let key = client_key("same-call", "z9hG4bK-same");
    assert!(manager.register(key.clone()).is_some());
    assert!(manager.register(key).is_none());
    assert_eq!(manager.active_len(), 1);
}

#[test]
fn unique_call_id_and_cseq_fallback_stops_invite_when_branch_differs() {
    let manager = ClientTransactionManager::new();
    let key = client_key("fallback-call", "z9hG4bK-request-branch");
    let registration = manager.register(key).expect("transaction should register");
    let response = response("fallback-call", "z9hG4bK-response-branch", 180);

    assert_eq!(manager.observe_response(&response), 1);
    assert_eq!(
        registration.control.state(),
        ClientTransactionState::Proceeding
    );
    assert!(!registration.control.should_retransmit());
}

#[test]
fn ambiguous_call_id_and_cseq_fallback_does_not_cross_forked_legs() {
    let manager = ClientTransactionManager::new();
    let first = manager
        .register(client_key("fork-call", "z9hG4bK-first"))
        .expect("first transaction should register");
    let second = manager
        .register(client_key("fork-call", "z9hG4bK-second"))
        .expect("second transaction should register");
    let response = response("fork-call", "z9hG4bK-unknown", 180);

    assert_eq!(manager.observe_response(&response), 0);
    assert_eq!(first.control.state(), ClientTransactionState::Calling);
    assert_eq!(second.control.state(), ClientTransactionState::Calling);
}

#[test]
fn captured_telephone_response_matches_registered_transaction() {
    let manager = ClientTransactionManager::new();
    let request = concat!(
        "INVITE sip:1001@127.0.0.1:59936;ob SIP/2.0\r\n",
        "Via: SIP/2.0/UDP 127.0.0.1:5060;branch=z9hG4bK-vosrs-4ruzdkqfpawnw6bqrrl6hp4qfcttmmuf-5121-invite\r\n",
        "Call-ID: 48f94a34-eb2f-499f-93ba-a72e8dbe811a\r\n",
        "CSeq: 5121 INVITE\r\n",
        "Content-Length: 0\r\n\r\n",
    );
    let SipMessage::Request(request) =
        parse_message(request.as_bytes()).expect("captured request should parse")
    else {
        panic!("expected request");
    };
    let key = ClientTransactionKey::from_request(&request).expect("request should have a key");
    let registration = manager
        .register(key)
        .expect("captured transaction should register");
    let raw_response = concat!(
        "SIP/2.0 180 Ringing\r\n",
        "Via: SIP/2.0/UDP 127.0.0.1:5060;received=127.0.0.1;branch=z9hG4bK-vosrs-4ruzdkqfpawnw6bqrrl6hp4qfcttmmuf-5121-invite\r\n",
        "Call-ID: 48f94a34-eb2f-499f-93ba-a72e8dbe811a\r\n",
        "CSeq: 5121 INVITE\r\n",
        "Content-Length: 0\r\n\r\n",
    );
    let SipMessage::Response(response) =
        parse_message(raw_response.as_bytes()).expect("captured response should parse")
    else {
        panic!("expected response");
    };

    assert_eq!(manager.observe_response(&response.into_owned()), 1);
    assert_eq!(
        registration.control.state(),
        ClientTransactionState::Proceeding
    );
    assert!(!registration.control.should_retransmit());
}

#[test]
fn transport_ingress_applies_captured_telephone_response() {
    let manager = ClientTransactionManager::new();
    let key = client_key(
        "e96f2254-fe7b-485f-a82b-228f472695ad",
        "z9hG4bK-vosrs-zjcwewfwwdbzzjiqdtpbpnzwqhchlk7g-6516-invite",
    );
    let registration = manager.register(ClientTransactionKey {
        cseq: "6516".to_string(),
        ..key
    });
    let registration = registration.expect("transaction should register");
    let response = concat!(
        "SIP/2.0 180 Ringing\r\n",
        "Via: SIP/2.0/UDP 127.0.0.1:5060;received=127.0.0.1;branch=z9hG4bK-vosrs-zjcwewfwwdbzzjiqdtpbpnzwqhchlk7g-6516-invite\r\n",
        "Call-ID: e96f2254-fe7b-485f-a82b-228f472695ad\r\n",
        "From: \"1002\" <sip:1002@vos-rs>;tag=caller\r\n",
        "To: <sip:1001@vos-rs>;tag=callee\r\n",
        "CSeq: 6516 INVITE\r\n",
        "Content-Length: 0\r\n\r\n",
    );

    assert_eq!(manager.observe_packet(response.as_bytes()), 1);
    assert_eq!(
        registration.control.state(),
        ClientTransactionState::Proceeding
    );
    assert!(!registration.control.should_retransmit());
}

#[test]
fn branch_response_ledger_stops_invite_without_active_index_delivery() {
    let manager = ClientTransactionManager::new();
    let key = client_key("ledger-call", "z9hG4bK-ledger");
    let response = response("different-call", "z9hG4bK-ledger", 180);

    assert_eq!(manager.observe_response(&response), 0);
    let control = ClientTransactionControl::new("INVITE");
    assert!(manager.apply_observed_branch_response(&key, &control));
    assert_eq!(control.state(), ClientTransactionState::Proceeding);
    assert!(!control.should_retransmit());
}

#[tokio::test]
async fn response_state_is_visible_before_async_notification_runs() {
    let control = ClientTransactionControl::new("INVITE");
    assert_eq!(
        control.on_response(180),
        ResponseAction::StopRetransmissions
    );
    assert!(!control.should_retransmit());
    assert_eq!(control.state(), ClientTransactionState::Proceeding);
}

fn client_key(call_id: &str, branch: &str) -> ClientTransactionKey {
    ClientTransactionKey {
        call_id: call_id.to_string(),
        cseq: "928".to_string(),
        method: "INVITE".to_string(),
        branch: branch.to_string(),
    }
}

fn response(call_id: &str, branch: &str, status_code: u16) -> sip_core::SipResponse {
    let raw = format!(
        "SIP/2.0 {status_code} Test\r\n\
         Via: SIP/2.0/UDP 127.0.0.1:5060;rport;branch={branch}\r\n\
         From: <sip:1002@vos-rs>;tag=caller\r\n\
         To: <sip:1001@vos-rs>;tag=callee\r\n\
         Call-ID: {call_id}\r\n\
         CSeq: 928 INVITE\r\n\
         Content-Length: 0\r\n\r\n"
    );
    let SipMessage::Response(response) = parse_message(raw.as_bytes())
        .expect("response should parse")
        .into_owned()
    else {
        panic!("expected response");
    };
    response
}
