    #[tokio::test]
    async fn test_client_transaction_retransmission() {
        let edge_state = Arc::new(state_with_default_route());
        let socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let local_port = socket.local_addr().unwrap().port();
        let target = format!("127.0.0.1:{}", local_port);

        let req_bytes = b"INVITE sip:gw@127.0.0.1:5060 SIP/2.0\r\n\
                          Via: SIP/2.0/UDP 127.0.0.1:5060;branch=z9hG4bK-tx-test\r\n\
                          Call-ID: tx-test@example.com\r\n\
                          CSeq: 1 INVITE\r\n\
                          Content-Length: 0\r\n\r\n";
        let req = parse_message(req_bytes).unwrap();
        let SipMessage::Request(req) = req else {
            panic!("expected request");
        };
        let key = ClientTransactionKey::from_request(&req).unwrap();

        spawn_client_transaction_retransmission(
            Arc::clone(&edge_state),
            Arc::clone(&socket),
            target.clone(),
            req_bytes.to_vec(),
            key.clone(),
            Arc::new(edge_config()),
        );

        tokio::time::sleep(Duration::from_millis(15)).await;

        let resp = parse_message(
            b"SIP/2.0 180 Ringing\r\n\
              Via: SIP/2.0/UDP 127.0.0.1:5060;branch=z9hG4bK-tx-test\r\n\
              Call-ID: tx-test@example.com\r\n\
              CSeq: 1 INVITE\r\n\
              Content-Length: 0\r\n\r\n",
        )
        .unwrap();
        let SipMessage::Response(resp) = resp else {
            panic!("expected response");
        };
        let resp_key = ClientTransactionKey::from_response(&resp).unwrap();

        edge_state.cancel_client_transaction(&resp_key);

        tokio::time::sleep(Duration::from_millis(5)).await;
        assert!(!edge_state
            .client_transactions
            .contains_key(&key));
    }

    #[tokio::test]
    async fn test_client_transaction_timeout_triggers_failover() {
        let routes = RouteTable::new(vec![
            Route::new(
                "primary",
                "".to_string(),
                100,
                RouteTarget::new("gw1".to_string(), "127.0.0.1".to_string(), Some(12345)),
            ),
            Route::new(
                "secondary",
                "".to_string(),
                200,
                RouteTarget::new("gw2".to_string(), "127.0.0.1".to_string(), Some(23456)),
            ),
        ]);
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let edge_state = Arc::new(EdgeState::new(CallManager::new(routes, tx)));
        let socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let call_id = "timeout-failover-test@example.com";

        let invite = format!(
            concat!(
                "INVITE sip:13800138000@example.com SIP/2.0\r\n",
                "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-timeout\r\n",
                "Max-Forwards: 70\r\n",
                "From: <sip:1001@example.com>;tag=from-tag\r\n",
                "To: <sip:13800138000@example.com>\r\n",
                "Call-ID: {call_id}\r\n",
                "CSeq: 1 INVITE\r\n",
                "Content-Length: 0\r\n",
                "\r\n"
            ),
            call_id = call_id
        );

        let datagrams =
            handle_datagram(invite.as_bytes(), peer(), &edge_state, &edge_config()).await;
        assert_eq!(datagrams.len(), 2);
        assert_eq!(datagrams[1].target, "127.0.0.1:23456");

        let outbound_invite = parse_message(&datagrams[1].bytes).unwrap();
        let SipMessage::Request(outbound_req) = outbound_invite else {
            panic!("expected request");
        };
        let key = ClientTransactionKey::from_request(&outbound_req).unwrap();

        spawn_client_transaction_retransmission(
            Arc::clone(&edge_state),
            Arc::clone(&socket),
            "127.0.0.1:23456".to_string(),
            datagrams[1].bytes.clone(),
            key,
            Arc::new(edge_config()),
        );

        let mut success = false;
        for _ in 0..15 {
            tokio::time::sleep(Duration::from_millis(20)).await;
            let call_guard = &edge_state.call_manager;
            if let Some(call) = call_guard.get(&CallId::new(call_id)) {
                if call.current_candidate_index == 1 {
                    success = true;
                    break;
                }
            }
        }
        assert!(success, "failed to trigger failover within timeout");

        let call_guard = &edge_state.call_manager;
        let call = call_guard.get(&CallId::new(call_id)).unwrap();
        assert_eq!(call.state, CallState::Routing);
        assert_eq!(
            call.outbound.as_ref().unwrap().remote_uri.to_string(),
            "sip:13800138000@127.0.0.1:12345;transport=udp"
        );
    }
