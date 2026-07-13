    #[tokio::test]
    async fn replies_to_options() {
        let edge_state = state_without_routes();
        let request = concat!(
            "OPTIONS sip:edge.example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-1\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:edge.example.com>\r\n",
            "Call-ID: options-1@example.com\r\n",
            "CSeq: 1 OPTIONS\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        );

        let datagrams =
            handle_datagram(request.as_bytes(), peer(), &edge_state, &edge_config()).await;
        assert_eq!(datagrams.len(), 1);

        let response = datagram_text(&datagrams[0]);

        assert_eq!(datagrams[0].target, "192.0.2.10:5060");
        assert!(response.starts_with("SIP/2.0 200 OK\r\n"));
        assert!(response.contains("Allow: REGISTER, INVITE, ACK, BYE, CANCEL, OPTIONS, INFO\r\n"));
        assert!(response.contains("To: <sip:edge.example.com>;tag=vosrs-edge\r\n"));
    }

    #[tokio::test]
    async fn invite_to_registered_contact_bypasses_gateway_routes() {
        let edge_state = state_with_default_route();
        register_contact(&edge_state, "1002", "192.0.2.20", 5070).await;
        let invite = concat!(
            "INVITE sip:1002@example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-internal\r\n",
            "Max-Forwards: 70\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:1002@example.com>\r\n",
            "Call-ID: invite-internal@example.com\r\n",
            "CSeq: 1 INVITE\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        );

        let datagrams =
            handle_datagram(invite.as_bytes(), peer(), &edge_state, &edge_config()).await;

        assert_eq!(datagrams.len(), 2);
        assert_eq!(datagrams[0].target, "192.0.2.10:5060");
        assert_eq!(datagrams[1].target, "192.0.2.20:5070");

        let trying = datagram_text(&datagrams[0]);
        assert!(trying.starts_with("SIP/2.0 100 Trying\r\n"));

        let outbound_invite = datagram_text(&datagrams[1]);
        assert!(
            outbound_invite
                .starts_with("INVITE sip:1002@192.0.2.20:5070;transport=udp SIP/2.0\r\n"),
            "{outbound_invite}"
        );
        assert!(!outbound_invite.contains("gw1.example.com"));
    }

    #[tokio::test]
    async fn invite_to_registered_contact_works_without_default_route() {
        let edge_state = state_without_routes();
        register_contact(&edge_state, "1002", "192.0.2.20", 5070).await;
        let invite = concat!(
            "INVITE sip:1002@example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-internal-no-route\r\n",
            "Max-Forwards: 70\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:1002@example.com>\r\n",
            "Call-ID: invite-internal-no-route@example.com\r\n",
            "CSeq: 1 INVITE\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        );

        let datagrams =
            handle_datagram(invite.as_bytes(), peer(), &edge_state, &edge_config()).await;

        assert_eq!(datagrams.len(), 2);
        assert_eq!(datagrams[1].target, "192.0.2.20:5070");

        let guard = &edge_state.call_manager;
        let call = guard
            .get(&CallId::new("invite-internal-no-route@example.com"))
            .expect("call should be stored");
        assert_eq!(call.state, CallState::Routing);
        assert_eq!(
            call.outbound.as_ref().unwrap().remote_uri.to_string(),
            "sip:1002@192.0.2.20:5070;transport=udp"
        );
    }

    #[tokio::test]
    async fn replies_to_invite_with_trying_and_dispatches_outbound_invite() {
        let edge_state = state_with_default_route();
        let request = concat!(
            "INVITE sip:13800138000@example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-2\r\n",
            "Max-Forwards: 70\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:13800138000@example.com>\r\n",
            "Call-ID: invite-1@example.com\r\n",
            "CSeq: 1 INVITE\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        );

        let datagrams =
            handle_datagram(request.as_bytes(), peer(), &edge_state, &edge_config()).await;
        assert_eq!(datagrams.len(), 2);

        let response = datagram_text(&datagrams[0]);
        assert_eq!(datagrams[0].target, "192.0.2.10:5060");
        assert!(response.starts_with("SIP/2.0 100 Trying\r\n"));
        assert!(response.contains("Call-ID: invite-1@example.com\r\n"));
        assert!(response.contains("CSeq: 1 INVITE\r\n"));
        assert!(response.contains("To: <sip:13800138000@example.com>;tag=vosrs-edge\r\n"));

        let outbound_invite = datagram_text(&datagrams[1]);
        assert_eq!(datagrams[1].target, "gw1.example.com:5060");
        assert!(outbound_invite
            .starts_with("INVITE sip:13800138000@gw1.example.com:5060;transport=udp SIP/2.0\r\n"));
        assert!(outbound_invite.contains("Via: SIP/2.0/UDP edge.example.com:5060;branch="));
        assert!(outbound_invite.contains("Max-Forwards: 69\r\n"));
        assert!(outbound_invite.contains("Contact: <sip:vosrs@edge.example.com:5060>\r\n"));
        assert_eq!(edge_state.call_manager.len(), 1);
    }

    #[tokio::test]
    async fn retransmitted_invite_replays_trying_without_duplicate_outbound_invite() {
        let edge_state = state_with_default_route();
        let request = concat!(
            "INVITE sip:13800138000@example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-retry-invite\r\n",
            "Max-Forwards: 70\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:13800138000@example.com>\r\n",
            "Call-ID: invite-retry@example.com\r\n",
            "CSeq: 1 INVITE\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        );

        let first = handle_datagram(request.as_bytes(), peer(), &edge_state, &edge_config()).await;
        let second = handle_datagram(request.as_bytes(), peer(), &edge_state, &edge_config()).await;

        assert_eq!(first.len(), 2);
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].target, "192.0.2.10:5060");

        let response = datagram_text(&second[0]);
        assert!(response.starts_with("SIP/2.0 100 Trying\r\n"));
        assert!(response.contains("Call-ID: invite-retry@example.com\r\n"));
        assert_eq!(edge_state.call_manager.len(), 1);
    }

    #[tokio::test]
    async fn invite_with_unsupported_audio_codec_receives_not_acceptable() {
        let edge_state = state_with_default_route();
        let body = concat!(
            "v=0\r\n",
            "o=caller 1 1 IN IP4 192.0.2.10\r\n",
            "s=caller\r\n",
            "c=IN IP4 192.0.2.10\r\n",
            "t=0 0\r\n",
            "m=audio 49170 RTP/AVP 101\r\n",
            "a=rtpmap:101 telephone-event/8000\r\n"
        );
        let request = format!(
            concat!(
                "INVITE sip:13800138000@example.com SIP/2.0\r\n",
                "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-sdp-unsupported\r\n",
                "Max-Forwards: 70\r\n",
                "From: <sip:1001@example.com>;tag=from-tag\r\n",
                "To: <sip:13800138000@example.com>\r\n",
                "Call-ID: invite-sdp-unsupported@example.com\r\n",
                "CSeq: 1 INVITE\r\n",
                "Content-Type: application/sdp\r\n",
                "Content-Length: {}\r\n",
                "\r\n",
                "{}"
            ),
            body.len(),
            body
        );

        let datagrams =
            handle_datagram(request.as_bytes(), peer(), &edge_state, &edge_config()).await;

        assert_eq!(datagrams.len(), 1);
        let response = datagram_text(&datagrams[0]);
        assert!(response.starts_with("SIP/2.0 488 Not Acceptable Here\r\n"));
        assert!(response.contains("X-VOS-RS-Error: missing compatible audio codec in SDP\r\n"));
    }

    #[tokio::test]
    async fn invite_with_newly_supported_codecs_passes() {
        let edge_state = state_with_default_route();
        let body = concat!(
            "v=0\r\n",
            "o=caller 1 1 IN IP4 192.0.2.10\r\n",
            "s=caller\r\n",
            "c=IN IP4 192.0.2.10\r\n",
            "t=0 0\r\n",
            "m=audio 49170 RTP/AVP 111 18\r\n",
            "a=rtpmap:111 opus/48000/2\r\n",
            "a=rtpmap:18 G729/8000\r\n"
        );
        let request = format!(
            concat!(
                "INVITE sip:13801380000@example.com SIP/2.0\r\n",
                "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-sdp-supported-opus\r\n",
                "Max-Forwards: 70\r\n",
                "From: <sip:1001@example.com>;tag=from-tag\r\n",
                "To: <sip:13801380000@example.com>\r\n",
                "Call-ID: invite-sdp-supported-opus@example.com\r\n",
                "CSeq: 1 INVITE\r\n",
                "Content-Type: application/sdp\r\n",
                "Content-Length: {}\r\n",
                "\r\n",
                "{}"
            ),
            body.len(),
            body
        );

        let datagrams =
            handle_datagram(request.as_bytes(), peer(), &edge_state, &edge_config()).await;

        assert!(!datagrams.is_empty());
        let response = datagram_text(&datagrams[0]);
        assert!(response.starts_with("SIP/2.0 100 Trying\r\n"));
    }

    #[tokio::test]
    async fn invite_with_exhausted_rtp_ports_receives_service_unavailable() {
        let edge_state = state_with_default_route();
        let edge_config = EdgeConfig {
            advertised_addr: "edge.example.com:5060".to_string(),
            database_url: None,
            nats_url: None,
            nats_cdr_stream: None,
            nats_cdr_subject: None,
            redis_url: None,
            media: MediaConfig::new("203.0.113.10", 31_000, 31_000),
            auth: AuthConfig::disabled(),
            session_expires_gateway: 600,
            session_expires_caller: 1800,
            sbc_allow_rules: Vec::new(),
            sbc_block_rules: Vec::new(),
            sbc_rate_limit_capacity: 100.0,
            sbc_rate_limit_fill_rate: 10.0,
            sbc_max_concurrency: 10,
            tls_cert_path: None,
            tls_key_path: None,
            tls_bind_addr: None,
            tls_allow_test_certificate: false,
            tls_ca_path: None,
            tls_insecure_skip_verify: false,
            tls_server_name: None,
            udp_workers: 1,
            udp_workers_auto: false,
            udp_receive_buffer_bytes: config::DEFAULT_UDP_BUFFER_BYTES,
            udp_send_buffer_bytes: config::DEFAULT_UDP_BUFFER_BYTES,
            ..Default::default()
        };
        let body = concat!(
            "v=0\r\n",
            "o=caller 1 1 IN IP4 192.0.2.10\r\n",
            "s=caller\r\n",
            "c=IN IP4 192.0.2.10\r\n",
            "t=0 0\r\n",
            "m=audio 49170 RTP/AVP 0\r\n",
            "a=rtpmap:0 PCMU/8000\r\n"
        );
        let first_invite = format!(
            concat!(
                "INVITE sip:13800138000@example.com SIP/2.0\r\n",
                "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-rtp-port-one\r\n",
                "Max-Forwards: 70\r\n",
                "From: <sip:1001@example.com>;tag=from-tag\r\n",
                "To: <sip:13800138000@example.com>\r\n",
                "Call-ID: invite-rtp-port-one@example.com\r\n",
                "CSeq: 1 INVITE\r\n",
                "Content-Type: application/sdp\r\n",
                "Content-Length: {}\r\n",
                "\r\n",
                "{}"
            ),
            body.len(),
            body
        );
        let first_datagrams =
            handle_datagram(first_invite.as_bytes(), peer(), &edge_state, &edge_config).await;
        assert_eq!(first_datagrams.len(), 2);
        assert!(datagram_text(&first_datagrams[1]).contains("m=audio 31000 RTP/AVP 0\r\n"));

        let second_invite = format!(
            concat!(
                "INVITE sip:13800138001@example.com SIP/2.0\r\n",
                "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-rtp-port-two\r\n",
                "Max-Forwards: 70\r\n",
                "From: <sip:1001@example.com>;tag=from-tag\r\n",
                "To: <sip:13800138001@example.com>\r\n",
                "Call-ID: invite-rtp-port-two@example.com\r\n",
                "CSeq: 1 INVITE\r\n",
                "Content-Type: application/sdp\r\n",
                "Content-Length: {}\r\n",
                "\r\n",
                "{}"
            ),
            body.len(),
            body
        );
        let second_datagrams =
            handle_datagram(second_invite.as_bytes(), peer(), &edge_state, &edge_config).await;

        assert_eq!(second_datagrams.len(), 1);
        let response = datagram_text(&second_datagrams[0]);
        assert!(response.starts_with("SIP/2.0 503 Service Unavailable\r\n"));
        assert!(response.contains("X-VOS-RS-Error: RTP port range exhausted: 31000-31000\r\n"));
    }

    #[tokio::test]
    async fn outbound_failure_releases_rtp_port_lease() {
        let edge_state = state_with_default_route();
        let edge_config = EdgeConfig {
            advertised_addr: "edge.example.com:5060".to_string(),
            database_url: None,
            nats_url: None,
            nats_cdr_stream: None,
            nats_cdr_subject: None,
            redis_url: None,
            media: MediaConfig::new("203.0.113.10", 32_000, 32_000),
            auth: AuthConfig::disabled(),
            session_expires_gateway: 600,
            session_expires_caller: 1800,
            sbc_allow_rules: Vec::new(),
            sbc_block_rules: Vec::new(),
            sbc_rate_limit_capacity: 100.0,
            sbc_rate_limit_fill_rate: 10.0,
            sbc_max_concurrency: 10,
            tls_cert_path: None,
            tls_key_path: None,
            tls_bind_addr: None,
            tls_allow_test_certificate: false,
            tls_ca_path: None,
            tls_insecure_skip_verify: false,
            tls_server_name: None,
            udp_workers: 1,
            udp_workers_auto: false,
            udp_receive_buffer_bytes: config::DEFAULT_UDP_BUFFER_BYTES,
            udp_send_buffer_bytes: config::DEFAULT_UDP_BUFFER_BYTES,
            ..Default::default()
        };
        let body = concat!(
            "v=0\r\n",
            "o=caller 1 1 IN IP4 192.0.2.10\r\n",
            "s=caller\r\n",
            "c=IN IP4 192.0.2.10\r\n",
            "t=0 0\r\n",
            "m=audio 49170 RTP/AVP 0\r\n",
            "a=rtpmap:0 PCMU/8000\r\n"
        );
        let first_invite = format!(
            concat!(
                "INVITE sip:13800138000@example.com SIP/2.0\r\n",
                "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-rtp-release-one\r\n",
                "Max-Forwards: 70\r\n",
                "From: <sip:1001@example.com>;tag=from-tag\r\n",
                "To: <sip:13800138000@example.com>\r\n",
                "Call-ID: invite-rtp-release-one@example.com\r\n",
                "CSeq: 1 INVITE\r\n",
                "Content-Type: application/sdp\r\n",
                "Content-Length: {}\r\n",
                "\r\n",
                "{}"
            ),
            body.len(),
            body
        );
        let first_datagrams =
            handle_datagram(first_invite.as_bytes(), peer(), &edge_state, &edge_config).await;
        assert_eq!(first_datagrams.len(), 2);

        let failure_response = concat!(
            "SIP/2.0 486 Busy Here\r\n",
            "Via: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-vosrs\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:13800138000@example.com>;tag=gw-tag\r\n",
            "Call-ID: invite-rtp-release-one@example.com\r\n",
            "CSeq: 1 INVITE\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        );
        let failure_datagrams = handle_datagram(
            failure_response.as_bytes(),
            "198.51.100.20:5060".parse().unwrap(),
            &edge_state,
            &edge_config,
        )
        .await;
        assert_eq!(failure_datagrams.len(), 1);
        assert!(datagram_text(&failure_datagrams[0]).starts_with("SIP/2.0 486 Busy Here\r\n"));

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let second_invite = format!(
            concat!(
                "INVITE sip:13800138001@example.com SIP/2.0\r\n",
                "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-rtp-release-two\r\n",
                "Max-Forwards: 70\r\n",
                "From: <sip:1001@example.com>;tag=from-tag\r\n",
                "To: <sip:13800138001@example.com>\r\n",
                "Call-ID: invite-rtp-release-two@example.com\r\n",
                "CSeq: 1 INVITE\r\n",
                "Content-Type: application/sdp\r\n",
                "Content-Length: {}\r\n",
                "\r\n",
                "{}"
            ),
            body.len(),
            body
        );
        let second_datagrams =
            handle_datagram(second_invite.as_bytes(), peer(), &edge_state, &edge_config).await;
        assert_eq!(second_datagrams.len(), 2);
        assert!(datagram_text(&second_datagrams[1]).contains("m=audio 32000 RTP/AVP 0\r\n"));
    }

    #[tokio::test]
    async fn invite_without_route_receives_not_found() {
        let edge_state = state_without_routes();
        let request = concat!(
            "INVITE sip:13900139000@example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-4\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:13900139000@example.com>\r\n",
            "Call-ID: invite-no-route@example.com\r\n",
            "CSeq: 1 INVITE\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        );

        let datagrams =
            handle_datagram(request.as_bytes(), peer(), &edge_state, &edge_config()).await;
        assert_eq!(datagrams.len(), 1);

        let response = datagram_text(&datagrams[0]);
        assert!(response.starts_with("SIP/2.0 404 Not Found\r\n"));
        assert!(response.contains("X-VOS-RS-Error: no route for destination: 13900139000\r\n"));
        assert_eq!(edge_state.call_manager.len(), 1);
    }

    #[tokio::test]
    async fn invalid_invite_receives_bad_request() {
        let edge_state = state_with_default_route();
        let request = concat!(
            "INVITE sip:13800138000@example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-3\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:13800138000@example.com>\r\n",
            "CSeq: 1 INVITE\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        );

        let datagrams =
            handle_datagram(request.as_bytes(), peer(), &edge_state, &edge_config()).await;
        assert_eq!(datagrams.len(), 1);

        let response = datagram_text(&datagrams[0]);
        assert!(response.starts_with("SIP/2.0 400 Bad Request\r\n"));
        assert!(response.contains("X-VOS-RS-Error: missing required SIP header: Call-ID\r\n"));
    }

    #[tokio::test]
    async fn test_out_of_dialog_message_routing_to_registered_contact() {
        let edge_state = state_with_default_route();

        // 1. Register contact 1001
        let register = "REGISTER sip:example.com SIP/2.0\r\n\
             Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-reg\r\n\
             From: <sip:1001@example.com>;tag=from-tag\r\n\
             To: <sip:1001@example.com>\r\n\
             Call-ID: reg-message-01\r\n\
             CSeq: 1 REGISTER\r\n\
             Contact: <sip:1001@192.0.2.10:5070;transport=udp>;expires=60\r\n\
             Content-Length: 0\r\n\r\n";
        let _ = handle_datagram(
            register.as_bytes(),
            "192.0.2.10:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // 2. Send MESSAGE from 1002 to 1001
        let call_id = "msg-001";
        let message_req = format!(
            "MESSAGE sip:1001@example.com SIP/2.0\r\n\
             Via: SIP/2.0/UDP 192.0.2.20:5060;branch=z9hG4bK-msg-1\r\n\
             From: <sip:1002@example.com>;tag=from-tag\r\n\
             To: <sip:1001@example.com>\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 1 MESSAGE\r\n\
             Content-Type: text/plain\r\n\
             Content-Length: 5\r\n\r\n\
             hello"
        );

        let datagrams = handle_datagram(
            message_req.as_bytes(),
            "192.0.2.20:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // Verify MESSAGE is forwarded to 1001's registered contact
        assert_eq!(datagrams.len(), 1);
        assert_eq!(datagrams[0].target, "192.0.2.10:5060");
        let forwarded_msg = datagram_text(&datagrams[0]);
        assert!(
            forwarded_msg.starts_with("MESSAGE sip:1001@192.0.2.10:5070;transport=udp SIP/2.0\r\n")
        );
        assert!(forwarded_msg.contains("\r\n\r\nhello"));

        // Check transaction registered
        {
            assert!(edge_state.inbound_transactions.contains_key(call_id));
        }

        // 3. Receive 200 OK from 1001
        let ok_200 = format!(
            "SIP/2.0 200 OK\r\n\
             Via: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-msg-1\r\n\
             Via: SIP/2.0/UDP 192.0.2.20:5060;branch=z9hG4bK-msg-1\r\n\
             From: <sip:1002@example.com>;tag=from-tag\r\n\
             To: <sip:1001@example.com>;tag=to-tag\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 1 MESSAGE\r\n\
             Content-Length: 0\r\n\r\n"
        );

        let response_datagrams = handle_datagram(
            ok_200.as_bytes(),
            "192.0.2.10:5070".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // Verify 200 OK forwarded back to sender 1002
        assert_eq!(response_datagrams.len(), 1);
        assert_eq!(response_datagrams[0].target, "192.0.2.20:5060");
        let forwarded_ok = datagram_text(&response_datagrams[0]);
        assert!(forwarded_ok.starts_with("SIP/2.0 200 OK\r\n"));

        // Check transaction cleaned up
        {
            assert!(!edge_state.inbound_transactions.contains_key(call_id));
        }
    }

    #[tokio::test]
    async fn test_out_of_dialog_message_routing_to_gateway() {
        let edge_state = state_with_default_route();

        // Send MESSAGE targeting an unregistered destination (so it falls back to the default route gateway)
        let call_id = "msg-gw-01";
        let message_req = format!(
            "MESSAGE sip:13800138000@example.com SIP/2.0\r\n\
             Via: SIP/2.0/UDP 192.0.2.20:5060;branch=z9hG4bK-msg-gw\r\n\
             From: <sip:1002@example.com>;tag=from-tag\r\n\
             To: <sip:13800138000@example.com>\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 1 MESSAGE\r\n\
             Content-Type: text/plain\r\n\
             Content-Length: 5\r\n\r\n\
             hello"
        );

        let datagrams = handle_datagram(
            message_req.as_bytes(),
            "192.0.2.20:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // Verify MESSAGE is forwarded to default gateway (gw1.example.com:5060)
        assert_eq!(datagrams.len(), 1);
        assert_eq!(datagrams[0].target, "gw1.example.com:5060");
        let forwarded_msg = datagram_text(&datagrams[0]);
        assert!(forwarded_msg
            .starts_with("MESSAGE sip:13800138000@gw1.example.com:5060;transport=udp SIP/2.0\r\n"));
        assert!(forwarded_msg.contains("\r\n\r\nhello"));
    }

    #[tokio::test]
    async fn test_graceful_shutdown_draining_invite_receives_503() {
        let edge_state = state_with_default_route();

        // 1. Enable draining
        edge_state.draining.store(true, Ordering::Relaxed);

        // 2. Send INVITE request
        let invite = "INVITE sip:13800138000@example.com SIP/2.0\r\n\
                      Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-invite-draining\r\n\
                      Max-Forwards: 70\r\n\
                      From: <sip:1001@example.com>;tag=from-tag\r\n\
                      To: <sip:13800138000@example.com>\r\n\
                      Call-ID: draining-invite@example.com\r\n\
                      CSeq: 1 INVITE\r\n\
                      Content-Length: 0\r\n\r\n";
        let datagrams =
            handle_datagram(invite.as_bytes(), peer(), &edge_state, &edge_config()).await;
        assert_eq!(datagrams.len(), 1);
        let resp = String::from_utf8_lossy(&datagrams[0].bytes);
        assert!(resp.starts_with("SIP/2.0 503 Service Unavailable\r\n"));
        assert!(resp.contains("Retry-After: 30\r\n"));

        // 3. Send OPTIONS request (should still be processed normally during drain)
        let options = "OPTIONS sip:edge@example.com SIP/2.0\r\n\
                       Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-options-draining\r\n\
                       Max-Forwards: 70\r\n\
                       From: <sip:1001@example.com>;tag=from-tag\r\n\
                       To: <sip:edge@example.com>\r\n\
                       Call-ID: draining-options@example.com\r\n\
                       CSeq: 1 OPTIONS\r\n\
                       Content-Length: 0\r\n\r\n";
        let datagrams =
            handle_datagram(options.as_bytes(), peer(), &edge_state, &edge_config()).await;
        assert_eq!(datagrams.len(), 1);
        let resp = String::from_utf8_lossy(&datagrams[0].bytes);
        assert!(resp.starts_with("SIP/2.0 200 OK\r\n"));
    }
