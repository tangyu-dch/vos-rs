    #[tokio::test]
    async fn inbound_refer_gets_accepted_and_notify_progress() {
        let edge_state = state_with_default_route();
        send_invite(&edge_state, "invite-refer@example.com").await;
        send_gateway_ok(&edge_state, "invite-refer@example.com").await;

        let refer = concat!(
            "REFER sip:13800138000@example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-refer\r\n",
            "Max-Forwards: 70\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:13800138000@example.com>;tag=gw-tag\r\n",
            "Call-ID: invite-refer@example.com\r\n",
            "CSeq: 2 REFER\r\n",
            "Refer-To: <sip:1002@example.com>\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        );

        let datagrams =
            handle_datagram(refer.as_bytes(), peer(), &edge_state, &edge_config()).await;

        assert_eq!(datagrams.len(), 3);
        assert_eq!(datagrams[0].target, peer().to_string());
        assert_eq!(datagrams[1].target, peer().to_string());
        assert_eq!(datagrams[2].target, "gw1.example.com:5060");

        let accepted = datagram_text(&datagrams[0]);
        assert!(accepted.starts_with("SIP/2.0 202 Accepted\r\n"));
        assert!(accepted.contains("CSeq: 2 REFER\r\n"));

        let notify = datagram_text(&datagrams[1]);
        assert!(notify.starts_with("NOTIFY sip:1001@example.com SIP/2.0\r\n"));
        assert!(notify.contains("From: <sip:13800138000@example.com>;tag=gw-tag\r\n"));
        assert!(notify.contains("To: <sip:1001@example.com>;tag=from-tag\r\n"));
        assert!(notify.contains("Call-ID: invite-refer@example.com\r\n"));
        assert!(notify.contains("CSeq: 52 NOTIFY\r\n"));
        assert!(notify.contains("Event: refer\r\n"));
        assert!(notify.contains("Subscription-State: active;expires=60\r\n"));
        assert!(notify.contains("Content-Type: message/sipfrag;version=2.0\r\n"));
        assert!(notify.ends_with("SIP/2.0 100 Trying\r\n"));

        let forwarded = datagram_text(&datagrams[2]);
        assert!(
            forwarded.starts_with("INVITE sip:1002@gw1.example.com:5060;transport=udp SIP/2.0\r\n")
        );
        assert!(forwarded.contains("CSeq: 1 INVITE\r\n"));
    }

    #[tokio::test]
    async fn gateway_refer_gets_accepted_notify_and_forwarded_to_caller() {
        let edge_state = state_with_default_route();
        send_invite(&edge_state, "invite-gateway-refer@example.com").await;
        send_gateway_ok(&edge_state, "invite-gateway-refer@example.com").await;

        let refer = concat!(
            "REFER sip:1001@example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 198.51.100.20:5060;branch=z9hG4bK-gw-refer\r\n",
            "Max-Forwards: 70\r\n",
            "From: <sip:13800138000@example.com>;tag=gw-tag\r\n",
            "To: <sip:1001@example.com>;tag=from-tag\r\n",
            "Call-ID: invite-gateway-refer@example.com\r\n",
            "CSeq: 2 REFER\r\n",
            "Refer-To: <sip:1003@example.com>\r\n",
            "Referred-By: <sip:13800138000@example.com>\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        );

        let datagrams = handle_datagram(
            refer.as_bytes(),
            "198.51.100.20:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        assert_eq!(datagrams.len(), 3);
        assert_eq!(datagrams[0].target, "198.51.100.20:5060");
        assert_eq!(datagrams[1].target, "198.51.100.20:5060");
        assert_eq!(datagrams[2].target, "gw1.example.com:5060");

        let accepted = datagram_text(&datagrams[0]);
        assert!(accepted.starts_with("SIP/2.0 202 Accepted\r\n"));
        assert!(accepted.contains("CSeq: 2 REFER\r\n"));

        let notify = datagram_text(&datagrams[1]);
        assert!(notify.starts_with("NOTIFY sip:13800138000@example.com SIP/2.0\r\n"));
        assert!(notify.contains("From: <sip:1001@example.com>;tag=from-tag\r\n"));
        assert!(notify.contains("To: <sip:13800138000@example.com>;tag=gw-tag\r\n"));
        assert!(notify.contains("Event: refer\r\n"));
        assert!(notify.ends_with("SIP/2.0 100 Trying\r\n"));

        let forwarded = datagram_text(&datagrams[2]);
        assert!(
            forwarded.starts_with("INVITE sip:1003@gw1.example.com:5060;transport=udp SIP/2.0\r\n")
        );
        assert!(forwarded.contains("CSeq: 1 INVITE\r\n"));
    }

    #[tokio::test]
    async fn test_refer_local_transfer_lifecycle() {
        let edge_state = state_with_default_route();
        let call_id = "refer-lifecycle-001";
        let offer_sdp = "v=0\r\no=- 0 0 IN IP4 192.0.2.10\r\ns=-\r\nc=IN IP4 192.0.2.10\r\nt=0 0\r\nm=audio 49170 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n";

        // 1. Inbound INVITE
        let invite = format!(
            "INVITE sip:13800138000@example.com SIP/2.0\r\nVia: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-inv-001\r\nMax-Forwards: 70\r\nFrom: <sip:1001@example.com>;tag=from-tag\r\nTo: <sip:13800138000@example.com>\r\nCall-ID: {call_id}\r\nCSeq: 1 INVITE\r\nContact: <sip:1001@192.0.2.10>\r\nContent-Type: application/sdp\r\nContent-Length: {len}\r\n\r\n{offer_sdp}",
            len = offer_sdp.len()
        );
        handle_datagram(
            invite.as_bytes(),
            "192.0.2.10:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // 2. Gateway responds 200 OK
        let answer_sdp = "v=0\r\no=- 0 0 IN IP4 198.51.100.20\r\ns=-\r\nc=IN IP4 198.51.100.20\r\nt=0 0\r\nm=audio 49200 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n";
        let ok_200 = format!(
            "SIP/2.0 200 OK\r\nVia: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-inv-001\r\nFrom: <sip:1001@example.com>;tag=from-tag\r\nTo: <sip:13800138000@example.com>;tag=gw-tag\r\nCall-ID: {call_id}\r\nCSeq: 1 INVITE\r\nContact: <sip:13800138000@gw1.example.com:5060>\r\nContent-Type: application/sdp\r\nContent-Length: {len}\r\n\r\n{answer_sdp}",
            len = answer_sdp.len()
        );
        handle_datagram(
            ok_200.as_bytes(),
            "203.0.113.20:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // 3. Caller sends REFER to transfer gateway B to target C (sip:1002@example.com)
        let refer = format!(
            "REFER sip:edge.example.com SIP/2.0\r\n\
             Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-ref-001\r\n\
             Max-Forwards: 70\r\n\
             From: <sip:1001@example.com>;tag=from-tag\r\n\
             To: <sip:13800138000@example.com>;tag=gw-tag\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 2 REFER\r\n\
             Refer-To: <sip:1002@example.com>\r\n\
             Content-Length: 0\r\n\r\n"
        );
        let datagrams = handle_datagram(
            refer.as_bytes(),
            "192.0.2.10:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // Should return 3 datagrams:
        // [0] 202 Accepted to referrer
        // [1] NOTIFY (100 Trying) to referrer
        // [2] INVITE to target C
        assert_eq!(datagrams.len(), 3, "expected 202 + NOTIFY + INVITE");

        let response_202 = datagram_text(&datagrams[0]);
        assert!(response_202.starts_with("SIP/2.0 202 Accepted\r\n"));

        let notify_trying = datagram_text(&datagrams[1]);
        assert!(notify_trying.starts_with("NOTIFY sip:1001@example.com SIP/2.0\r\n"));
        assert!(notify_trying.contains("SIP/2.0 100 Trying\r\n"));

        let invite_c = datagram_text(&datagrams[2]);
        assert!(
            invite_c.starts_with("INVITE sip:1002@gw1.example.com:5060;transport=udp SIP/2.0\r\n")
        );

        // Extract transfer call id
        let transfer_call_id = {
            invite_c
                .lines()
                .find(|l| l.starts_with("Call-ID:"))
                .unwrap()
                .split_whitespace()
                .nth(1)
                .unwrap()
                .to_string()
        };

        // 4. Target C responds 180 Ringing
        let ringing_180 = format!(
            "SIP/2.0 180 Ringing\r\n\
             Via: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-transfer-refer-lifecycle-001-52\r\n\
             From: <sip:13800138000@example.com>;tag=gw-tag\r\n\
             To: <sip:1002@example.com>;tag=c-tag\r\n\
             Call-ID: {transfer_call_id}\r\n\
             CSeq: 1 INVITE\r\n\
             Content-Length: 0\r\n\r\n"
        );
        let ringing_datagrams = handle_datagram(
            ringing_180.as_bytes(),
            "203.0.113.20:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;
        assert_eq!(ringing_datagrams.len(), 1);
        let notify_ringing = datagram_text(&ringing_datagrams[0]);
        assert!(notify_ringing.contains("SIP/2.0 180 Ringing\r\n"));

        // 5. Target C responds 200 OK with SDP
        let target_sdp = "v=0\r\no=- 0 0 IN IP4 198.51.100.30\r\ns=-\r\nc=IN IP4 198.51.100.30\r\nt=0 0\r\nm=audio 49300 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n";
        let answer_200 = format!(
            "SIP/2.0 200 OK\r\n\
             Via: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-transfer-refer-lifecycle-001-52\r\n\
             From: <sip:13800138000@example.com>;tag=gw-tag\r\n\
             To: <sip:1002@example.com>;tag=c-tag\r\n\
             Call-ID: {transfer_call_id}\r\n\
             CSeq: 1 INVITE\r\n\
             Contact: <sip:1002@198.51.100.30:5060>\r\n\
             Content-Type: application/sdp\r\n\
             Content-Length: {len}\r\n\r\n{target_sdp}",
            len = target_sdp.len()
        );
        let ok_datagrams = handle_datagram(
            answer_200.as_bytes(),
            "203.0.113.20:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // Should return 2 datagrams:
        // [0] NOTIFY (200 OK) to referrer
        // [1] BYE to referrer
        assert_eq!(ok_datagrams.len(), 2);
        let notify_ok = datagram_text(&ok_datagrams[0]);
        assert!(notify_ok.contains("SIP/2.0 200 OK\r\n"));
        assert!(notify_ok.contains("Subscription-State: terminated;reason=noresource\r\n"));

        let bye = datagram_text(&ok_datagrams[1]);
        assert!(bye.starts_with("BYE "));

        // Verify media bridging
        let tx = edge_state.inbound_transactions.get(call_id).unwrap();

        // Check if gateway's relay port target is updated to C's media IP/port (198.51.100.30:49300)
        let gw_relay = tx.gateway_relay_rtp.as_ref().unwrap();
        assert_eq!(
            edge_state.media_relay.target_for_port(gw_relay.port),
            Some("198.51.100.30:49300".parse().unwrap())
        );

        // Check if target C's relay port target is updated to the transferee's remote media endpoint (198.51.100.20:49200)
        let c_port = invite_c
            .lines()
            .find(|l| l.starts_with("m=audio"))
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap()
            .parse::<u16>()
            .unwrap();
        assert_eq!(
            edge_state.media_relay.target_for_port(c_port),
            Some("198.51.100.20:49200".parse().unwrap())
        );
    }

    #[tokio::test]
    async fn test_refer_transfer_failure_rollback() {
        let edge_state = state_with_default_route();
        let call_id = "refer-rollback-001";
        let offer_sdp = "v=0\r\no=- 0 0 IN IP4 192.0.2.10\r\ns=-\r\nc=IN IP4 192.0.2.10\r\nt=0 0\r\nm=audio 49170 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n";

        // 1. Inbound INVITE
        let invite = format!(
            "INVITE sip:13800138000@example.com SIP/2.0\r\nVia: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-inv-002\r\nMax-Forwards: 70\r\nFrom: <sip:1001@example.com>;tag=from-tag\r\nTo: <sip:13800138000@example.com>\r\nCall-ID: {call_id}\r\nCSeq: 1 INVITE\r\nContact: <sip:1001@192.0.2.10>\r\nContent-Type: application/sdp\r\nContent-Length: {len}\r\n\r\n{offer_sdp}",
            len = offer_sdp.len()
        );
        handle_datagram(
            invite.as_bytes(),
            "192.0.2.10:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // 2. Gateway responds 200 OK
        let answer_sdp = "v=0\r\no=- 0 0 IN IP4 198.51.100.20\r\ns=-\r\nc=IN IP4 198.51.100.20\r\nt=0 0\r\nm=audio 49200 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n";
        let ok_200 = format!(
            "SIP/2.0 200 OK\r\nVia: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-inv-002\r\nFrom: <sip:1001@example.com>;tag=from-tag\r\nTo: <sip:13800138000@example.com>;tag=gw-tag\r\nCall-ID: {call_id}\r\nCSeq: 1 INVITE\r\nContact: <sip:13800138000@gw1.example.com:5060>\r\nContent-Type: application/sdp\r\nContent-Length: {len}\r\n\r\n{answer_sdp}",
            len = answer_sdp.len()
        );
        handle_datagram(
            ok_200.as_bytes(),
            "203.0.113.20:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // Verify initial media pairing and target config
        {
            let tx = edge_state.inbound_transactions.get(call_id).unwrap();
            let caller_relay = tx.caller_relay_rtp.as_ref().unwrap();
            let gw_relay = tx.gateway_relay_rtp.as_ref().unwrap();
            assert_eq!(
                edge_state.media_relay.peer_port_for(caller_relay.port),
                Some(gw_relay.port)
            );
            assert_eq!(
                edge_state.media_relay.target_for_port(caller_relay.port),
                Some("198.51.100.20:49200".parse().unwrap())
            );
            assert_eq!(
                edge_state.media_relay.target_for_port(gw_relay.port),
                Some("192.0.2.10:49170".parse().unwrap())
            );
        }

        // 3. Caller sends REFER to transfer gateway B to target C (sip:1002@example.com)
        let refer = format!(
            "REFER sip:edge.example.com SIP/2.0\r\n\
             Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-ref-002\r\n\
             Max-Forwards: 70\r\n\
             From: <sip:1001@example.com>;tag=from-tag\r\n\
             To: <sip:13800138000@example.com>;tag=gw-tag\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 2 REFER\r\n\
             Refer-To: <sip:1002@example.com>\r\n\
             Content-Length: 0\r\n\r\n"
        );
        let datagrams = handle_datagram(
            refer.as_bytes(),
            "192.0.2.10:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;
        assert_eq!(datagrams.len(), 3);
        let invite_c = datagram_text(&datagrams[2]);
        let transfer_call_id = invite_c
            .lines()
            .find(|l| l.starts_with("Call-ID:"))
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap()
            .to_string();

        // 4. Target C responds with a failure (e.g. 486 Busy Here)
        let busy_486 = format!(
            "SIP/2.0 486 Busy Here\r\n\
             Via: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-transfer-refer-rollback-001-52\r\n\
             From: <sip:13800138000@example.com>;tag=gw-tag\r\n\
             To: <sip:1002@example.com>;tag=c-tag\r\n\
             Call-ID: {transfer_call_id}\r\n\
             CSeq: 1 INVITE\r\n\
             Content-Length: 0\r\n\r\n"
        );
        let err_datagrams = handle_datagram(
            busy_486.as_bytes(),
            "203.0.113.20:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // Should return 1 datagram: NOTIFY (486 Busy Here) to referrer with Subscription-State: terminated
        assert_eq!(err_datagrams.len(), 1);
        let notify_err = datagram_text(&err_datagrams[0]);
        assert!(notify_err.contains("SIP/2.0 486 Busy Here\r\n"));
        assert!(notify_err.contains("Subscription-State: terminated;reason=noresource\r\n"));

        // Verify media rollback occurred and original targets were restored
        {
            let tx = edge_state.inbound_transactions.get(call_id).unwrap();
            let caller_relay = tx.caller_relay_rtp.as_ref().unwrap();
            let gw_relay = tx.gateway_relay_rtp.as_ref().unwrap();
            assert_eq!(
                edge_state.media_relay.peer_port_for(caller_relay.port),
                Some(gw_relay.port)
            );
            assert_eq!(
                edge_state.media_relay.target_for_port(caller_relay.port),
                Some("198.51.100.20:49200".parse().unwrap())
            );
            assert_eq!(
                edge_state.media_relay.target_for_port(gw_relay.port),
                Some("192.0.2.10:49170".parse().unwrap())
            );
        }
    }
