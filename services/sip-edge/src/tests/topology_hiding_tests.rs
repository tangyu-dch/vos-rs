    // ── Topology Hiding (Scheme 6) ────────────────────────────────────────────

    /// Verifies that outbound INVITEs carry a new (external) Call-ID distinct
    /// from the inbound (internal) one — the core of topology hiding.
    #[tokio::test]
    async fn test_topology_hiding_call_id_rewritten_on_outbound_invite() {
        let routes = RouteTable::new(vec![Route::new(
            "gw1",
            "".to_string(),
            100,
            RouteTarget::new("gw1".to_string(), "203.0.113.20".to_string(), Some(5060)),
        )]);
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let edge_state = Arc::new(EdgeState::new(CallManager::new(routes, tx)));
        let internal_call_id = "internal-call-id-topo-test@example.com";
        let invite = format!(
            concat!(
                "INVITE sip:13800138000@example.com SIP/2.0\r\n",
                "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-topo\r\n",
                "Max-Forwards: 70\r\n",
                "From: <sip:1001@example.com>;tag=from-tag\r\n",
                "To: <sip:13800138000@example.com>\r\n",
                "Call-ID: {call_id}\r\n",
                "CSeq: 1 INVITE\r\n",
                "Content-Length: 0\r\n",
                "\r\n"
            ),
            call_id = internal_call_id
        );

        let datagrams =
            handle_datagram(invite.as_bytes(), peer(), &edge_state, &edge_config()).await;
        assert_eq!(datagrams.len(), 2);
        let outbound_text = datagram_text(&datagrams[1]);

        assert!(
            !outbound_text.contains(internal_call_id),
            "outbound INVITE should not expose the internal Call-ID, got:\n{}",
            outbound_text
        );
        let call_id_count = outbound_text
            .lines()
            .filter(|l| l.to_ascii_lowercase().starts_with("call-id:"))
            .count();
        assert_eq!(
            call_id_count, 1,
            "outbound INVITE must have exactly one Call-ID header"
        );

        let external_call_id = outbound_text
            .lines()
            .find(|l| l.to_ascii_lowercase().starts_with("call-id:"))
            .and_then(|l| l.split_once(':').map(|(_, v)| v.trim().to_string()))
            .expect("Call-ID header not found in outbound INVITE");

        assert_ne!(external_call_id, internal_call_id);
        assert_eq!(
            edge_state
                .get_internal_call_id(&external_call_id)
                .as_deref(),
            Some(internal_call_id)
        );
        assert_eq!(
            edge_state.get_external_call_id(internal_call_id).as_deref(),
            Some(external_call_id.as_str())
        );
    }

    /// Verifies that a 200 OK from the gateway (with external Call-ID) is
    /// forwarded to the caller using the original internal Call-ID.
    #[tokio::test]
    async fn test_topology_hiding_gateway_200_forwarded_with_internal_call_id() {
        let routes = RouteTable::new(vec![Route::new(
            "gw1",
            "".to_string(),
            100,
            RouteTarget::new("gw1".to_string(), "203.0.113.20".to_string(), Some(5060)),
        )]);
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let edge_state = Arc::new(EdgeState::new(CallManager::new(routes, tx)));
        let internal_call_id = "topo-gw-200-test@example.com";

        let invite = format!(
            concat!(
                "INVITE sip:13800138000@example.com SIP/2.0\r\n",
                "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-topo-gw\r\n",
                "Max-Forwards: 70\r\n",
                "From: <sip:1001@example.com>;tag=caller-tag\r\n",
                "To: <sip:13800138000@example.com>\r\n",
                "Call-ID: {cid}\r\n",
                "CSeq: 1 INVITE\r\n",
                "Content-Length: 0\r\n",
                "\r\n"
            ),
            cid = internal_call_id
        );
        let dg = handle_datagram(invite.as_bytes(), peer(), &edge_state, &edge_config()).await;
        assert_eq!(dg.len(), 2);

        let out_text = datagram_text(&dg[1]);
        let external_call_id = out_text
            .lines()
            .find(|l| l.to_ascii_lowercase().starts_with("call-id:"))
            .and_then(|l| l.split_once(':').map(|(_, v)| v.trim().to_string()))
            .expect("outbound INVITE has no Call-ID");

        let gw_200 = format!(
            concat!(
                "SIP/2.0 200 OK\r\n",
                "Via: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-vosrs\r\n",
                "From: <sip:1001@example.com>;tag=caller-tag\r\n",
                "To: <sip:13800138000@example.com>;tag=gw-tag\r\n",
                "Call-ID: {cid}\r\n",
                "CSeq: 1 INVITE\r\n",
                "Content-Length: 0\r\n",
                "\r\n"
            ),
            cid = external_call_id
        );
        let gw_peer: SocketAddr = "203.0.113.20:5060".parse().unwrap();
        let resp_dg =
            handle_datagram(gw_200.as_bytes(), gw_peer, &edge_state, &edge_config()).await;

        assert_eq!(resp_dg.len(), 1, "200 OK should be forwarded to the caller");
        let forwarded = datagram_text(&resp_dg[0]);

        assert!(
            forwarded.contains(internal_call_id),
            "forwarded 200 OK should contain the internal Call-ID, got:\n{}",
            forwarded
        );
        assert!(
            !forwarded.contains(external_call_id.as_str()),
            "forwarded 200 OK must not expose the external Call-ID, got:\n{}",
            forwarded
        );
    }
