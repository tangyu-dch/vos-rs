    #[tokio::test]
    async fn test_prack_header_validation() {
        let edge_state = state_with_default_route();

        // 1. Establish transaction
        let call_id = "prack-val-001";
        let body = sdp_body();
        let invite = format!(
            concat!(
                "INVITE sip:13800138000@example.com SIP/2.0\r\n",
                "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-prval\r\n",
                "Max-Forwards: 70\r\n",
                "From: <sip:1001@example.com>;tag=from-tag\r\n",
                "To: <sip:13800138000@example.com>\r\n",
                "Call-ID: {call_id}\r\n",
                "CSeq: 1 INVITE\r\n",
                "Contact: <sip:1001@192.0.2.10>\r\n",
                "Content-Type: application/sdp\r\n",
                "Content-Length: {}\r\n",
                "\r\n",
                "{}"
            ),
            body.len(),
            body,
            call_id = call_id
        );

        let _ = handle_datagram(invite.as_bytes(), peer(), &edge_state, &edge_config()).await;

        // Gateway responds 180 Ringing with 100rel
        let ringing = format!(
            "SIP/2.0 180 Ringing\r\n\
             Via: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-vosrs\r\n\
             Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-prval\r\n\
             From: <sip:1001@example.com>;tag=from-tag\r\n\
             To: <sip:13800138000@example.com>;tag=gw-tag\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 1 INVITE\r\n\
             Require: 100rel\r\n\
             RSeq: 42\r\n\
             Content-Length: 0\r\n\r\n",
            call_id = call_id
        );
        let _ = handle_datagram(
            ringing.as_bytes(),
            "198.51.100.20:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // 2. Send PRACK with missing RAck header
        let prack_missing = format!(
            "PRACK sip:edge.example.com SIP/2.0\r\n\
             Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-prack-miss\r\n\
             From: <sip:1001@example.com>;tag=from-tag\r\n\
             To: <sip:13800138000@example.com>;tag=gw-tag\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 2 PRACK\r\n\
             Content-Length: 0\r\n\r\n",
            call_id = call_id
        );
        let datagrams = handle_datagram(
            prack_missing.as_bytes(),
            "192.0.2.10:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        assert_eq!(datagrams.len(), 1);
        let resp = datagram_text(&datagrams[0]);
        assert!(resp.starts_with("SIP/2.0 400 Bad Request - Invalid RAck\r\n"));

        // 3. Send PRACK with valid RAck header
        let prack_valid = format!(
            "PRACK sip:edge.example.com SIP/2.0\r\n\
             Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-prack-val\r\n\
             From: <sip:1001@example.com>;tag=from-tag\r\n\
             To: <sip:13800138000@example.com>;tag=gw-tag\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 3 PRACK\r\n\
             RAck: 42 1 INVITE\r\n\
             Content-Length: 0\r\n\r\n",
            call_id = call_id
        );
        let datagrams_ok = handle_datagram(
            prack_valid.as_bytes(),
            "192.0.2.10:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        assert_eq!(datagrams_ok.len(), 1);
        let resp_ok = datagram_text(&datagrams_ok[0]);
        assert!(resp_ok.starts_with("SIP/2.0 200 OK\r\n"));
    }

    // ── outbound::tests additions ────────────────────────────────────────────

    /// Verify the outbound INVITE contains both 'timer' and '100rel' in Supported.
    #[tokio::test]
    async fn test_invite_supported_includes_100rel() {
        let edge_state = state_with_gateway_uri("sip:198.51.100.20:5060");
        let sdp_body = "v=0\r\no=- 0 0 IN IP4 192.0.2.10\r\ns=-\r\nc=IN IP4 192.0.2.10\r\nt=0 0\r\nm=audio 49170 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n";
        let call_id = "prack-supported-test-001";
        let invite = format!(
            "INVITE sip:13800138000@example.com SIP/2.0\r\nVia: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-pr-sup\r\nMax-Forwards: 70\r\nFrom: <sip:1001@example.com>;tag=from-tag\r\nTo: <sip:13800138000@example.com>\r\nCall-ID: {call_id}\r\nCSeq: 1 INVITE\r\nContact: <sip:1001@192.0.2.10>\r\nContent-Type: application/sdp\r\nContent-Length: {len}\r\n\r\n{sdp_body}",
            len = sdp_body.len()
        );
        let datagrams = handle_datagram(
            invite.as_bytes(),
            "192.0.2.10:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;
        assert_eq!(datagrams.len(), 2);
        let outbound = datagram_text(&datagrams[1]);
        assert!(
            outbound.contains("Supported: timer,100rel"),
            "outbound INVITE must advertise both timer and 100rel\n{outbound}"
        );
    }

    /// Verify that a 180 with Require: 100rel causes sip-edge to:
    ///   - emit a PRACK toward the gateway
    ///   - forward the 180 (with rewritten RSeq) toward the caller
    #[tokio::test]
    async fn test_180_with_100rel_triggers_prack_to_gateway() {
        let edge_state = state_with_gateway_uri("sip:198.51.100.20:5060");
        let call_id = "prack-180-test-001";
        let sdp_body = "v=0\r\no=- 0 0 IN IP4 192.0.2.10\r\ns=-\r\nc=IN IP4 192.0.2.10\r\nt=0 0\r\nm=audio 49170 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n";

        // 1. Establish inbound INVITE to create transaction
        let invite = format!(
            "INVITE sip:13800138000@example.com SIP/2.0\r\nVia: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-pr-180\r\nMax-Forwards: 70\r\nFrom: <sip:1001@example.com>;tag=from-tag\r\nTo: <sip:13800138000@example.com>\r\nCall-ID: {call_id}\r\nCSeq: 1 INVITE\r\nContact: <sip:1001@192.0.2.10>\r\nContent-Type: application/sdp\r\nContent-Length: {len}\r\n\r\n{sdp_body}",
            len = sdp_body.len()
        );
        handle_datagram(
            invite.as_bytes(),
            "192.0.2.10:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // 2. Gateway sends 180 Ringing with Require: 100rel and RSeq: 42
        let ringing_180 = format!(
            "SIP/2.0 180 Ringing\r\nVia: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-pr-180\r\nFrom: <sip:1001@example.com>;tag=from-tag\r\nTo: <sip:13800138000@example.com>;tag=gw-tag\r\nCall-ID: {call_id}\r\nCSeq: 1 INVITE\r\nRequire: 100rel\r\nRSeq: 42\r\nContent-Length: 0\r\n\r\n"
        );
        let datagrams = handle_datagram(
            ringing_180.as_bytes(),
            "198.51.100.20:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // Should produce exactly 2 datagrams:
        //   [0] PRACK → gateway
        //   [1] 180 Ringing (rewritten RSeq: 1) → caller
        assert_eq!(datagrams.len(), 2, "expected PRACK + forwarded 180");

        let prack = datagram_text(&datagrams[0]);
        assert!(
            prack.starts_with("PRACK "),
            "first datagram must be PRACK\n{prack}"
        );
        assert!(
            prack.contains("RAck: 42 1 INVITE"),
            "PRACK must echo gateway RSeq in RAck\n{prack}"
        );

        let forwarded_180 = datagram_text(&datagrams[1]);
        assert!(
            forwarded_180.contains("180 Ringing"),
            "second datagram must be the 180\n{forwarded_180}"
        );
        assert!(
            forwarded_180.contains("Require: 100rel"),
            "forwarded 180 must keep Require: 100rel\n{forwarded_180}"
        );
        assert!(
            forwarded_180.contains("RSeq: 1"),
            "RSeq must be rewritten to 1 by sip-edge\n{forwarded_180}"
        );

        // Verify transaction state was updated
        let tx = edge_state.inbound_transactions.get(call_id).unwrap();
        assert!(
            tx.gateway_100rel,
            "gateway_100rel must be set after receiving Require: 100rel"
        );
        assert_eq!(tx.prack_rseq, 1, "prack_rseq counter must be 1");
    }

    /// Verify that a PRACK from the caller receives 200 OK and is not forwarded.
    #[tokio::test]
    async fn test_prack_from_caller_receives_200_ok_only() {
        let edge_state = state_with_gateway_uri("sip:198.51.100.20:5060");
        let call_id = "prack-ack-test-001";
        let sdp_body = "v=0\r\no=- 0 0 IN IP4 192.0.2.10\r\ns=-\r\nc=IN IP4 192.0.2.10\r\nt=0 0\r\nm=audio 49170 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n";

        // Establish transaction and trigger 100rel so prack_rseq > 0
        let invite = format!(
            "INVITE sip:13800138000@example.com SIP/2.0\r\nVia: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-prack-ack\r\nMax-Forwards: 70\r\nFrom: <sip:1001@example.com>;tag=from-tag\r\nTo: <sip:13800138000@example.com>\r\nCall-ID: {call_id}\r\nCSeq: 1 INVITE\r\nContact: <sip:1001@192.0.2.10>\r\nContent-Type: application/sdp\r\nContent-Length: {len}\r\n\r\n{sdp_body}",
            len = sdp_body.len()
        );
        handle_datagram(
            invite.as_bytes(),
            "192.0.2.10:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;
        handle_datagram(
            format!("SIP/2.0 180 Ringing\r\nVia: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-prack-ack\r\nFrom: <sip:1001@example.com>;tag=from-tag\r\nTo: <sip:13800138000@example.com>;tag=gw-tag\r\nCall-ID: {call_id}\r\nCSeq: 1 INVITE\r\nRequire: 100rel\r\nRSeq: 1\r\nContent-Length: 0\r\n\r\n").as_bytes(),
            "198.51.100.20:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        ).await;

        // Caller sends PRACK
        let caller_prack = format!(
            "PRACK sip:edge.example.com SIP/2.0\r\nVia: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-caller-prack\r\nMax-Forwards: 70\r\nFrom: <sip:1001@example.com>;tag=from-tag\r\nTo: <sip:13800138000@example.com>;tag=gw-tag\r\nCall-ID: {call_id}\r\nCSeq: 2 PRACK\r\nRAck: 1 1 INVITE\r\nContent-Length: 0\r\n\r\n"
        );
        let datagrams = handle_datagram(
            caller_prack.as_bytes(),
            "192.0.2.10:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // Must produce exactly 1 datagram: 200 OK to caller (no forwarding to gateway)
        assert_eq!(
            datagrams.len(),
            1,
            "PRACK must only produce 200 OK — not forwarded\n{:?}",
            datagrams.iter().map(datagram_text).collect::<Vec<_>>()
        );
        let resp = datagram_text(&datagrams[0]);
        assert!(resp.starts_with("SIP/2.0 200 OK"), "must be 200 OK\n{resp}");
    }

    // ── Early Media (183 Session Progress + SDP) ────────────────────────────

    /// Verify that a 183 with SDP:
    ///   - allocates relay ports for early media
    ///   - rewrites the SDP endpoint to the relay IP
    ///   - forwards the 183 to the caller
    /// And that the subsequent 200 OK still finalises media correctly.
    #[tokio::test]
    async fn test_early_media_183_sdp_allocates_relay_and_forwards() {
        let edge_state = state_with_default_route();
        let call_id = "early-media-test-001";
        let offer_sdp = "v=0\r\no=- 0 0 IN IP4 192.0.2.10\r\ns=-\r\nc=IN IP4 192.0.2.10\r\nt=0 0\r\nm=audio 49170 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n";

        // Step 1: caller sends INVITE
        let invite = format!(
            "INVITE sip:13800138000@example.com SIP/2.0\r\nVia: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-em-001\r\nMax-Forwards: 70\r\nFrom: <sip:1001@example.com>;tag=from-tag\r\nTo: <sip:13800138000@example.com>\r\nCall-ID: {call_id}\r\nCSeq: 1 INVITE\r\nContact: <sip:1001@192.0.2.10>\r\nContent-Type: application/sdp\r\nContent-Length: {len}\r\n\r\n{offer_sdp}",
            len = offer_sdp.len()
        );
        let invite_datagrams = handle_datagram(
            invite.as_bytes(),
            "192.0.2.10:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;
        assert_eq!(
            invite_datagrams.len(),
            2,
            "INVITE must produce 100 Trying + outbound INVITE"
        );

        // Extract the relay port allocated for the gateway-facing side
        let outbound_invite_body = datagram_text(&invite_datagrams[1]);
        let gateway_relay_port: u16 = {
            outbound_invite_body
                .lines()
                .find(|l| l.starts_with("m=audio"))
                .and_then(|l| l.split_whitespace().nth(1)?.parse().ok())
                .expect("outbound INVITE must have m=audio with relay port")
        };

        // Step 2: gateway sends 183 Session Progress with SDP (early media)
        let early_sdp = "v=0\r\no=- 0 0 IN IP4 198.51.100.20\r\ns=-\r\nc=IN IP4 198.51.100.20\r\nt=0 0\r\nm=audio 49200 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n";
        let session_progress_183 = format!(
            "SIP/2.0 183 Session Progress\r\nVia: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-em-001\r\nFrom: <sip:1001@example.com>;tag=from-tag\r\nTo: <sip:13800138000@example.com>;tag=gw-tag\r\nCall-ID: {call_id}\r\nCSeq: 1 INVITE\r\nContact: <sip:13800138000@gw1.example.com:5060>\r\nContent-Type: application/sdp\r\nContent-Length: {len}\r\n\r\n{early_sdp}",
            len = early_sdp.len()
        );
        let datagrams_183 = handle_datagram(
            session_progress_183.as_bytes(),
            "203.0.113.20:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // 183 with SDP must be forwarded to caller with rewritten media endpoint
        assert_eq!(datagrams_183.len(), 1, "183 must be forwarded to caller");
        let forwarded_183 = datagram_text(&datagrams_183[0]);
        assert!(
            forwarded_183.contains("183 Session Progress"),
            "forwarded message must be 183\n{forwarded_183}"
        );
        assert!(
            forwarded_183.contains("203.0.113.10"),
            "SDP must be rewritten to relay IP\n{forwarded_183}"
        );

        // The relay port facing the caller (caller_relay_rtp) must now be set
        let caller_relay_port: u16 = {
            forwarded_183
                .lines()
                .find(|l| l.starts_with("m=audio"))
                .and_then(|l| l.split_whitespace().nth(1)?.parse().ok())
                .expect("forwarded 183 must have m=audio with caller relay port")
        };
        assert_ne!(
            caller_relay_port, 49200,
            "SDP port must be rewritten away from gateway port"
        );
        assert_ne!(
            caller_relay_port, gateway_relay_port,
            "caller relay port must differ from gateway relay port"
        );

        // The caller-facing relay port target must point to the gateway's early media endpoint
        // (caller-relay → gateway direction)
        assert_eq!(
            edge_state.media_relay.target_for_port(caller_relay_port),
            Some("198.51.100.20:49200".parse().unwrap()),
            "caller relay must target the gateway early media port"
        );

        // The gateway relay (allocated during INVITE) target must point to the caller
        // (gateway-relay → caller direction, set during INVITE SDP rewrite)
        assert_eq!(
            edge_state.media_relay.target_for_port(gateway_relay_port),
            Some("192.0.2.10:49170".parse().unwrap()),
            "gateway relay must still target the caller's RTP port"
        );

        // Step 3: verify final 200 OK is still handled correctly after early media
        let final_sdp = "v=0\r\no=- 0 0 IN IP4 198.51.100.20\r\ns=-\r\nc=IN IP4 198.51.100.20\r\nt=0 0\r\nm=audio 49202 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n";
        let ok_200 = format!(
            "SIP/2.0 200 OK\r\nVia: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-em-001\r\nFrom: <sip:1001@example.com>;tag=from-tag\r\nTo: <sip:13800138000@example.com>;tag=gw-tag\r\nCall-ID: {call_id}\r\nCSeq: 1 INVITE\r\nContact: <sip:13800138000@gw1.example.com:5060>\r\nContent-Type: application/sdp\r\nContent-Length: {len}\r\n\r\n{final_sdp}",
            len = final_sdp.len()
        );
        let datagrams_200 = handle_datagram(
            ok_200.as_bytes(),
            "203.0.113.20:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        assert_eq!(datagrams_200.len(), 1, "200 OK must be forwarded to caller");
        let forwarded_200 = datagram_text(&datagrams_200[0]);
        assert!(
            forwarded_200.starts_with("SIP/2.0 200 OK"),
            "must be 200 OK\n{forwarded_200}"
        );
        assert!(
            forwarded_200.contains("203.0.113.10"),
            "200 OK SDP must also use relay IP\n{forwarded_200}"
        );

        // Gateway relay target must be updated to final SDP port — check the caller-relay port
        // (The caller-relay → gateway direction is what gets updated when the final SDP arrives)
        assert_eq!(
            edge_state.media_relay.target_for_port(caller_relay_port),
            Some("198.51.100.20:49202".parse().unwrap()),
            "caller relay target must be updated to final media port from 200 OK"
        );
    }

    /// Verify that a 183 WITHOUT SDP is forwarded as-is (no relay allocation needed).
    #[tokio::test]
    async fn test_183_without_sdp_forwarded_unchanged() {
        let edge_state = state_with_default_route();
        let call_id = "early-media-no-sdp-001";
        let offer_sdp = "v=0\r\no=- 0 0 IN IP4 192.0.2.10\r\ns=-\r\nc=IN IP4 192.0.2.10\r\nt=0 0\r\nm=audio 49170 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n";

        let invite = format!(
            "INVITE sip:13800138000@example.com SIP/2.0\r\nVia: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-em-nossdp\r\nMax-Forwards: 70\r\nFrom: <sip:1001@example.com>;tag=from-tag\r\nTo: <sip:13800138000@example.com>\r\nCall-ID: {call_id}\r\nCSeq: 1 INVITE\r\nContact: <sip:1001@192.0.2.10>\r\nContent-Type: application/sdp\r\nContent-Length: {len}\r\n\r\n{offer_sdp}",
            len = offer_sdp.len()
        );
        handle_datagram(
            invite.as_bytes(),
            "192.0.2.10:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // 183 without SDP body
        let session_progress_183 = format!(
            "SIP/2.0 183 Session Progress\r\nVia: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-em-nossdp\r\nFrom: <sip:1001@example.com>;tag=from-tag\r\nTo: <sip:13800138000@example.com>;tag=gw-tag\r\nCall-ID: {call_id}\r\nCSeq: 1 INVITE\r\nContent-Length: 0\r\n\r\n"
        );
        let datagrams = handle_datagram(
            session_progress_183.as_bytes(),
            "203.0.113.20:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // 183 without SDP = forward straight to caller
        assert_eq!(datagrams.len(), 1, "183 without SDP must be forwarded");
        let forwarded = datagram_text(&datagrams[0]);
        assert!(
            forwarded.contains("183 Session Progress"),
            "must be 183\n{forwarded}"
        );

        // No caller_relay_rtp should be set (no SDP to allocate relay for)
        let tx = edge_state.inbound_transactions.get(call_id).expect("transaction must exist");
        assert!(
            tx.caller_relay_rtp.is_none(),
            "no relay should be set without SDP in 183"
        );
    }
