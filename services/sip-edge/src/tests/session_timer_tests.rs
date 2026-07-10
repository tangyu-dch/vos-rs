    /// Verify that the outbound INVITE carries Session-Expires and Supported: timer headers.
    #[tokio::test]
    async fn test_session_timer_header_injected_in_invite() {
        let edge_state = state_with_gateway_uri("sip:198.51.100.20:5060");
        let call_id = "session-timer-header-test-001";

        let sdp_body = "v=0\r\no=- 0 0 IN IP4 192.0.2.10\r\ns=-\r\nc=IN IP4 192.0.2.10\r\nt=0 0\r\nm=audio 49170 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n";
        let invite = format!(
            "INVITE sip:13800138000@example.com SIP/2.0\r\nVia: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-st-hdr\r\nMax-Forwards: 70\r\nFrom: <sip:1001@example.com>;tag=from-tag\r\nTo: <sip:13800138000@example.com>\r\nCall-ID: {call_id}\r\nCSeq: 1 INVITE\r\nContact: <sip:1001@192.0.2.10>\r\nContent-Type: application/sdp\r\nContent-Length: {len}\r\n\r\n{sdp_body}",
            len = sdp_body.len()
        );

        let datagrams = handle_datagram(
            invite.as_bytes(),
            "192.0.2.10:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;
        // Should produce 2 datagrams: 100 Trying to caller + outbound INVITE to gateway
        assert_eq!(datagrams.len(), 2);

        let outbound_invite = datagram_text(&datagrams[1]);
        assert!(
            outbound_invite.contains("Session-Expires: 600;refresher=uac"),
            "outbound INVITE must carry Session-Expires header\n{outbound_invite}"
        );
        assert!(
            outbound_invite.contains("Supported: timer"),
            "outbound INVITE must carry Supported: timer header\n{outbound_invite}"
        );
        assert!(
            outbound_invite.contains("Min-SE: 90"),
            "outbound INVITE must carry Min-SE header\n{outbound_invite}"
        );
    }

    /// Verify that a 200 OK containing Session-Expires stores the value on the transaction.
    #[tokio::test]
    async fn test_session_expires_stored_from_200_ok() {
        let edge_state = state_with_gateway_uri("sip:198.51.100.20:5060");
        let call_id = "session-timer-store-test-001";
        let sdp_body = "v=0\r\no=- 0 0 IN IP4 192.0.2.10\r\ns=-\r\nc=IN IP4 192.0.2.10\r\nt=0 0\r\nm=audio 49170 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n";

        // Step 1: send INVITE to establish transaction
        let invite = format!(
            "INVITE sip:13800138000@example.com SIP/2.0\r\n\
             Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-se-store\r\n\
             Max-Forwards: 70\r\n\
             From: <sip:1001@example.com>;tag=from-tag\r\n\
             To: <sip:13800138000@example.com>\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 1 INVITE\r\n\
             Contact: <sip:1001@192.0.2.10>\r\n\
             Content-Type: application/sdp\r\n\
             Content-Length: {len}\r\n\r\n{sdp_body}",
            len = sdp_body.len()
        );
        handle_datagram(
            invite.as_bytes(),
            "192.0.2.10:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // Step 2: gateway returns 200 OK with Session-Expires
        let sdp_answer = "v=0\r\no=- 0 0 IN IP4 198.51.100.20\r\ns=-\r\nc=IN IP4 198.51.100.20\r\nt=0 0\r\nm=audio 49172 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n";
        let ok_200 = format!(
            "SIP/2.0 200 OK\r\n\
             Via: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-se-store\r\n\
             From: <sip:1001@example.com>;tag=from-tag\r\n\
             To: <sip:13800138000@example.com>;tag=gw-tag\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 1 INVITE\r\n\
             Session-Expires: 600;refresher=uac\r\n\
             Content-Type: application/sdp\r\n\
             Content-Length: {len}\r\n\r\n{sdp_answer}",
            len = sdp_answer.len()
        );
        handle_datagram(
            ok_200.as_bytes(),
            "198.51.100.20:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // Verify session timer was stored on the transaction
        let tx = edge_state.inbound_transactions.get(call_id).expect("transaction must exist");
        assert_eq!(
            tx.session_expires,
            Some(600),
            "session_expires must be stored"
        );
        assert_eq!(
            tx.session_refresher.as_deref(),
            Some("uac"),
            "refresher must be stored"
        );
        assert!(
            tx.last_session_refresh.is_some(),
            "last_session_refresh must be set"
        );
    }

    /// Verify that Re-INVITE resets the last_session_refresh timestamp.
    #[tokio::test]
    async fn test_session_refresh_resets_on_reinvite() {
        let edge_state = state_with_gateway_uri("sip:198.51.100.20:5060");
        let call_id = "session-timer-refresh-test-001";
        let sdp_body = "v=0\r\no=- 0 0 IN IP4 192.0.2.10\r\ns=-\r\nc=IN IP4 192.0.2.10\r\nt=0 0\r\nm=audio 49170 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n";

        // Establish the call
        let invite = format!(
            "INVITE sip:13800138000@example.com SIP/2.0\r\n\
             Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-se-refresh\r\n\
             Max-Forwards: 70\r\n\
             From: <sip:1001@example.com>;tag=from-tag\r\n\
             To: <sip:13800138000@example.com>\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 1 INVITE\r\n\
             Contact: <sip:1001@192.0.2.10>\r\n\
             Content-Type: application/sdp\r\n\
             Content-Length: {len}\r\n\r\n{sdp_body}",
            len = sdp_body.len()
        );
        handle_datagram(
            invite.as_bytes(),
            "192.0.2.10:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        let sdp_answer = "v=0\r\no=- 0 0 IN IP4 198.51.100.20\r\ns=-\r\nc=IN IP4 198.51.100.20\r\nt=0 0\r\nm=audio 49172 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n";
        let ok_200 = format!(
            "SIP/2.0 200 OK\r\n\
             Via: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-se-refresh\r\n\
             From: <sip:1001@example.com>;tag=from-tag\r\n\
             To: <sip:13800138000@example.com>;tag=gw-tag\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 1 INVITE\r\n\
             Session-Expires: 600;refresher=uac\r\n\
             Content-Type: application/sdp\r\n\
             Content-Length: {len}\r\n\r\n{sdp_answer}",
            len = sdp_answer.len()
        );
        handle_datagram(
            ok_200.as_bytes(),
            "198.51.100.20:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // Capture last_session_refresh time before Re-INVITE
        let before_reinvite = {
            let tx_guard = edge_state.inbound_transactions.get(call_id).unwrap(); tx_guard.last_session_refresh
        };

        // Small delay so the timestamp will differ
        tokio::time::sleep(Duration::from_millis(5)).await;

        // Send Re-INVITE (To has tag) — this acts as session refresh
        let reinvite_sdp = sdp_body;
        let reinvite = format!(
            "INVITE sip:13800138000@example.com SIP/2.0\r\n\
             Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-se-refresh-2\r\n\
             Max-Forwards: 70\r\n\
             From: <sip:1001@example.com>;tag=from-tag\r\n\
             To: <sip:13800138000@example.com>;tag=gw-tag\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 2 INVITE\r\n\
             Contact: <sip:1001@192.0.2.10>\r\n\
             Content-Type: application/sdp\r\n\
             Content-Length: {len}\r\n\r\n{reinvite_sdp}",
            len = reinvite_sdp.len()
        );
        handle_datagram(
            reinvite.as_bytes(),
            "192.0.2.10:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        let after_reinvite = {
            let tx_guard = edge_state.inbound_transactions.get(call_id).unwrap(); tx_guard.last_session_refresh
        };

        assert!(before_reinvite.is_some(), "initial timestamp must be set");
        assert!(
            after_reinvite.is_some(),
            "post-reinvite timestamp must be set"
        );
        assert!(
            after_reinvite.unwrap() >= before_reinvite.unwrap(),
            "last_session_refresh must be updated after Re-INVITE"
        );
    }

    #[tokio::test]
    async fn test_session_timer_response_forwarding() {
        let raw_resp = concat!(
            "SIP/2.0 200 OK\r\n",
            "Via: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-vosrs\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:13800138000@example.com>;tag=gw-tag\r\n",
            "Call-ID: test-session-expires-forwarding@example.com\r\n",
            "CSeq: 1 INVITE\r\n",
            "Session-Expires: 600;refresher=uac\r\n",
            "Min-SE: 90\r\n",
            "Supported: timer\r\n",
            "Content-Length: 0\r\n\r\n"
        );

        let SipMessage::Response(resp) = parse_message(raw_resp.as_bytes()).unwrap() else {
            panic!("expected response");
        };

        let vias = vec!["SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-inbound".to_string()];
        let route_set = vec![];
        let forwarded =
            response::forward_response_to_inbound_with_body(&resp, &vias, &route_set, &[]);
        let forwarded_str = String::from_utf8(forwarded).unwrap();

        assert!(forwarded_str.contains("Session-Expires: 600;refresher=uac\r\n"));
        assert!(forwarded_str.contains("Min-SE: 90\r\n"));
        assert!(forwarded_str.contains("Supported: timer\r\n"));
    }

    #[tokio::test]
    async fn test_active_session_refresh_triggering() {
        let edge_state = Arc::new(state_with_default_route());
        let call_id = "test-active-refresh-trigger@example.com";

        // Setup a tracked established call
        let raw_invite = concat!(
            "INVITE sip:13800138000@example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-refresh-invite\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:13800138000@example.com>\r\n",
            "Call-ID: test-active-refresh-trigger@example.com\r\n",
            "CSeq: 1 INVITE\r\n",
            "Content-Length: 0\r\n\r\n"
        );

        // 1. Receive INVITE
        let _ = handle_datagram(raw_invite.as_bytes(), peer(), &edge_state, &edge_config()).await;

        // 2. Setup refresher = Some("uac") and last_session_refresh = 10 seconds ago
        {
            // DashMap get_mut returns RefMut
            let mut tx = edge_state.inbound_transactions.get_mut(call_id).unwrap();
            tx.session_expires = Some(10); // Expires in 10s, refresh at 5s
            tx.session_refresher = Some("uac".to_string());
            tx.last_session_refresh =
                Some(std::time::Instant::now() - std::time::Duration::from_secs(6));
            tx.callee_contact =
                Some(SipUri::from_str("sip:13800138000@gw-real-ip.com:5060").unwrap());
        }

        // 3. Setup a mock socket to capture outbound refresh UPDATE packet
        let tokio_socket = Arc::new(tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let port = tokio_socket.local_addr().unwrap().port();

        // Spawn watchdog with short interval
        let mut config = edge_config();
        config.advertised_addr = format!("127.0.0.1:{}", port);

        spawn_session_timer_watchdog(Arc::clone(&edge_state), Arc::clone(&tokio_socket), Arc::new(config));

        // Wait a bit for watchdog loop to tick
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;

        // Verify that the call's last_session_refresh was reset (throttled)
        {
            let tx = edge_state.inbound_transactions.get(call_id).unwrap();
            let elapsed = tx.last_session_refresh.unwrap().elapsed().as_secs();
            assert!(elapsed < 2);
        }
    }

    #[tokio::test]
    async fn test_self_refresh_response_drop() {
        let edge_state = state_with_default_route();
        let call_id = "test-self-refresh-response-drop@example.com";

        // Setup a tracked established call
        let raw_invite = concat!(
            "INVITE sip:13800138000@example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-drop-invite\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:13800138000@example.com>\r\n",
            "Call-ID: test-self-refresh-response-drop@example.com\r\n",
            "CSeq: 1 INVITE\r\n",
            "Content-Length: 0\r\n\r\n"
        );

        let _ = handle_datagram(raw_invite.as_bytes(), peer(), &edge_state, &edge_config()).await;

        // Setup mock session timer state
        {
            // DashMap get_mut returns RefMut
            let mut tx = edge_state.inbound_transactions.get_mut(call_id).unwrap();
            tx.session_expires = Some(600);
            tx.session_refresher = Some("uac".to_string());
            tx.last_session_refresh =
                Some(std::time::Instant::now() - std::time::Duration::from_secs(400));
        }

        // Send a 200 OK response corresponding to our self-generated refresh request (Via contains branch=z9hG4bK-refresh-)
        let raw_200 = format!(
            "SIP/2.0 200 OK\r\n\
             Via: SIP/2.0/UDP 127.0.0.1:5060;branch=z9hG4bK-refresh-true-2\r\n\
             From: <sip:1001@example.com>;tag=from-tag\r\n\
             To: <sip:13800138000@example.com>;tag=gw-tag\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 2 UPDATE\r\n\
             Content-Length: 0\r\n\r\n",
            call_id = call_id
        );
        let datagrams = handle_datagram(
            raw_200.as_bytes(),
            "203.0.113.20:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // The response must be consumed (empty datagram list returned)
        assert!(datagrams.is_empty());

        // The last_session_refresh must be reset to now (elapsed < 2 seconds)
        {
            let tx = edge_state.inbound_transactions.get(call_id).unwrap();
            let elapsed = tx.last_session_refresh.unwrap().elapsed().as_secs();
            assert!(elapsed < 2);
        }
    }
