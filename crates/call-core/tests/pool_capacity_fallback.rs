use call_core::{
    CallManager, CallSource, CallerPoolStrategy, OutboundPolicyDirectory, Route, RouteTable,
    RouteTarget, RuntimeCallerPool, RuntimeCallerPoolMember, RuntimeEgressGroupMember,
    RuntimeEgressPolicy, RuntimeSourcePolicy,
};
use sip_core::{parse_message, SipMessage};

#[test]
fn advancing_pool_switches_number_owner_and_route_together() {
    let routes = RouteTable::new(vec![
        Route::new(
            "route-a",
            "",
            100,
            RouteTarget::new("egress-a", "a.test", Some(5060)),
        ),
        Route::new(
            "route-b",
            "",
            100,
            RouteTarget::new("egress-b", "b.test", Some(5060)),
        ),
    ]);
    let (cdr_sender, _cdr_receiver) = tokio::sync::mpsc::unbounded_channel();
    let manager = CallManager::new(routes, cdr_sender);
    let source = CallSource::new("trunk", "access-a");
    manager.update_outbound_policies(directory(source.clone()));

    let first = manager
        .handle_inbound_invite_with_source_and_health(&invite(), Some(&source), None)
        .expect("initial pool member should resolve");
    let first_identity = first
        .caller_identity
        .expect("pool resolves caller identity");
    assert_eq!(first_identity.presented_number, "10001");
    assert_eq!(first_identity.owner_gateway_id.as_str(), "egress-a");
    assert_eq!(
        manager
            .current_gateway_id(first.call_id.as_str())
            .as_deref(),
        Some("egress-a")
    );

    let second = manager
        .advance_caller_pool(&first.call_id)
        .expect("second member should be available");
    let second_identity = second
        .caller_identity
        .expect("fallback keeps caller identity");
    assert_eq!(second_identity.presented_number, "10002");
    assert_eq!(second_identity.owner_gateway_id.as_str(), "egress-b");
    assert_eq!(
        manager
            .current_gateway_id(second.call_id.as_str())
            .as_deref(),
        Some("egress-b")
    );
    assert!(manager.advance_caller_pool(&second.call_id).is_none());
}

fn directory(source: CallSource) -> OutboundPolicyDirectory {
    let allocations = ["10001", "10002"]
        .into_iter()
        .map(|number| (number.to_string(), source.clone()))
        .collect::<Vec<_>>();
    OutboundPolicyDirectory::new(
        [
            ("10001".to_string(), "egress-a".to_string(), 1),
            ("10002".to_string(), "egress-b".to_string(), 1),
        ],
        allocations,
        [RuntimeSourcePolicy {
            source: source.clone(),
            caller_mode: "virtual_pool".to_string(),
            fixed_number: None,
            caller_pool_id: Some("pool-a".to_string()),
            egress: RuntimeEgressPolicy::Group("group-a".to_string()),
        }],
        [RuntimeCallerPool {
            id: "pool-a".to_string(),
            owner: source,
            strategy: CallerPoolStrategy::Priority,
            members: ["10001", "10002"]
                .into_iter()
                .map(|number| RuntimeCallerPoolMember {
                    number: number.to_string(),
                    priority: 100,
                    weight: 1,
                    max_concurrent: 1,
                })
                .collect(),
        }],
        [
            RuntimeEgressGroupMember {
                group_id: "group-a".to_string(),
                gateway_id: "egress-a".to_string(),
                destination_prefix: String::new(),
            },
            RuntimeEgressGroupMember {
                group_id: "group-a".to_string(),
                gateway_id: "egress-b".to_string(),
                destination_prefix: String::new(),
            },
        ],
    )
}

fn invite() -> sip_core::SipRequest {
    let raw = concat!(
        "INVITE sip:callee@example.com SIP/2.0\r\n",
        "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-pool\r\n",
        "From: <sip:original@example.com>;tag=from-tag\r\n",
        "To: <sip:callee@example.com>\r\n",
        "Call-ID: pool-fallback@example.com\r\n",
        "CSeq: 1 INVITE\r\n",
        "Content-Length: 0\r\n",
        "\r\n"
    );
    let SipMessage::Request(request) = parse_message(raw.as_bytes()).expect("valid INVITE") else {
        panic!("expected request");
    };
    request
}
