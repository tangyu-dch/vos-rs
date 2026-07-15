use std::collections::HashSet;

use super::*;
use crate::discovery::SipNode;

const REQUEST: &[u8] = b"OPTIONS sip:test@example.com SIP/2.0\r\nVia: SIP/2.0/UDP client;branch=z9hG4bK-client\r\nCall-ID: call-1\r\nCSeq: 1 OPTIONS\r\nContent-Length: 0\r\n\r\n";

#[test]
fn test_add_and_remove_router_via_round_trips_packet() {
    let forwarded =
        add_router_via(REQUEST, "router:5060", "UDP", "z9hG4bK-router").expect("add Via");
    assert_eq!(
        top_via_branch(&forwarded).as_deref(),
        Some("z9hG4bK-router")
    );
    assert_eq!(remove_top_via(&forwarded).expect("remove Via"), REQUEST);
}

#[test]
fn test_router_branch_is_stable_for_retransmission() {
    assert_eq!(router_branch(REQUEST, "UDP"), router_branch(REQUEST, "UDP"));
}

#[test]
fn test_dialog_route_release_only_happens_after_terminal_response() {
    use super::transaction::should_release;

    assert!(!should_release("INVITE", 180));
    assert!(!should_release("INVITE", 200));
    assert!(should_release("INVITE", 503));
    assert!(should_release("BYE", 200));
    assert!(should_release("OPTIONS", 200));
}

#[test]
fn test_select_node_is_stable_for_call_id() {
    let nodes = test_nodes();
    assert_eq!(select_node("call-1", &nodes), select_node("call-1", &nodes));
}

#[test]
fn test_uuid_call_ids_cover_both_nodes() {
    let nodes = test_nodes();
    let call_ids = [
        "58d4c1d2-5bd3-4c1b-a0e4-7c30dd331101",
        "bc47848a-a335-419d-923e-05e9b8111302",
        "c25be5aa-e8fb-4bbc-82fe-f6dbd69f1303",
        "22ea8f46-00e0-46d2-8b85-35a2db971304",
    ];
    let selected: HashSet<&str> = call_ids
        .iter()
        .filter_map(|call_id| select_node(call_id, &nodes).map(|node| node.id.as_str()))
        .collect();

    assert_eq!(selected.len(), 2);
}

fn test_nodes() -> Vec<SipNode> {
    vec![
        SipNode {
            id: "sip-edge-a".to_string(),
            address: "127.0.0.1:5061".parse().expect("address"),
        },
        SipNode {
            id: "sip-edge-b".to_string(),
            address: "127.0.0.1:5062".parse().expect("address"),
        },
    ]
}
