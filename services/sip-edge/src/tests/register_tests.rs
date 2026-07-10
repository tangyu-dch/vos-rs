    #[tokio::test]
    async fn register_stores_contact_and_returns_binding() {
        let edge_state = state_without_routes();
        let request = concat!(
            "REGISTER sip:example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-reg\r\n",
            "Max-Forwards: 70\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:1001@example.com>\r\n",
            "Call-ID: reg-1@example.com\r\n",
            "CSeq: 1 REGISTER\r\n",
            "Contact: <sip:1001@192.0.2.10:5070;transport=udp>;expires=120\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        );

        let datagrams =
            handle_datagram(request.as_bytes(), peer(), &edge_state, &edge_config()).await;
        assert_eq!(datagrams.len(), 1);
        assert_eq!(datagrams[0].target, "192.0.2.10:5060");

        let response = datagram_text(&datagrams[0]);
        assert!(response.starts_with("SIP/2.0 200 OK\r\n"));
        assert!(response.contains("X-VOS-RS-AOR: sip:1001@example.com\r\n"));
        assert!(
            response.contains("Contact: <sip:1001@192.0.2.10:5070;transport=udp>;expires=120\r\n")
        );
        assert_eq!(edge_state.registrar.read().await.binding_count(), 1);
    }

    #[tokio::test]
    async fn register_query_returns_existing_contact() {
        let edge_state = state_without_routes();
        let register = concat!(
            "REGISTER sip:example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-reg\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:1001@example.com>\r\n",
            "Call-ID: reg-query@example.com\r\n",
            "CSeq: 1 REGISTER\r\n",
            "Contact: <sip:1001@192.0.2.10:5070>;expires=120\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        );
        let _ = handle_datagram(register.as_bytes(), peer(), &edge_state, &edge_config()).await;

        let query = concat!(
            "REGISTER sip:example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-query\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:1001@example.com>\r\n",
            "Call-ID: reg-query@example.com\r\n",
            "CSeq: 2 REGISTER\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        );

        let datagrams =
            handle_datagram(query.as_bytes(), peer(), &edge_state, &edge_config()).await;

        assert_eq!(datagrams.len(), 1);
        let response = datagram_text(&datagrams[0]);
        assert!(response.starts_with("SIP/2.0 200 OK\r\n"));
        assert!(response.contains("Contact: <sip:1001@192.0.2.10:5070>;expires="));
    }

    #[tokio::test]
    async fn unregister_removes_contact() {
        let edge_state = state_without_routes();
        let register = concat!(
            "REGISTER sip:example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-reg\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:1001@example.com>\r\n",
            "Call-ID: reg-unregister@example.com\r\n",
            "CSeq: 1 REGISTER\r\n",
            "Contact: <sip:1001@192.0.2.10:5070>;expires=120\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        );
        let _ = handle_datagram(register.as_bytes(), peer(), &edge_state, &edge_config()).await;

        let unregister = concat!(
            "REGISTER sip:example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-unreg\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:1001@example.com>\r\n",
            "Call-ID: reg-unregister@example.com\r\n",
            "CSeq: 2 REGISTER\r\n",
            "Contact: <sip:1001@192.0.2.10:5070>;expires=0\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        );

        let datagrams =
            handle_datagram(unregister.as_bytes(), peer(), &edge_state, &edge_config()).await;

        assert_eq!(datagrams.len(), 1);
        let response = datagram_text(&datagrams[0]);
        assert!(response.starts_with("SIP/2.0 200 OK\r\n"));
        assert!(!response.contains("Contact: <sip:1001@192.0.2.10:5070>"));
        assert_eq!(edge_state.registrar.read().await.binding_count(), 0);
    }
