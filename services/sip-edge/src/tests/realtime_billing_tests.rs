use tokio::time::sleep;

#[tokio::test]
async fn test_balance_exhaustion_disconnect_flow() {
    let edge_state = std::sync::Arc::new(state_with_default_route());
    
    // Bind mock socket for the test state
    let test_socket = std::sync::Arc::new(tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap());
    edge_state.set_socket(test_socket.clone());

    // 1. Send inbound INVITE from caller 1001 with test max duration limit = 1 second.
    let call_id = "billing-limit-call@example.com";
    let invite = format!(
        "INVITE sip:13800000000@edge.example.com SIP/2.0\r\n\
         Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-inv-bill\r\n\
         Max-Forwards: 70\r\n\
         From: <sip:1001@example.com>;tag=from-tag\r\n\
         To: <sip:13800000000@edge.example.com>\r\n\
         Call-ID: {call_id}\r\n\
         CSeq: 1 INVITE\r\n\
         X-Test-Max-Duration: 1\r\n\
         Content-Length: 0\r\n\r\n",
        call_id = call_id
    );

    let datagrams = handle_datagram(
        invite.as_bytes(),
        peer(),
        &edge_state,
        &edge_config(),
    )
    .await;

    // Should route outbound invite to gateway gw1
    assert!(!datagrams.is_empty());

    // Extract the dynamically generated external Call-ID
    let outbound_invite_txt = String::from_utf8_lossy(&datagrams[0].bytes);
    let external_call_id = outbound_invite_txt
        .lines()
        .find(|l| l.starts_with("Call-ID:"))
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .to_string();

    // 2. Send 200 OK back from gateway to answer the call
    let ok_200 = format!(
        "SIP/2.0 200 OK\r\n\
         Via: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-vosrs\r\n\
         From: <sip:1001@example.com>;tag=from-tag\r\n\
         To: <sip:13800000000@edge.example.com>;tag=gw-tag\r\n\
         Call-ID: {external_call_id}\r\n\
         CSeq: 1 INVITE\r\n\
         Content-Length: 0\r\n\r\n",
        external_call_id = external_call_id
    );

    let _ = handle_datagram(
        ok_200.as_bytes(),
        "198.51.100.20:5060".parse().unwrap(),
        &edge_state,
        &edge_config(),
    )
    .await;

    // Verify call is established and max_duration_secs is set to 1s
    {
        let tx = edge_state.inbound_transactions.get(call_id).unwrap();
        assert_eq!(tx.max_duration_secs, Some(1));
        assert!(tx.established_at.is_some());
    }

    // 3. Start the watchdog timer loop in background to check expiration (simulate elapse of 1s)
    let edge_config_arc = std::sync::Arc::new(edge_config());

    // Wait 1.1s for balance to exhaust
    sleep(Duration::from_millis(1100)).await;

    // Trigger one watchdog pass manually by invoking the logic or running the loop.
    spawn_session_timer_watchdog(edge_state.clone(), test_socket, edge_config_arc);

    // Wait a brief moment for the timer loop to execute its check pass
    sleep(Duration::from_millis(100)).await;

    // Verify transaction has been cleaned up due to balance disconnect
    assert!(!edge_state.inbound_transactions.contains_key(call_id));
}
