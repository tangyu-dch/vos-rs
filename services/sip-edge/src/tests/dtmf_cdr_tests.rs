    #[tokio::test]
    async fn flush_completed_cdrs_discards_when_postgres_is_disabled() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let edge_state = EdgeState::new(CallManager::new(RouteTable::new(vec![Route::new(
            "default",
            "",
            100,
            RouteTarget::new("gw1", "gw1.example.com", Some(5060)),
        )]), tx));
        send_invite(&edge_state, "invite-7@example.com").await;
        send_gateway_ok(&edge_state, "invite-7@example.com").await;

        let bye = concat!(
            "BYE sip:13800138000@example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-bye\r\n",
            "Max-Forwards: 70\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:13800138000@example.com>;tag=gw-tag\r\n",
            "Call-ID: invite-7@example.com\r\n",
            "CSeq: 2 BYE\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        );

        let _ = handle_datagram(bye.as_bytes(), peer(), &edge_state, &edge_config()).await;
        let cdr = rx.try_recv().expect("CDR should exist");
        let cdrs = vec![cdr];
        flush_cdr_batch(&CdrSinks::default(), &cdrs)
            .await
            .unwrap();

        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn test_cdr_mos_persistence() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let edge_state = EdgeState::new(CallManager::new(RouteTable::new(vec![Route::new(
            "default",
            "",
            100,
            RouteTarget::new("gw1", "gw1.example.com", Some(5060)),
        )]), tx));
        let call_id = "mos-test@example.com";

        // 1. Setup call
        let invite = format!(
            "INVITE sip:13800138000@example.com SIP/2.0\r\n\
             Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-mos-invite\r\n\
             Max-Forwards: 70\r\n\
             From: <sip:1001@example.com>;tag=from-tag\r\n\
             To: <sip:13800138000@example.com>\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 1 INVITE\r\n\
             Content-Length: 0\r\n\r\n",
            call_id = call_id
        );

        let _ = handle_datagram(invite.as_bytes(), peer(), &edge_state, &edge_config()).await;

        // 2. Answer call
        let response_200 = format!(
            "SIP/2.0 200 OK\r\n\
             Via: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-vosrs\r\n\
             From: <sip:1001@example.com>;tag=from-tag\r\n\
             To: <sip:13800138000@example.com>;tag=gw-tag\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 1 INVITE\r\n\
             Content-Length: 0\r\n\r\n",
            call_id = call_id
        );

        let _ = handle_datagram(
            response_200.as_bytes(),
            "198.51.100.20:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // Before sending BYE, set some mock metrics in the media relay
        {
            let mut transaction = edge_state.inbound_transactions.get_mut(call_id).unwrap();

            // Assign dummy caller and gateway endpoints to simulate media ports
            let caller_endpoint = RtpEndpoint::new("127.0.0.1", 45000);
            let gateway_endpoint = RtpEndpoint::new("127.0.0.1", 45002);
            transaction.caller_relay_rtp = Some(caller_endpoint.clone());
            transaction.gateway_relay_rtp = Some(gateway_endpoint.clone());

            // Write dummy RTCP reports into the media relay state
            let rtcp_caller = super::media::RtcpQualitySnapshot {
                max_fraction_lost: Some(1), // 1/256 ≈ 0.4%
                max_rtt_ms: Some(40),
                max_jitter: Some(16), // 16 / 8 = 2ms
                ..Default::default()
            };

            let rtcp_gateway = super::media::RtcpQualitySnapshot {
                max_fraction_lost: Some(1), // 1/256 ≈ 0.4%
                max_rtt_ms: Some(20),
                max_jitter: Some(8), // 8 / 8 = 1ms
                ..Default::default()
            };

            edge_state
                .media_relay
                .record_rtcp_reports_for_test(caller_endpoint.port, rtcp_caller);
            edge_state
                .media_relay
                .record_rtcp_reports_for_test(gateway_endpoint.port, rtcp_gateway);
        };

        // 3. Collect mock metrics and terminate via BYE
        let bye = format!(
            "BYE sip:13800138000@example.com SIP/2.0\r\n\
             Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-mos-bye\r\n\
             From: <sip:1001@example.com>;tag=from-tag\r\n\
             To: <sip:13800138000@example.com>;tag=gw-tag\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 2 BYE\r\n\
             Content-Length: 0\r\n\r\n",
            call_id = call_id
        );

        let datagrams = handle_datagram(bye.as_bytes(), peer(), &edge_state, &edge_config()).await;
        assert_eq!(datagrams.len(), 2); // BYE ok response and forward BYE

        // Verify that the CDR contains the expected metrics and MOS
        let cdr = rx.try_recv().expect("CDR should exist");

        assert!(cdr.mos.is_some());
        let mos_val = cdr.mos.unwrap();
        assert!(mos_val > 3.5 && mos_val < 4.4); // typical reasonable quality under slight loss/jitter
        assert!(cdr.caller_rtcp_loss_rate.is_some());
        assert!(cdr.gateway_rtcp_loss_rate.is_some());
    }

    #[tokio::test]
    async fn test_sip_info_dtmf_extraction() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let edge_state = EdgeState::new(CallManager::new(RouteTable::new(vec![Route::new(
            "default",
            "",
            100,
            RouteTarget::new("gw1", "gw1.example.com", Some(5060)),
        )]), tx));
        let call_id = "info-dtmf-test@example.com";

        // 1. Setup call
        let invite = format!(
            "INVITE sip:13800138000@example.com SIP/2.0\r\n\
             Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-info-invite\r\n\
             Max-Forwards: 70\r\n\
             From: <sip:1001@example.com>;tag=from-tag\r\n\
             To: <sip:13800138000@example.com>\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 1 INVITE\r\n\
             Content-Length: 0\r\n\r\n",
            call_id = call_id
        );
        let _ = handle_datagram(invite.as_bytes(), peer(), &edge_state, &edge_config()).await;

        // 2. Answer call
        let response_200 = format!(
            "SIP/2.0 200 OK\r\n\
             Via: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-vosrs\r\n\
             From: <sip:1001@example.com>;tag=from-tag\r\n\
             To: <sip:13800138000@example.com>;tag=gw-tag\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 1 INVITE\r\n\
             Content-Length: 0\r\n\r\n",
            call_id = call_id
        );
        let _ = handle_datagram(
            response_200.as_bytes(),
            "198.51.100.20:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // 3. Send SIP INFO with application/dtmf-relay body (digit '7')
        let body_relay = "Signal= 7\r\nDuration= 160\r\n";
        let info_relay = format!(
            "INFO sip:13800138000@example.com SIP/2.0\r\n\
             Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-info-relay\r\n\
             Max-Forwards: 70\r\n\
             From: <sip:1001@example.com>;tag=from-tag\r\n\
             To: <sip:13800138000@example.com>;tag=gw-tag\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 2 INFO\r\n\
             Content-Type: application/dtmf-relay\r\n\
             Content-Length: {len}\r\n\r\n\
             {body}",
            call_id = call_id,
            len = body_relay.len(),
            body = body_relay
        );
        let _ = handle_datagram(info_relay.as_bytes(), peer(), &edge_state, &edge_config()).await;

        // 4. Send SIP INFO with application/dtmf body (digit '8')
        let body_dtmf = "8";
        let info_dtmf = format!(
            "INFO sip:13800138000@example.com SIP/2.0\r\n\
             Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-info-dtmf\r\n\
             Max-Forwards: 70\r\n\
             From: <sip:1001@example.com>;tag=from-tag\r\n\
             To: <sip:13800138000@example.com>;tag=gw-tag\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 3 INFO\r\n\
             Content-Type: application/dtmf\r\n\
             Content-Length: {len}\r\n\r\n\
             {body}",
            call_id = call_id,
            len = body_dtmf.len(),
            body = body_dtmf
        );
        let _ = handle_datagram(info_dtmf.as_bytes(), peer(), &edge_state, &edge_config()).await;

        // 5. Send BYE to terminate call
        let bye = format!(
            "BYE sip:13800138000@example.com SIP/2.0\r\n\
             Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-info-bye\r\n\
             From: <sip:1001@example.com>;tag=from-tag\r\n\
             To: <sip:13800138000@example.com>;tag=gw-tag\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 4 BYE\r\n\
             Content-Length: 0\r\n\r\n",
            call_id = call_id
        );
        let _ = handle_datagram(bye.as_bytes(), peer(), &edge_state, &edge_config()).await;

        // 6. Verify that the CDR contains the expected DTMF digits "78"
        let cdr = rx.try_recv().expect("CDR should exist");
        assert_eq!(cdr.dtmf_digits.as_deref(), Some("78"));
    }
