    #[tokio::test]
    async fn test_sbc_ip_acl() {
        let mut config = edge_config();
        config.sbc_allow_rules = vec!["192.0.2.0/24".to_string()];
        config.sbc_block_rules = vec!["192.0.2.100".to_string()];
        let edge_state = state_with_default_route_and_config(&config);

        let packet = b"OPTIONS sip:edge.example.com SIP/2.0\r\n\r\n";
        let d1 = handle_datagram(
            packet,
            "192.0.2.50:5060".parse().unwrap(),
            &edge_state,
            &config,
        )
        .await;
        assert!(!d1.is_empty());

        let d2 = handle_datagram(
            packet,
            "192.0.2.100:5060".parse().unwrap(),
            &edge_state,
            &config,
        )
        .await;
        assert!(d2.is_empty());

        let d3 = handle_datagram(
            packet,
            "10.0.0.1:5060".parse().unwrap(),
            &edge_state,
            &config,
        )
        .await;
        assert!(d3.is_empty());
    }

    #[tokio::test]
    async fn test_sbc_cps_rate_limiting() {
        let mut config = edge_config();
        config.sbc_rate_limit_capacity = 2.0;
        config.sbc_rate_limit_fill_rate = 0.0;
        let edge_state = state_with_default_route_and_config(&config);

        let packet = b"OPTIONS sip:edge.example.com SIP/2.0\r\n\r\n";
        let peer_addr = "192.0.2.50:5060".parse().unwrap();

        let d1 = handle_datagram(packet, peer_addr, &edge_state, &config).await;
        assert!(!d1.is_empty());
        assert!(datagram_text(&d1[0]).starts_with("SIP/2.0 200 OK\r\n"));

        let d2 = handle_datagram(packet, peer_addr, &edge_state, &config).await;
        assert!(!d2.is_empty());
        assert!(datagram_text(&d2[0]).starts_with("SIP/2.0 200 OK\r\n"));

        let d3 = handle_datagram(packet, peer_addr, &edge_state, &config).await;
        assert!(!d3.is_empty());
        assert!(datagram_text(&d3[0])
            .starts_with("SIP/2.0 503 Service Unavailable - Rate Limit Exceeded\r\n"));
    }

    #[tokio::test]
    async fn test_sbc_concurrency_limiting() {
        let mut config = edge_config();
        config.sbc_max_concurrency = 1;
        let edge_state = state_with_default_route_and_config(&config);

        let call_id_1 = "call-concurrent-1";
        let body = sdp_body();
        let invite_1 = format!(
            concat!(
                "INVITE sip:13800138000@example.com SIP/2.0\r\n",
                "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-conc1\r\n",
                "From: <sip:1001@example.com>;tag=from-tag1\r\n",
                "To: <sip:13800138000@example.com>\r\n",
                "Call-ID: {call_id}\r\n",
                "CSeq: 1 INVITE\r\n",
                "Content-Length: {}\r\n\r\n",
                "{}"
            ),
            body.len(),
            body,
            call_id = call_id_1
        );

        let d1 = handle_datagram(
            invite_1.as_bytes(),
            "192.0.2.10:5060".parse().unwrap(),
            &edge_state,
            &config,
        )
        .await;
        assert_eq!(d1.len(), 2);

        let ok_200 = format!(
            "SIP/2.0 200 OK\r\n\
             Via: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-vosrs\r\n\
             Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-conc1\r\n\
             From: <sip:1001@example.com>;tag=from-tag1\r\n\
             To: <sip:13800138000@example.com>;tag=gw-tag\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 1 INVITE\r\n\
             Content-Length: 0\r\n\r\n",
            call_id = call_id_1
        );
        let _ = handle_datagram(
            ok_200.as_bytes(),
            "198.51.100.20:5060".parse().unwrap(),
            &edge_state,
            &config,
        )
        .await;

        let call_id_2 = "call-concurrent-2";
        let invite_2 = format!(
            concat!(
                "INVITE sip:13800138000@example.com SIP/2.0\r\n",
                "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-conc2\r\n",
                "From: <sip:1001@example.com>;tag=from-tag2\r\n",
                "To: <sip:13800138000@example.com>\r\n",
                "Call-ID: {call_id}\r\n",
                "CSeq: 1 INVITE\r\n",
                "Content-Length: {}\r\n\r\n",
                "{}"
            ),
            body.len(),
            body,
            call_id = call_id_2
        );

        let d2 = handle_datagram(
            invite_2.as_bytes(),
            "192.0.2.10:5060".parse().unwrap(),
            &edge_state,
            &config,
        )
        .await;
        assert_eq!(d2.len(), 1);
        let resp = datagram_text(&d2[0]);
        assert!(resp.starts_with("SIP/2.0 486 Busy Here - Concurrency Limit Exceeded\r\n"));
    }
