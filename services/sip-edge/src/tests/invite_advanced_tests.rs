    #[tokio::test]
    async fn forwards_gateway_ringing_response_to_inbound_peer() {
        let edge_state = state_with_default_route();
        let invite = concat!(
            "INVITE sip:13800138000@example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-inbound\r\n",
            "Max-Forwards: 70\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:13800138000@example.com>\r\n",
            "Call-ID: invite-2@example.com\r\n",
            "CSeq: 1 INVITE\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        );
        let _ = handle_datagram(invite.as_bytes(), peer(), &edge_state, &edge_config()).await;

        let gateway_response = concat!(
            "SIP/2.0 180 Ringing\r\n",
            "Via: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-vosrs\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:13800138000@example.com>;tag=gw-tag\r\n",
            "Call-ID: invite-2@example.com\r\n",
            "CSeq: 1 INVITE\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        );

        let datagrams = handle_datagram(
            gateway_response.as_bytes(),
            "198.51.100.20:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        assert_eq!(datagrams.len(), 1);
        assert_eq!(datagrams[0].target, "192.0.2.10:5060");

        let response = datagram_text(&datagrams[0]);
        assert!(response.starts_with("SIP/2.0 180 Ringing\r\n"));
        assert!(response.contains("Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-inbound\r\n"));
        assert!(response.contains("To: <sip:13800138000@example.com>;tag=gw-tag\r\n"));

        let call_guard = &edge_state.call_manager;
        let call = call_guard
            .get(&CallId::new("invite-2@example.com"))
            .expect("call should still be tracked");
        assert_eq!(call.state, CallState::Ringing);
    }

    #[tokio::test]
    async fn forwards_gateway_ok_with_sdp_and_establishes_call() {
        let edge_state = state_with_default_route();
        send_invite(&edge_state, "invite-3@example.com").await;

        let gateway_response = concat!(
            "SIP/2.0 200 OK\r\n",
            "Via: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-vosrs\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:13800138000@example.com>;tag=gw-tag\r\n",
            "Call-ID: invite-3@example.com\r\n",
            "CSeq: 1 INVITE\r\n",
            "Content-Type: application/sdp\r\n",
            "Content-Length: 5\r\n",
            "\r\n",
            "v=0\r\n"
        );

        let datagrams = handle_datagram(
            gateway_response.as_bytes(),
            "198.51.100.20:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        assert_eq!(datagrams.len(), 1);
        assert_eq!(datagrams[0].target, "192.0.2.10:5060");

        let response = datagram_text(&datagrams[0]);
        assert!(response.starts_with("SIP/2.0 200 OK\r\n"));
        assert!(response.contains("Content-Type: application/sdp\r\n"));
        assert!(response.contains("Content-Length: 5\r\n\r\nv=0\r\n"));

        let call_guard = &edge_state.call_manager;
        let call = call_guard
            .get(&CallId::new("invite-3@example.com"))
            .expect("call should still be tracked");
        assert_eq!(call.state, CallState::Established);
        // DashMap get returns Ref which owns the lock
        let transaction = edge_state.inbound_transactions.get("invite-3@example.com")
            .expect("transaction should be remembered");
        assert_eq!(transaction.inbound_from_tag.as_deref(), Some("from-tag"));
        assert_eq!(transaction.inbound_to_tag.as_deref(), Some("gw-tag"));
        assert_eq!(transaction.last_inbound_cseq, Some(1));
    }

    #[tokio::test]
    async fn initial_ok_after_early_media_establishes_call() {
        let edge_state = state_with_default_route();
        let call_id = "invite-early-media-ok@example.com";
        send_invite(&edge_state, call_id).await;

        // A 183 response with SDP allocates this relay before the initial INVITE's 200 OK.
        let mut transaction = edge_state
            .inbound_transactions
            .get_mut(call_id)
            .expect("transaction should be remembered");
        transaction.caller_relay_rtp = Some(RtpEndpoint::new("192.0.2.20", 40_000));
        drop(transaction);

        send_gateway_ok(&edge_state, call_id).await;

        let call = edge_state
            .call_manager
            .get(&CallId::new(call_id))
            .expect("call should still be tracked");
        assert_eq!(call.state, CallState::Established);
        assert!(call.answered_at.is_some());
    }

    #[tokio::test]
    async fn pairs_rtp_relay_ports_after_sdp_offer_answer() {
        let edge_state = state_with_default_route();
        let offer_body = concat!(
            "v=0\r\n",
            "o=caller 1 1 IN IP4 192.0.2.10\r\n",
            "s=caller\r\n",
            "c=IN IP4 192.0.2.10\r\n",
            "t=0 0\r\n",
            "m=audio 49170 RTP/AVP 0 8\r\n",
            "a=rtpmap:0 PCMU/8000\r\n",
            "a=rtpmap:8 PCMA/8000\r\n"
        );
        let invite = format!(
            concat!(
                "INVITE sip:13800138000@example.com SIP/2.0\r\n",
                "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-sdp-pair\r\n",
                "Max-Forwards: 70\r\n",
                "From: <sip:1001@example.com>;tag=from-tag\r\n",
                "To: <sip:13800138000@example.com>\r\n",
                "Call-ID: invite-sdp-pair@example.com\r\n",
                "CSeq: 1 INVITE\r\n",
                "Content-Type: application/sdp\r\n",
                "Content-Length: {}\r\n",
                "\r\n",
                "{}"
            ),
            offer_body.len(),
            offer_body
        );
        let invite_datagrams =
            handle_datagram(invite.as_bytes(), peer(), &edge_state, &edge_config()).await;
        assert_eq!(invite_datagrams.len(), 2);

        let answer_body = concat!(
            "v=0\r\n",
            "o=gateway 1 1 IN IP4 198.51.100.20\r\n",
            "s=gateway\r\n",
            "c=IN IP4 198.51.100.20\r\n",
            "t=0 0\r\n",
            "m=audio 49172 RTP/AVP 8\r\n",
            "a=rtpmap:8 PCMA/8000\r\n"
        );
        let gateway_response = format!(
            concat!(
                "SIP/2.0 200 OK\r\n",
                "Via: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-vosrs\r\n",
                "From: <sip:1001@example.com>;tag=from-tag\r\n",
                "To: <sip:13800138000@example.com>;tag=gw-tag\r\n",
                "Call-ID: invite-sdp-pair@example.com\r\n",
                "CSeq: 1 INVITE\r\n",
                "Content-Type: application/sdp\r\n",
                "Content-Length: {}\r\n",
                "\r\n",
                "{}"
            ),
            answer_body.len(),
            answer_body
        );
        let answer_datagrams = handle_datagram(
            gateway_response.as_bytes(),
            "198.51.100.20:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;
        assert_eq!(answer_datagrams.len(), 1);

        let response = datagram_text(&answer_datagrams[0]);
        let port_min = get_test_port_min();
        assert!(response.contains(&format!("m=audio {} RTP/AVP 8\r\n", port_min + 2)));

        // DashMap get returns Ref which owns the lock
        let transaction = edge_state.inbound_transactions.get("invite-sdp-pair@example.com")
            .expect("transaction should be remembered");
        assert_eq!(
            transaction.gateway_relay_rtp,
            Some(RtpEndpoint::new("203.0.113.10", port_min))
        );
        assert_eq!(
            transaction.caller_relay_rtp,
            Some(RtpEndpoint::new("203.0.113.10", port_min + 2))
        );
        assert_eq!(
            edge_state.media_relay.peer_port_for(port_min),
            Some(port_min + 2)
        );
        assert_eq!(
            edge_state.media_relay.peer_port_for(port_min + 2),
            Some(port_min)
        );
        assert_eq!(
            edge_state.media_relay.target_for_port(port_min),
            Some("192.0.2.10:49170".parse().unwrap())
        );
        assert_eq!(
            edge_state.media_relay.target_for_port(port_min + 2),
            Some("198.51.100.20:49172".parse().unwrap())
        );
    }

    #[tokio::test]
    async fn forwards_inbound_ack_to_gateway_without_response() {
        let edge_state = state_with_default_route();
        send_invite(&edge_state, "invite-4@example.com").await;
        send_gateway_ok(&edge_state, "invite-4@example.com").await;

        let ack = concat!(
            "ACK sip:13800138000@example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-ack\r\n",
            "Max-Forwards: 70\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:13800138000@example.com>;tag=gw-tag\r\n",
            "Call-ID: invite-4@example.com\r\n",
            "CSeq: 1 ACK\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        );

        let datagrams = handle_datagram(ack.as_bytes(), peer(), &edge_state, &edge_config()).await;

        assert_eq!(datagrams.len(), 1);
        assert_eq!(datagrams[0].target, "gw1.example.com:5060");

        let outbound_ack = datagram_text(&datagrams[0]);
        assert!(outbound_ack
            .starts_with("ACK sip:13800138000@gw1.example.com:5060;transport=udp SIP/2.0\r\n"));
        assert!(outbound_ack.contains("CSeq: 1 ACK\r\n"));
        assert!(outbound_ack.contains("Content-Length: 0\r\n\r\n"));
    }

    #[tokio::test]
    async fn retransmitted_ack_is_forwarded_instead_of_cached() {
        let edge_state = state_with_default_route();
        send_invite(&edge_state, "invite-ack-retry@example.com").await;
        send_gateway_ok(&edge_state, "invite-ack-retry@example.com").await;

        let ack = concat!(
            "ACK sip:13800138000@example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-ack-retry\r\n",
            "Max-Forwards: 70\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:13800138000@example.com>;tag=gw-tag\r\n",
            "Call-ID: invite-ack-retry@example.com\r\n",
            "CSeq: 1 ACK\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        );

        let first = handle_datagram(ack.as_bytes(), peer(), &edge_state, &edge_config()).await;
        let second = handle_datagram(ack.as_bytes(), peer(), &edge_state, &edge_config()).await;

        assert_eq!(first.len(), 1);
        assert_eq!(second.len(), 1);
        assert_eq!(first[0].target, "gw1.example.com:5060");
        assert_eq!(second[0].target, "gw1.example.com:5060");
        assert!(datagram_text(&second[0])
            .starts_with("ACK sip:13800138000@gw1.example.com:5060;transport=udp SIP/2.0\r\n"));
    }

    #[tokio::test]
    async fn inbound_info_gets_ok_and_is_forwarded_to_gateway_with_body() {
        let edge_state = state_with_default_route();
        send_invite(&edge_state, "invite-info@example.com").await;
        send_gateway_ok(&edge_state, "invite-info@example.com").await;
        let body = "Signal=1\r\nDuration=160\r\n";
        let info = format!(
            concat!(
                "INFO sip:13800138000@example.com SIP/2.0\r\n",
                "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-info\r\n",
                "Max-Forwards: 70\r\n",
                "From: <sip:1001@example.com>;tag=from-tag\r\n",
                "To: <sip:13800138000@example.com>;tag=gw-tag\r\n",
                "Call-ID: invite-info@example.com\r\n",
                "CSeq: 2 INFO\r\n",
                "Content-Type: application/dtmf-relay\r\n",
                "Content-Length: {}\r\n",
                "\r\n",
                "{}"
            ),
            body.len(),
            body
        );

        let datagrams = handle_datagram(info.as_bytes(), peer(), &edge_state, &edge_config()).await;

        assert_eq!(datagrams.len(), 2);
        assert_eq!(datagrams[0].target, "192.0.2.10:5060");
        assert_eq!(datagrams[1].target, "gw1.example.com:5060");

        let response = datagram_text(&datagrams[0]);
        assert!(response.starts_with("SIP/2.0 200 OK\r\n"));
        assert!(response.contains("CSeq: 2 INFO\r\n"));

        let outbound_info = datagram_text(&datagrams[1]);
        assert!(outbound_info
            .starts_with("INFO sip:13800138000@gw1.example.com:5060;transport=udp SIP/2.0\r\n"));
        assert!(outbound_info.contains("CSeq: 2 INFO\r\n"));
        assert!(outbound_info.contains("Content-Type: application/dtmf-relay\r\n"));
        assert!(outbound_info.contains("Content-Length: 24\r\n\r\nSignal=1\r\nDuration=160\r\n"));
    }

    #[tokio::test]
    async fn retransmitted_info_replays_ok_without_duplicate_outbound_info() {
        let edge_state = state_with_default_route();
        send_invite(&edge_state, "invite-info-retry@example.com").await;
        send_gateway_ok(&edge_state, "invite-info-retry@example.com").await;
        let body = "Signal=2\r\nDuration=120\r\n";
        let info = format!(
            concat!(
                "INFO sip:13800138000@example.com SIP/2.0\r\n",
                "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-info-retry\r\n",
                "Max-Forwards: 70\r\n",
                "From: <sip:1001@example.com>;tag=from-tag\r\n",
                "To: <sip:13800138000@example.com>;tag=gw-tag\r\n",
                "Call-ID: invite-info-retry@example.com\r\n",
                "CSeq: 2 INFO\r\n",
                "Content-Type: application/dtmf-relay\r\n",
                "Content-Length: {}\r\n",
                "\r\n",
                "{}"
            ),
            body.len(),
            body
        );

        let first = handle_datagram(info.as_bytes(), peer(), &edge_state, &edge_config()).await;
        let second = handle_datagram(info.as_bytes(), peer(), &edge_state, &edge_config()).await;

        assert_eq!(first.len(), 2);
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].target, "192.0.2.10:5060");

        let response = datagram_text(&second[0]);
        assert!(response.starts_with("SIP/2.0 200 OK\r\n"));
        assert!(response.contains("CSeq: 2 INFO\r\n"));
    }

    #[tokio::test]
    async fn in_dialog_request_with_wrong_from_tag_receives_481() {
        let edge_state = state_with_default_route();
        send_invite(&edge_state, "invite-bad-from-tag@example.com").await;
        send_gateway_ok(&edge_state, "invite-bad-from-tag@example.com").await;
        let info = concat!(
            "INFO sip:13800138000@example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-bad-from-tag\r\n",
            "Max-Forwards: 70\r\n",
            "From: <sip:1001@example.com>;tag=wrong-tag\r\n",
            "To: <sip:13800138000@example.com>;tag=gw-tag\r\n",
            "Call-ID: invite-bad-from-tag@example.com\r\n",
            "CSeq: 2 INFO\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        );

        let datagrams = handle_datagram(info.as_bytes(), peer(), &edge_state, &edge_config()).await;

        assert_eq!(datagrams.len(), 1);
        assert_eq!(datagrams[0].target, "192.0.2.10:5060");
        let response = datagram_text(&datagrams[0]);
        assert!(response.starts_with("SIP/2.0 481 Call/Transaction Does Not Exist\r\n"));
        assert!(
            response.contains("X-VOS-RS-Error: in-dialog From tag does not match call dialog\r\n")
        );
    }

    #[tokio::test]
    async fn in_dialog_request_with_wrong_to_tag_receives_481() {
        let edge_state = state_with_default_route();
        send_invite(&edge_state, "invite-bad-to-tag@example.com").await;
        send_gateway_ok(&edge_state, "invite-bad-to-tag@example.com").await;
        let info = concat!(
            "INFO sip:13800138000@example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-bad-to-tag\r\n",
            "Max-Forwards: 70\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:13800138000@example.com>;tag=wrong-tag\r\n",
            "Call-ID: invite-bad-to-tag@example.com\r\n",
            "CSeq: 2 INFO\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        );

        let datagrams = handle_datagram(info.as_bytes(), peer(), &edge_state, &edge_config()).await;

        assert_eq!(datagrams.len(), 1);
        assert_eq!(datagrams[0].target, "192.0.2.10:5060");
        let response = datagram_text(&datagrams[0]);
        assert!(response.starts_with("SIP/2.0 481 Call/Transaction Does Not Exist\r\n"));
        assert!(
            response.contains("X-VOS-RS-Error: in-dialog To tag does not match call dialog\r\n")
        );
    }

    #[tokio::test]
    async fn in_dialog_request_with_stale_cseq_receives_server_error_without_forwarding() {
        let edge_state = state_with_default_route();
        send_invite(&edge_state, "invite-stale-cseq@example.com").await;
        send_gateway_ok(&edge_state, "invite-stale-cseq@example.com").await;
        let info = concat!(
            "INFO sip:13800138000@example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-info-before-stale\r\n",
            "Max-Forwards: 70\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:13800138000@example.com>;tag=gw-tag\r\n",
            "Call-ID: invite-stale-cseq@example.com\r\n",
            "CSeq: 2 INFO\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        );
        let first = handle_datagram(info.as_bytes(), peer(), &edge_state, &edge_config()).await;
        assert_eq!(first.len(), 2);

        let stale_bye = concat!(
            "BYE sip:13800138000@example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-stale-bye\r\n",
            "Max-Forwards: 70\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:13800138000@example.com>;tag=gw-tag\r\n",
            "Call-ID: invite-stale-cseq@example.com\r\n",
            "CSeq: 2 BYE\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        );

        let datagrams =
            handle_datagram(stale_bye.as_bytes(), peer(), &edge_state, &edge_config()).await;

        assert_eq!(datagrams.len(), 1);
        assert_eq!(datagrams[0].target, "192.0.2.10:5060");
        let response = datagram_text(&datagrams[0]);
        assert!(response.starts_with("SIP/2.0 500 Server Internal Error\r\n"));
        assert!(response
            .contains("X-VOS-RS-Error: out-of-order in-dialog CSeq: received 2, last 2\r\n"));

        let guard = &edge_state.call_manager;
        let call = guard
            .get(&CallId::new("invite-stale-cseq@example.com"))
            .expect("call should still be tracked");
        assert_eq!(call.state, CallState::Established);
    }

    #[tokio::test]
    async fn inbound_bye_gets_ok_and_is_forwarded_to_gateway() {
        let edge_state = state_with_default_route();
        send_invite(&edge_state, "invite-5@example.com").await;
        send_gateway_ok(&edge_state, "invite-5@example.com").await;

        let bye = concat!(
            "BYE sip:13800138000@example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-bye\r\n",
            "Max-Forwards: 70\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:13800138000@example.com>;tag=gw-tag\r\n",
            "Call-ID: invite-5@example.com\r\n",
            "CSeq: 2 BYE\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        );

        let datagrams = handle_datagram(bye.as_bytes(), peer(), &edge_state, &edge_config()).await;

        assert_eq!(datagrams.len(), 2);
        assert_eq!(datagrams[0].target, "192.0.2.10:5060");
        assert_eq!(datagrams[1].target, "gw1.example.com:5060");

        let response = datagram_text(&datagrams[0]);
        assert!(response.starts_with("SIP/2.0 200 OK\r\n"));
        assert!(response.contains("CSeq: 2 BYE\r\n"));

        let outbound_bye = datagram_text(&datagrams[1]);
        assert!(outbound_bye
            .starts_with("BYE sip:13800138000@gw1.example.com:5060;transport=udp SIP/2.0\r\n"));
        assert!(outbound_bye.contains("CSeq: 2 BYE\r\n"));

        let guard = &edge_state.call_manager;
        let call = guard
            .get(&CallId::new("invite-5@example.com"))
            .expect("call should still be tracked");
        assert_eq!(call.state, CallState::Terminated);
    }

    #[tokio::test]
    async fn gateway_bye_gets_ok_and_is_forwarded_to_caller() {
        let edge_state = state_with_default_route();
        send_invite(&edge_state, "invite-gateway-bye@example.com").await;
        send_gateway_ok(&edge_state, "invite-gateway-bye@example.com").await;

        let bye = concat!(
            "BYE sip:1001@example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 198.51.100.20:5060;branch=z9hG4bK-gw-bye\r\n",
            "Max-Forwards: 70\r\n",
            "From: <sip:13800138000@example.com>;tag=gw-tag\r\n",
            "To: <sip:1001@example.com>;tag=from-tag\r\n",
            "Call-ID: invite-gateway-bye@example.com\r\n",
            "CSeq: 2 BYE\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        );

        let datagrams = handle_datagram(
            bye.as_bytes(),
            "198.51.100.20:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        assert_eq!(datagrams.len(), 2);
        assert_eq!(datagrams[0].target, "198.51.100.20:5060");
        assert_eq!(datagrams[1].target, peer().to_string());

        let response = datagram_text(&datagrams[0]);
        assert!(response.starts_with("SIP/2.0 200 OK\r\n"));
        assert!(response.contains("CSeq: 2 BYE\r\n"));

        let forwarded_bye = datagram_text(&datagrams[1]);
        assert!(forwarded_bye.starts_with("BYE sip:192.0.2.10:5060 SIP/2.0\r\n"));
        assert!(forwarded_bye.contains("From: <sip:13800138000@example.com>;tag=gw-tag\r\n"));
        assert!(forwarded_bye.contains("To: <sip:1001@example.com>;tag=from-tag\r\n"));
        assert!(forwarded_bye.contains("CSeq: 2 BYE\r\n"));

        let guard = &edge_state.call_manager;
        let call = guard
            .get(&CallId::new("invite-gateway-bye@example.com"))
            .expect("call should still be tracked");
        assert_eq!(call.state, CallState::Terminated);
    }

    #[tokio::test]
    async fn retransmitted_bye_replays_ok_without_duplicate_outbound_bye() {
        let edge_state = state_with_default_route();
        send_invite(&edge_state, "invite-bye-retry@example.com").await;
        send_gateway_ok(&edge_state, "invite-bye-retry@example.com").await;

        let bye = concat!(
            "BYE sip:13800138000@example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-bye-retry\r\n",
            "Max-Forwards: 70\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:13800138000@example.com>;tag=gw-tag\r\n",
            "Call-ID: invite-bye-retry@example.com\r\n",
            "CSeq: 2 BYE\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        );

        let first = handle_datagram(bye.as_bytes(), peer(), &edge_state, &edge_config()).await;
        let second = handle_datagram(bye.as_bytes(), peer(), &edge_state, &edge_config()).await;

        assert_eq!(first.len(), 2);
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].target, "192.0.2.10:5060");

        let response = datagram_text(&second[0]);
        assert!(response.starts_with("SIP/2.0 200 OK\r\n"));
        assert!(response.contains("CSeq: 2 BYE\r\n"));
    }

    #[tokio::test]
    async fn inbound_cancel_gets_ok_and_is_forwarded_to_gateway() {
        let edge_state = state_with_default_route();
        send_invite(&edge_state, "invite-6@example.com").await;

        let cancel = concat!(
            "CANCEL sip:13800138000@example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-cancel\r\n",
            "Max-Forwards: 70\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:13800138000@example.com>\r\n",
            "Call-ID: invite-6@example.com\r\n",
            "CSeq: 1 CANCEL\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        );

        let datagrams =
            handle_datagram(cancel.as_bytes(), peer(), &edge_state, &edge_config()).await;

        assert_eq!(datagrams.len(), 2);
        assert_eq!(datagrams[0].target, "192.0.2.10:5060");
        assert_eq!(datagrams[1].target, "gw1.example.com:5060");

        let response = datagram_text(&datagrams[0]);
        assert!(response.starts_with("SIP/2.0 200 OK\r\n"));
        assert!(response.contains("CSeq: 1 CANCEL\r\n"));

        let outbound_cancel = datagram_text(&datagrams[1]);
        assert!(outbound_cancel
            .starts_with("CANCEL sip:13800138000@gw1.example.com:5060;transport=udp SIP/2.0\r\n"));
        assert!(outbound_cancel.contains("CSeq: 1 CANCEL\r\n"));

        let guard = &edge_state.call_manager;
        let call = guard
            .get(&CallId::new("invite-6@example.com"))
            .expect("call should still be tracked");
        assert_eq!(call.state, CallState::Terminated);
    }

    #[tokio::test]
    async fn test_gateway_failover_on_503() {
        let routes = RouteTable::new(vec![
            Route::new(
                "primary",
                "",
                200,
                RouteTarget::new("gw1", "gw1.example.com", Some(5060)),
            ),
            Route::new(
                "backup",
                "",
                100,
                RouteTarget::new("gw2", "gw2.example.com", Some(5060)),
            ),
        ]);
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let edge_state = EdgeState::new(CallManager::new(routes, tx));
        let call_id = "failover-test@example.com";

        let body = concat!(
            "v=0\r\n",
            "o=caller 1 1 IN IP4 192.0.2.10\r\n",
            "s=caller\r\n",
            "c=IN IP4 192.0.2.10\r\n",
            "t=0 0\r\n",
            "m=audio 49170 RTP/AVP 0\r\n",
            "a=rtpmap:0 PCMU/8000\r\n"
        );
        let invite = format!(
            concat!(
                "INVITE sip:13800138000@example.com SIP/2.0\r\n",
                "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-failover-invite\r\n",
                "Max-Forwards: 70\r\n",
                "From: <sip:1001@example.com>;tag=from-tag\r\n",
                "To: <sip:13800138000@example.com>\r\n",
                "Call-ID: {call_id}\r\n",
                "CSeq: 1 INVITE\r\n",
                "Content-Type: application/sdp\r\n",
                "Content-Length: {}\r\n",
                "\r\n",
                "{}"
            ),
            body.len(),
            body,
            call_id = call_id
        );

        let datagrams =
            handle_datagram(invite.as_bytes(), peer(), &edge_state, &edge_config()).await;
        assert_eq!(datagrams.len(), 2);
        assert!(datagram_text(&datagrams[0]).starts_with("SIP/2.0 100 Trying\r\n"));
        assert_eq!(datagrams[1].target, "gw1.example.com:5060");
        assert!(datagram_text(&datagrams[1])
            .starts_with("INVITE sip:13800138000@gw1.example.com:5060;transport=udp SIP/2.0\r\n"));

        let failure_response = format!(
            concat!(
                "SIP/2.0 503 Service Unavailable\r\n",
                "Via: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-vosrs\r\n",
                "From: <sip:1001@example.com>;tag=from-tag\r\n",
                "To: <sip:13800138000@example.com>;tag=gw1-tag\r\n",
                "Call-ID: {call_id}\r\n",
                "CSeq: 1 INVITE\r\n",
                "Content-Length: 0\r\n",
                "\r\n"
            ),
            call_id = call_id
        );

        let failover_datagrams = handle_datagram(
            failure_response.as_bytes(),
            "198.51.100.20:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        assert_eq!(failover_datagrams.len(), 1);
        assert_eq!(failover_datagrams[0].target, "gw2.example.com:5060");
        assert!(datagram_text(&failover_datagrams[0])
            .starts_with("INVITE sip:13800138000@gw2.example.com:5060;transport=udp SIP/2.0\r\n"));

        let call_guard = &edge_state.call_manager;
        let call = call_guard.get(&CallId::new(call_id)).unwrap();
        assert_eq!(call.state, CallState::Routing);
        assert_eq!(call.current_candidate_index, 1);
        assert_eq!(call.outbound_history.len(), 1);
        assert_eq!(
            call.outbound_history[0].remote_uri.to_string(),
            "sip:13800138000@gw1.example.com:5060;transport=udp"
        );
        assert_eq!(
            call.outbound.as_ref().unwrap().remote_uri.to_string(),
            "sip:13800138000@gw2.example.com:5060;transport=udp"
        );
    }

    #[tokio::test]
    async fn test_gateway_302_redirect_recursion() {
        let routes = RouteTable::new(vec![Route::new(
            "primary",
            "",
            200,
            RouteTarget::new("gw1", "gw1.example.com", Some(5060)),
        )]);
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let edge_state = EdgeState::new(CallManager::new(routes, tx));
        let call_id = "redirect-test@example.com";

        let body = concat!(
            "v=0\r\n",
            "o=caller 1 1 IN IP4 192.0.2.10\r\n",
            "s=caller\r\n",
            "c=IN IP4 192.0.2.10\r\n",
            "t=0 0\r\n",
            "m=audio 49170 RTP/AVP 0\r\n",
            "a=rtpmap:0 PCMU/8000\r\n"
        );
        let invite = format!(
            concat!(
                "INVITE sip:13800138000@example.com SIP/2.0\r\n",
                "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-redirect-invite\r\n",
                "Max-Forwards: 70\r\n",
                "From: <sip:1001@example.com>;tag=from-tag\r\n",
                "To: <sip:13800138000@example.com>\r\n",
                "Call-ID: {call_id}\r\n",
                "CSeq: 1 INVITE\r\n",
                "Content-Type: application/sdp\r\n",
                "Content-Length: {}\r\n",
                "\r\n",
                "{}"
            ),
            body.len(),
            body,
            call_id = call_id
        );

        let datagrams =
            handle_datagram(invite.as_bytes(), peer(), &edge_state, &edge_config()).await;
        assert_eq!(datagrams.len(), 2);
        assert_eq!(datagrams[1].target, "gw1.example.com:5060");

        // GW1 responds with 302 Moved Temporarily directing to sip:13800138000@redirect-target.example.com:5060
        let redirect_response = format!(
            concat!(
                "SIP/2.0 302 Moved Temporarily\r\n",
                "Via: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-vosrs\r\n",
                "From: <sip:1001@example.com>;tag=from-tag\r\n",
                "To: <sip:13800138000@example.com>;tag=gw1-tag\r\n",
                "Call-ID: {call_id}\r\n",
                "CSeq: 1 INVITE\r\n",
                "Contact: <sip:13800138000@redirect-target.example.com:5060>\r\n",
                "Content-Length: 0\r\n",
                "\r\n"
            ),
            call_id = call_id
        );

        let redirect_datagrams = handle_datagram(
            redirect_response.as_bytes(),
            "198.51.100.20:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // Verify that 302 response was intercepted and resulted in redirect INVITE to redirect-target.example.com:5060
        assert_eq!(redirect_datagrams.len(), 1);
        assert_eq!(
            redirect_datagrams[0].target,
            "redirect-target.example.com:5060"
        );
        assert!(datagram_text(&redirect_datagrams[0])
            .starts_with("INVITE sip:13800138000@redirect-target.example.com:5060 SIP/2.0\r\n"));

        let call_guard = &edge_state.call_manager;
        let call = call_guard.get(&CallId::new(call_id)).unwrap();
        assert_eq!(call.state, CallState::Routing);
        assert_eq!(call.current_candidate_index, 1);
        assert_eq!(
            call.outbound.as_ref().unwrap().remote_uri.to_string(),
            "sip:13800138000@redirect-target.example.com:5060"
        );
    }

    #[tokio::test]
    async fn test_path_service_route_propagation() {
        let config = edge_config();
        let edge_state = state_with_default_route_and_config(&config);

        let register = concat!(
            "REGISTER sip:example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-regpath\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:1001@example.com>\r\n",
            "Call-ID: reg-path-01\r\n",
            "CSeq: 1 REGISTER\r\n",
            "Contact: <sip:1001@192.0.2.10:5070;transport=udp>;expires=60\r\n",
            "Path: <sip:proxy1.example.com;lr>, <sip:proxy2.example.com;lr>\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        );

        let d1 = handle_datagram(
            register.as_bytes(),
            "192.0.2.10:5060".parse().unwrap(),
            &edge_state,
            &config,
        )
        .await;
        assert_eq!(d1.len(), 1);
        let resp = datagram_text(&d1[0]);
        assert!(resp.starts_with("SIP/2.0 200 OK\r\n"));
        let expected_service_route =
            format!("Service-Route: <sip:{};lr>\r\n", config.advertised_addr);
        assert!(resp.contains(&expected_service_route));

        let call_id = "path-invite-01";
        let body = sdp_body();
        let invite = format!(
            concat!(
                "INVITE sip:1001@example.com SIP/2.0\r\n",
                "Via: SIP/2.0/UDP 192.0.2.200:5060;branch=z9hG4bK-invitepath\r\n",
                "Max-Forwards: 70\r\n",
                "From: <sip:1002@example.com>;tag=caller-tag\r\n",
                "To: <sip:1001@example.com>\r\n",
                "Call-ID: {call_id}\r\n",
                "CSeq: 1 INVITE\r\n",
                "Content-Length: {}\r\n\r\n",
                "{}"
            ),
            body.len(),
            body,
            call_id = call_id
        );

        let d2 = handle_datagram(
            invite.as_bytes(),
            "192.0.2.200:5060".parse().unwrap(),
            &edge_state,
            &config,
        )
        .await;
        assert_eq!(d2.len(), 2);
        let forwarded_invite = datagram_text(&d2[1]);
        assert!(forwarded_invite.contains("Route: <sip:proxy1.example.com;lr>\r\n"));
        assert!(forwarded_invite.contains("Route: <sip:proxy2.example.com;lr>\r\n"));
    }

    #[tokio::test]
    async fn test_record_route_and_route_propagation() {
        let edge_state = state_with_default_route();
        let call_id = "rr-test@example.com";

        let invite = format!(
            concat!(
                "INVITE sip:13800138000@example.com SIP/2.0\r\n",
                "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-rr-invite\r\n",
                "Max-Forwards: 70\r\n",
                "From: <sip:1001@example.com>;tag=from-tag\r\n",
                "To: <sip:13800138000@example.com>\r\n",
                "Call-ID: {call_id}\r\n",
                "CSeq: 1 INVITE\r\n",
                "Record-Route: <sip:proxy-inbound.example.com;lr>\r\n",
                "Content-Length: 0\r\n",
                "\r\n"
            ),
            call_id = call_id
        );

        let datagrams =
            handle_datagram(invite.as_bytes(), peer(), &edge_state, &edge_config()).await;
        assert_eq!(datagrams.len(), 2);
        assert_eq!(datagrams[1].target, "gw1.example.com:5060");
        let invite_out = datagram_text(&datagrams[1]);
        assert!(!invite_out.contains("Record-Route:"));

        let response_200 = format!(
            concat!(
                "SIP/2.0 200 OK\r\n",
                "Via: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-vosrs\r\n",
                "From: <sip:1001@example.com>;tag=from-tag\r\n",
                "To: <sip:13800138000@example.com>;tag=gw-tag\r\n",
                "Call-ID: {call_id}\r\n",
                "CSeq: 1 INVITE\r\n",
                "Record-Route: <sip:proxy-outbound.example.com;lr>\r\n",
                "Contact: <sip:gateway-direct@198.51.100.20:5070;transport=udp>\r\n",
                "Content-Length: 0\r\n",
                "\r\n"
            ),
            call_id = call_id
        );

        let answer_datagrams = handle_datagram(
            response_200.as_bytes(),
            "198.51.100.20:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        assert_eq!(answer_datagrams.len(), 1);
        let response_to_caller = datagram_text(&answer_datagrams[0]);
        assert!(response_to_caller.contains("Record-Route: <sip:proxy-inbound.example.com;lr>\r\n"));

        let ack = format!(
            concat!(
                "ACK sip:13800138000@example.com SIP/2.0\r\n",
                "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-rr-ack\r\n",
                "Max-Forwards: 70\r\n",
                "From: <sip:1001@example.com>;tag=from-tag\r\n",
                "To: <sip:13800138000@example.com>;tag=gw-tag\r\n",
                "Call-ID: {call_id}\r\n",
                "CSeq: 1 ACK\r\n",
                "Content-Length: 0\r\n",
                "\r\n"
            ),
            call_id = call_id
        );

        let ack_datagrams =
            handle_datagram(ack.as_bytes(), peer(), &edge_state, &edge_config()).await;
        assert_eq!(ack_datagrams.len(), 1);
        assert_eq!(ack_datagrams[0].target, "proxy-outbound.example.com:5060");
        let ack_out = datagram_text(&ack_datagrams[0]);
        assert!(ack_out
            .starts_with("ACK sip:gateway-direct@198.51.100.20:5070;transport=udp SIP/2.0\r\n"));
        assert!(ack_out.contains("Route: <sip:proxy-outbound.example.com;lr>\r\n"));
    }

    #[tokio::test]
    async fn test_anti_fraud_callee_blacklist() {
        let edge_state = state_with_default_route();
        
        // 注入被叫前缀黑名单规则
        {
            let mut rules = edge_state.anti_fraud_rules.write().unwrap();
            rules.push(cdr_core::AntiFraudRule {
                id: "rule-1".to_string(),
                rule_type: "callee_blacklist".to_string(),
                target_value: "13800".to_string(), // 拦截所有 13800 开头的号码
                limit_number: None,
                enabled: true,
            });
        }

        let call_id = "antifraud-callee-blacklist@example.com";
        let body = "v=0\r\n";
        let invite = format!(
            concat!(
                "INVITE sip:13800138000@example.com SIP/2.0\r\n",
                "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-blacklist\r\n",
                "Max-Forwards: 70\r\n",
                "From: <sip:1001@example.com>;tag=from-tag\r\n",
                "To: <sip:13800138000@example.com>\r\n",
                "Call-ID: {call_id}\r\n",
                "CSeq: 1 INVITE\r\n",
                "Content-Type: application/sdp\r\n",
                "Content-Length: {}\r\n",
                "\r\n",
                "{}"
            ),
            body.len(),
            body,
            call_id = call_id
        );

        let datagrams =
            handle_datagram(invite.as_bytes(), peer(), &edge_state, &edge_config()).await;
        
        // 应该返回 1 个表示 Forbidden 的本地包，不向网关转发
        assert_eq!(datagrams.len(), 1);
        assert_eq!(datagrams[0].target, "192.0.2.10:5060");
        let response = datagram_text(&datagrams[0]);
        assert!(response.starts_with("SIP/2.0 403 Forbidden"));
        assert!(response.contains("X-VOS-RS-Error: Callee number is blacklisted"));
    }

    #[tokio::test]
    async fn test_anti_fraud_user_concurrency_limit() {
        let edge_state = state_with_default_route();
        
        // 注入单账户最大并发限制为 1 的规则
        {
            let mut rules = edge_state.anti_fraud_rules.write().unwrap();
            rules.push(cdr_core::AntiFraudRule {
                id: "rule-2".to_string(),
                rule_type: "user_concurrency".to_string(),
                target_value: "1001".to_string(), // 针对 1001 账户
                limit_number: Some(1),
                enabled: true,
            });
        }

        // 手动递增该用户并发数，模拟当前已有 1 路通话进行中
        edge_state.increment_user_concurrency("1001");

        // 发送呼叫，应被拦截
        let call_id_2 = "antifraud-concurrency-2@example.com";
        let body = "v=0\r\n";
        let invite_2 = format!(
            concat!(
                "INVITE sip:13800138000@example.com SIP/2.0\r\n",
                "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-concur-2\r\n",
                "Max-Forwards: 70\r\n",
                "From: <sip:1001@example.com>;tag=from-tag-2\r\n",
                "To: <sip:13800138000@example.com>\r\n",
                "Call-ID: {call_id}\r\n",
                "CSeq: 1 INVITE\r\n",
                "Content-Type: application/sdp\r\n",
                "Content-Length: {}\r\n",
                "\r\n",
                "{}"
            ),
            body.len(),
            body,
            call_id = call_id_2
        );
        let datagrams_2 =
            handle_datagram(invite_2.as_bytes(), peer(), &edge_state, &edge_config()).await;
        
        // 应该直接拦截并返回 503
        assert_eq!(datagrams_2.len(), 1);
        assert_eq!(datagrams_2[0].target, "192.0.2.10:5060");
        let response = datagram_text(&datagrams_2[0]);
        assert!(response.starts_with("SIP/2.0 503 Service Unavailable"));
        assert!(response.contains("X-VOS-RS-Error: User concurrent call limit exceeded"));
    }

    #[tokio::test]
    async fn test_parallel_ringing_call_forking() {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let edge_state = EdgeState::new(
            CallManager::new(
                RouteTable::new(vec![
                    Route::new("route1", "", 100, RouteTarget::new("gw1", "gw1.example.com", Some(5060))),
                    Route::new("route2", "", 100, RouteTarget::new("gw2", "gw2.example.com", Some(5060))),
                ]),
                tx,
            ),
        );

        let invite = concat!(
            "INVITE sip:13800138000@example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-forking\r\n",
            "Max-Forwards: 70\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:13800138000@example.com>\r\n",
            "Call-ID: forking-call-1@example.com\r\n",
            "CSeq: 1 INVITE\r\n",
            "X-Forking-Enabled: true\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        );

        let datagrams = handle_datagram(invite.as_bytes(), peer(), &edge_state, &edge_config()).await;
        assert_eq!(datagrams.len(), 3);
        assert_eq!(datagrams[0].target, peer().to_string());
        
        let mut branches = Vec::new();
        for d in &datagrams[1..3] {
            let invite_txt = datagram_text(d);
            let call_id = invite_txt
                .lines()
                .find(|l| l.starts_with("Call-ID:"))
                .unwrap()
                .split_whitespace()
                .nth(1)
                .unwrap()
                .to_string();
            branches.push((call_id, d.target.clone()));
        }

        {
            let health = edge_state.gateway_health.lock().unwrap();
            assert_eq!(health.health("gw1").map(|h| h.active_calls()).unwrap_or(0), 1);
            assert_eq!(health.health("gw2").map(|h| h.active_calls()).unwrap_or(0), 1);
        }

        let branch_1_call_id = &branches[0].0;
        let ok_200 = format!(
            "SIP/2.0 200 OK\r\n\
             Via: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-vosrs\r\n\
             From: <sip:1001@example.com>;tag=from-tag\r\n\
             To: <sip:13800138000@example.com>;tag=gw-tag-1\r\n\
             Call-ID: {branch_call_id}\r\n\
             CSeq: 1 INVITE\r\n\
             Content-Length: 0\r\n\r\n",
            branch_call_id = branch_1_call_id
        );

        let res_datagrams = handle_datagram(
            ok_200.as_bytes(),
            "198.51.100.20:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        assert_eq!(res_datagrams.len(), 2);
        assert_eq!(res_datagrams[0].target, peer().to_string());
        
        let cancel_datagram = &res_datagrams[1];
        assert_eq!(cancel_datagram.target, branches[1].1);
        let cancel_txt = datagram_text(cancel_datagram);
        assert!(cancel_txt.starts_with("CANCEL "));
        assert!(cancel_txt.contains(&format!("Call-ID: {}", branches[1].0)));

        {
            let health = edge_state.gateway_health.lock().unwrap();
            let winning_gw = if branches[0].1.contains("gw1") { "gw1" } else { "gw2" };
            let canceled_gw = if branches[1].1.contains("gw1") { "gw1" } else { "gw2" };
            assert_eq!(health.health(winning_gw).map(|h| h.active_calls()).unwrap_or(0), 1);
            assert_eq!(health.health(canceled_gw).map(|h| h.active_calls()).unwrap_or(0), 0);
        }
    }
