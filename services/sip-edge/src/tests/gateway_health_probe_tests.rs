#[tokio::test]
async fn gateway_options_response_updates_probe_health() {
    let (cdr_tx, _cdr_rx) = tokio::sync::mpsc::unbounded_channel();
    let call_manager = CallManager::new(
        RouteTable::new(vec![Route::new(
            "gw1-route",
            "",
            100,
            RouteTarget::new("gw1", "127.0.0.1", Some(5060)),
        )]),
        cdr_tx,
    );
    let edge_state = EdgeState::new(call_manager);
    let config = EdgeConfig::from_env();
    let call_id = "health-probe-gw1-test";
    edge_state
        .gateway_probes
        .insert(call_id.to_string(), "gw1".to_string());

    let response = format!(
        "SIP/2.0 200 OK\r\n\
         Via: SIP/2.0/UDP 127.0.0.1:5060;branch=z9hG4bK-health-1\r\n\
         From: <sip:health-check@127.0.0.1>;tag=health-1\r\n\
         To: <sip:health-check@127.0.0.1>;tag=gw\r\n\
         Call-ID: {call_id}\r\n\
         CSeq: 1 OPTIONS\r\n\
         Content-Length: 0\r\n\r\n"
    );

    let datagrams = handle_datagram(
        response.as_bytes(),
        "127.0.0.1:5060".parse().unwrap(),
        &edge_state,
        &config,
    )
    .await;

    assert!(datagrams.is_empty());
    assert!(!edge_state.gateway_probes.contains_key(call_id));
    let health = edge_state.gateway_health.lock().unwrap();
    let status = health.get_gateway_status("gw1");
    assert!(status.is_some());
    let (open, failures, _, _, _) = status.unwrap();
    assert!(!open);
    assert_eq!(failures, 0);
}
