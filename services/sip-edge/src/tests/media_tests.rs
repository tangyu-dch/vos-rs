    #[tokio::test]
    async fn rewrites_invite_sdp_offer_for_gateway_media_relay() {
        let edge_state = state_with_default_route();
        let body = concat!(
            "v=0\r\n",
            "o=caller 1 1 IN IP4 192.0.2.10\r\n",
            "s=caller\r\n",
            "c=IN IP4 192.0.2.10\r\n",
            "t=0 0\r\n",
            "m=audio 49170 RTP/AVP 0 8 101\r\n",
            "a=rtpmap:0 PCMU/8000\r\n",
            "a=rtpmap:8 PCMA/8000\r\n",
            "a=rtpmap:101 telephone-event/8000\r\n"
        );
        let request = format!(
            concat!(
                "INVITE sip:13800138000@example.com SIP/2.0\r\n",
                "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-sdp-offer\r\n",
                "Max-Forwards: 70\r\n",
                "From: <sip:1001@example.com>;tag=from-tag\r\n",
                "To: <sip:13800138000@example.com>\r\n",
                "Call-ID: invite-sdp-offer@example.com\r\n",
                "CSeq: 1 INVITE\r\n",
                "Content-Type: application/sdp\r\n",
                "Content-Length: {}\r\n",
                "\r\n",
                "{}"
            ),
            body.len(),
            body
        );

        let port_min = get_test_port_min();
        let datagrams =
            handle_datagram(request.as_bytes(), peer(), &edge_state, &edge_config()).await;
        assert_eq!(datagrams.len(), 2);

        let outbound_invite = datagram_text(&datagrams[1]);
        assert!(outbound_invite.contains("c=IN IP4 203.0.113.10\r\n"));
        assert!(outbound_invite.contains(&format!("m=audio {} RTP/AVP 0 8 101\r\n", port_min)));
        assert!(outbound_invite.contains("a=rtpmap:0 PCMU/8000\r\n"));
        assert!(outbound_invite.contains("a=rtpmap:8 PCMA/8000\r\n"));
        assert!(outbound_invite.contains("a=rtpmap:101 telephone-event/8000\r\n"));

        // DashMap get returns Ref which owns the lock
        let transaction = edge_state.inbound_transactions.get("invite-sdp-offer@example.com")
            .expect("transaction should be remembered");
        assert_eq!(
            transaction.caller_rtp,
            Some(RtpEndpoint::new("192.0.2.10", 49170))
        );
        assert_eq!(
            transaction.gateway_relay_rtp,
            Some(RtpEndpoint::new("203.0.113.10", port_min))
        );
        assert_eq!(
            edge_state.media_relay.target_for_port(port_min),
            Some("192.0.2.10:49170".parse().unwrap())
        );
    }

    #[tokio::test]
    async fn rewrites_gateway_answer_sdp_for_caller_media_relay() {
        let edge_state = state_with_default_route();
        send_invite(&edge_state, "invite-sdp-answer@example.com").await;
        let body = concat!(
            "v=0\r\n",
            "o=gateway 1 1 IN IP4 198.51.100.20\r\n",
            "s=gateway\r\n",
            "c=IN IP4 198.51.100.20\r\n",
            "t=0 0\r\n",
            "m=audio 49172 RTP/AVP 0\r\n",
            "a=rtpmap:0 PCMU/8000\r\n"
        );
        let gateway_response = format!(
            concat!(
                "SIP/2.0 200 OK\r\n",
                "Via: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-vosrs\r\n",
                "From: <sip:1001@example.com>;tag=from-tag\r\n",
                "To: <sip:13800138000@example.com>;tag=gw-tag\r\n",
                "Call-ID: invite-sdp-answer@example.com\r\n",
                "CSeq: 1 INVITE\r\n",
                "Content-Type: application/sdp\r\n",
                "Content-Length: {}\r\n",
                "\r\n",
                "{}"
            ),
            body.len(),
            body
        );

        let datagrams = handle_datagram(
            gateway_response.as_bytes(),
            "198.51.100.20:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        assert_eq!(datagrams.len(), 1);
        let response = datagram_text(&datagrams[0]);
        let port_min = get_test_port_min();
        assert!(response.contains("c=IN IP4 203.0.113.10\r\n"));
        assert!(response.contains(&format!("m=audio {} RTP/AVP 0\r\n", port_min)));

        // DashMap get returns Ref which owns the lock
        let transaction = edge_state.inbound_transactions.get("invite-sdp-answer@example.com")
            .expect("transaction should be remembered");
        assert_eq!(
            transaction.gateway_rtp,
            Some(RtpEndpoint::new("198.51.100.20", 49172))
        );
        assert_eq!(
            transaction.caller_relay_rtp,
            Some(RtpEndpoint::new("203.0.113.10", port_min))
        );
        assert_eq!(
            edge_state.media_relay.target_for_port(port_min),
            Some("198.51.100.20:49172".parse().unwrap())
        );
    }

    #[tokio::test]
    async fn test_media_hold_renegotiation() {
        let edge_state = state_with_default_route();

        // Establish media targets
        let ep = edge_state
            .media_relay
            .allocate_endpoint(&edge_config().media)
            .unwrap();

        // Register 0.0.0.0 (hold target)
        let hold_target: SocketAddr = "0.0.0.0:0".parse().unwrap();
        edge_state.media_relay.set_target_addr(ep.port, hold_target);

        // Verify that target_for_port returns 0.0.0.0
        assert_eq!(
            edge_state.media_relay.target_for_port(ep.port),
            Some(hold_target)
        );

        // Now simulate sending a media packet to target.
        let socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        edge_state.set_socket(Arc::clone(&socket));

        let raw_sdp = concat!(
            "v=0\r\n",
            "c=IN IP4 0.0.0.0\r\n",
            "m=audio 0 RTP/AVP 0 8\r\n",
            "a=sendonly\r\n"
        );
        let parsed_endpoint = media::parse_sdp_rtp_endpoint(raw_sdp.as_bytes()).unwrap();
        assert_eq!(parsed_endpoint.address, "0.0.0.0");
        assert_eq!(parsed_endpoint.port, 0);

        let rewritten =
            media::rewrite_sdp_body(raw_sdp.as_bytes(), RtpEndpoint::new("127.0.0.1", 40000))
                .unwrap();
        let rewritten_str = std::str::from_utf8(&rewritten).unwrap();
        assert!(rewritten_str.contains("a=sendonly\r\n"));
        assert!(rewritten_str.contains("c=IN IP4 127.0.0.1\r\n"));
        assert!(rewritten_str.contains("m=audio 40000 RTP/AVP 0 8\r\n"));
    }

    #[tokio::test]
    async fn test_mid_dialog_reinvite_sdp_rewrite() {
        let edge_state = state_with_default_route();
        let call_id = "mid-dialog-sdp-test@example.com";

        // 1. Setup call with initial SDP offer
        let offer_body = concat!(
            "v=0\r\n",
            "o=caller 1 1 IN IP4 192.0.2.10\r\n",
            "s=caller\r\n",
            "c=IN IP4 192.0.2.10\r\n",
            "t=0 0\r\n",
            "m=audio 49170 RTP/AVP 0\r\n",
            "a=rtpmap:0 PCMU/8000\r\n"
        );
        let invite = format!(
            "INVITE sip:13800138000@example.com SIP/2.0\r\n\
             Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-init-invite\r\n\
             Max-Forwards: 70\r\n\
             From: <sip:1001@example.com>;tag=from-tag\r\n\
             To: <sip:13800138000@example.com>\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 1 INVITE\r\n\
             Content-Type: application/sdp\r\n\
             Content-Length: {len}\r\n\r\n\
             {body}",
            call_id = call_id,
            len = offer_body.len(),
            body = offer_body
        );

        let invite_datagrams =
            handle_datagram(invite.as_bytes(), peer(), &edge_state, &edge_config()).await;
        assert_eq!(invite_datagrams.len(), 2);

        // 2. Answer with 200 OK containing gateway SDP answer
        let answer_body = concat!(
            "v=0\r\n",
            "o=gateway 1 1 IN IP4 198.51.100.20\r\n",
            "s=gateway\r\n",
            "c=IN IP4 198.51.100.20\r\n",
            "t=0 0\r\n",
            "m=audio 49172 RTP/AVP 0\r\n",
            "a=rtpmap:0 PCMU/8000\r\n"
        );
        let response_200 = format!(
            "SIP/2.0 200 OK\r\n\
             Via: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-vosrs\r\n\
             From: <sip:1001@example.com>;tag=from-tag\r\n\
             To: <sip:13800138000@example.com>;tag=gw-tag\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 1 INVITE\r\n\
             Content-Type: application/sdp\r\n\
             Content-Length: {len}\r\n\r\n\
             {body}",
            call_id = call_id,
            len = answer_body.len(),
            body = answer_body
        );

        let answer_datagrams = handle_datagram(
            response_200.as_bytes(),
            "198.51.100.20:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;
        assert_eq!(answer_datagrams.len(), 1);

        // Verify initial relay endpoints are set up in the transaction
        let (caller_relay, gw_relay) = {
            let transaction = edge_state.inbound_transactions.get(call_id).unwrap();
            (
                transaction.caller_relay_rtp.clone().unwrap(),
                transaction.gateway_relay_rtp.clone().unwrap(),
            )
        };

        // 3. Simulate mid-dialog Re-INVITE (Call Hold / SDP renegotiation) from caller
        let hold_body = concat!(
            "v=0\r\n",
            "o=caller 1 2 IN IP4 192.0.2.10\r\n",
            "s=caller\r\n",
            "c=IN IP4 0.0.0.0\r\n", // Call Hold IP
            "t=0 0\r\n",
            "m=audio 49170 RTP/AVP 0\r\n",
            "a=rtpmap:0 PCMU/8000\r\n",
            "a=sendonly\r\n"
        );
        let reinvite = format!(
            "INVITE sip:13800138000@example.com SIP/2.0\r\n\
             Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-reinvite\r\n\
             Max-Forwards: 70\r\n\
             From: <sip:1001@example.com>;tag=from-tag\r\n\
             To: <sip:13800138000@example.com>;tag=gw-tag\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 2 INVITE\r\n\
             Content-Type: application/sdp\r\n\
             Content-Length: {len}\r\n\r\n\
             {body}",
            call_id = call_id,
            len = hold_body.len(),
            body = hold_body
        );

        let reinvite_datagrams =
            handle_datagram(reinvite.as_bytes(), peer(), &edge_state, &edge_config()).await;
        assert_eq!(reinvite_datagrams.len(), 1);

        // Verify the outgoing Re-INVITE has rewritten SDP presenting gw_relay (reusing the same port!)
        let forwarded_reinvite = datagram_text(&reinvite_datagrams[0]);
        assert!(forwarded_reinvite.contains(&format!("m=audio {} RTP/AVP 0\r\n", gw_relay.port)));

        // Verify target for gw_relay is updated to caller's new IP (0.0.0.0:49170)
        assert_eq!(
            edge_state.media_relay.target_for_port(gw_relay.port),
            Some("0.0.0.0:49170".parse().unwrap())
        );

        // 4. Gateway responds with 200 OK (renegotiation answer)
        let hold_answer_body = concat!(
            "v=0\r\n",
            "o=gateway 1 2 IN IP4 198.51.100.20\r\n",
            "s=gateway\r\n",
            "c=IN IP4 198.51.100.20\r\n",
            "t=0 0\r\n",
            "m=audio 49172 RTP/AVP 0\r\n",
            "a=rtpmap:0 PCMU/8000\r\n",
            "a=recvonly\r\n"
        );
        let reinvite_resp_200 = format!(
            "SIP/2.0 200 OK\r\n\
             Via: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-reinvite\r\n\
             From: <sip:1001@example.com>;tag=from-tag\r\n\
             To: <sip:13800138000@example.com>;tag=gw-tag\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 2 INVITE\r\n\
             Content-Type: application/sdp\r\n\
             Content-Length: {len}\r\n\r\n\
             {body}",
            call_id = call_id,
            len = hold_answer_body.len(),
            body = hold_answer_body
        );

        let reinvite_resp_datagrams = handle_datagram(
            reinvite_resp_200.as_bytes(),
            "198.51.100.20:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;
        assert_eq!(reinvite_resp_datagrams.len(), 1);

        // Verify the outgoing response to caller has rewritten SDP presenting caller_relay (reusing the same port!)
        let forwarded_resp = datagram_text(&reinvite_resp_datagrams[0]);
        assert!(forwarded_resp.contains(&format!("m=audio {} RTP/AVP 0\r\n", caller_relay.port)));

        // Verify target for caller_relay is still gateway target (198.51.100.20:49172)
        assert_eq!(
            edge_state.media_relay.target_for_port(caller_relay.port),
            Some("198.51.100.20:49172".parse().unwrap())
        );
    }
