    #[tokio::test]
    async fn invalid_register_receives_bad_request() {
        let edge_state = state_without_routes();
        let request = concat!(
            "REGISTER sip:example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-reg-bad\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:1001@example.com>\r\n",
            "Call-ID: reg-bad@example.com\r\n",
            "CSeq: 1 REGISTER\r\n",
            "Contact: *\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        );

        let datagrams =
            handle_datagram(request.as_bytes(), peer(), &edge_state, &edge_config()).await;

        assert_eq!(datagrams.len(), 1);
        let response = datagram_text(&datagrams[0]);
        assert!(response.starts_with("SIP/2.0 400 Bad Request\r\n"));
        assert!(response.contains("X-VOS-RS-Error: invalid REGISTER Contact: *\r\n"));
    }

    #[tokio::test]
    async fn register_requires_digest_auth_when_configured() {
        let edge_state = state_without_routes();
        let request = concat!(
            "REGISTER sip:example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-auth\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:1001@example.com>\r\n",
            "Call-ID: reg-auth@example.com\r\n",
            "CSeq: 1 REGISTER\r\n",
            "Contact: <sip:1001@192.0.2.10:5070>;expires=120\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        );

        let datagrams = handle_datagram(
            request.as_bytes(),
            peer(),
            &edge_state,
            &edge_config_with_auth(),
        )
        .await;

        assert_eq!(datagrams.len(), 1);
        let response = datagram_text(&datagrams[0]);
        assert!(response.starts_with("SIP/2.0 401 Unauthorized\r\n"));
        assert!(response.contains(
            "WWW-Authenticate: Digest realm=\"vos-rs\", nonce=\"test-nonce\", algorithm=MD5, qop=\"auth\"\r\n"
        ));
        assert_eq!(edge_state.registrar.read().await.binding_count(), 0);
    }

    #[tokio::test]
    async fn register_accepts_valid_digest_auth_when_configured() {
        let edge_state = state_without_routes();
        let uri = "sip:example.com";
        let digest = digest_response(
            "1001",
            "secret",
            "vos-rs",
            "test-nonce",
            "REGISTER",
            uri,
            Some(("auth", "00000001", "abcdef")),
        );
        let request = format!(
            concat!(
                "REGISTER {uri} SIP/2.0\r\n",
                "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-auth-ok\r\n",
                "From: <sip:1001@example.com>;tag=from-tag\r\n",
                "To: <sip:1001@example.com>\r\n",
                "Call-ID: reg-auth-ok@example.com\r\n",
                "CSeq: 1 REGISTER\r\n",
                "Authorization: Digest username=\"1001\", realm=\"vos-rs\", nonce=\"test-nonce\", uri=\"{uri}\", response=\"{digest}\", algorithm=MD5, qop=auth, nc=00000001, cnonce=\"abcdef\"\r\n",
                "Contact: <sip:1001@192.0.2.10:5070>;expires=120\r\n",
                "Content-Length: 0\r\n",
                "\r\n"
            ),
            uri = uri,
            digest = digest
        );

        let datagrams = handle_datagram(
            request.as_bytes(),
            peer(),
            &edge_state,
            &edge_config_with_auth(),
        )
        .await;

        assert_eq!(datagrams.len(), 1);
        let response = datagram_text(&datagrams[0]);
        assert!(response.starts_with("SIP/2.0 200 OK\r\n"));
        assert!(response.contains("Contact: <sip:1001@192.0.2.10:5070>;expires=120\r\n"));
        assert_eq!(edge_state.registrar.read().await.binding_count(), 1);
    }

    #[tokio::test]
    async fn test_dynamic_nonce_verification() {
        let auth = AuthConfig::new(
            "vos-rs",
            "test-nonce",
            HashMap::from([("1001".to_string(), "secret".to_string())]),
        );

        let nonce = auth.generate_dynamic_nonce();
        assert!(auth.verify_dynamic_nonce(&nonce, 300));

        // Tamper: replace the signature portion with wrong value
        // Format is {ts}-{seq}-{sig}, replace last segment
        let parts: Vec<&str> = nonce.split('-').collect();
        let tampered = format!("{}-{}-wrongsig", parts[0], parts[1]);
        assert!(!auth.verify_dynamic_nonce(&tampered, 300));

        // Use a deterministic older timestamp to test age expiration
        let past_ts = std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            - 10;
        let past_sig = format!(
            "{:x}",
            md5::compute(format!("{}:{}:{}", past_ts, 0, auth.secret_key).as_bytes())
        );
        let past_nonce = format!("{}-0-{}", past_ts, past_sig);

        assert!(auth.verify_dynamic_nonce(&past_nonce, 15));
        assert!(!auth.verify_dynamic_nonce(&past_nonce, 5));
    }

    #[tokio::test]
    async fn test_nonce_anti_replay_protection() {
        let auth = AuthConfig::new(
            "vos-rs",
            "test-nonce",
            HashMap::from([("1001".to_string(), "secret".to_string())]),
        );

        let cache = dashmap::DashMap::new();

        let raw_invite = concat!(
            "INVITE sip:13800138000@edge.example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-replay\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:13800138000@example.com>\r\n",
            "Call-ID: replay-test@example.com\r\n",
            "CSeq: 1 INVITE\r\n",
            "Content-Length: 0\r\n\r\n"
        );

        let SipMessage::Request(request) = parse_message(raw_invite.as_bytes()).unwrap() else {
            panic!("expected request");
        };

        let response = crate::auth::digest_response(
            "1001",
            "secret",
            "vos-rs",
            "test-nonce",
            "INVITE",
            "sip:13800138000@edge.example.com",
            Some(("auth", "00000001", "abcdef")),
        );

        let auth_hdr = format!(
            "Digest username=\"1001\", realm=\"vos-rs\", nonce=\"test-nonce\", uri=\"sip:13800138000@edge.example.com\", response=\"{response}\", algorithm=MD5, qop=auth, nc=00000001, cnonce=\"abcdef\""
        );

        let mut request_correct = request.clone();
        request_correct.headers.insert(
            sip_core::HeaderName::new("proxy-authorization").unwrap(),
            sip_core::HeaderValue::new(&auth_hdr),
        );

        // First attempt succeeds
        assert_eq!(
            auth.verify_request(&request_correct, None, Some(&cache))
                .await,
            crate::AuthDecision::Authorized {
                username: "1001".to_string()
            }
        );

        // Replay attempt fails (returns Challenge due to replay block)
        assert_eq!(
            auth.verify_request(&request_correct, None, Some(&cache))
                .await,
            crate::AuthDecision::Challenge
        );
    }

    #[tokio::test]
    async fn test_invite_proxy_auth_flow() {
        let edge_state = state_with_default_route();
        let config = edge_config_with_auth();

        let raw_invite = concat!(
            "INVITE sip:13800138000@edge.example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-invite-1\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:13800138000@example.com>\r\n",
            "Call-ID: invite-auth-test@example.com\r\n",
            "CSeq: 1 INVITE\r\n",
            "Content-Length: 0\r\n\r\n"
        );

        // 1. Initial INVITE should be challenged with 407 Proxy Authentication Required
        let datagrams = handle_datagram(raw_invite.as_bytes(), peer(), &edge_state, &config).await;

        assert_eq!(datagrams.len(), 1);
        let challenge_resp = datagram_text(&datagrams[0]);
        assert!(challenge_resp.starts_with("SIP/2.0 407 Proxy Authentication Required\r\n"));
        assert!(challenge_resp.contains("Proxy-Authenticate: Digest"));

        // Extract nonce from Proxy-Authenticate
        let nonce = challenge_resp
            .lines()
            .find(|l| l.starts_with("Proxy-Authenticate:"))
            .unwrap()
            .split("nonce=\"")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap();

        // 2. Build second INVITE with valid Proxy-Authorization
        let response = crate::auth::digest_response(
            "1001",
            "secret",
            "vos-rs",
            nonce,
            "INVITE",
            "sip:13800138000@edge.example.com",
            Some(("auth", "00000002", "cnonce123")),
        );

        let auth_hdr = format!(
            "Digest username=\"1001\", realm=\"vos-rs\", nonce=\"{nonce}\", uri=\"sip:13800138000@edge.example.com\", response=\"{response}\", algorithm=MD5, qop=auth, nc=00000002, cnonce=\"cnonce123\""
        );

        let raw_invite_auth = format!(
            "INVITE sip:13800138000@edge.example.com SIP/2.0\r\n\
             Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-invite-2\r\n\
             From: <sip:1001@example.com>;tag=from-tag\r\n\
             To: <sip:13800138000@edge.example.com>\r\n\
             Call-ID: invite-auth-test@example.com\r\n\
             CSeq: 2 INVITE\r\n\
             Proxy-Authorization: {auth_hdr}\r\n\
             Content-Length: 0\r\n\r\n"
        );

        let datagrams_auth =
            handle_datagram(raw_invite_auth.as_bytes(), peer(), &edge_state, &config).await;

        // Second INVITE should bypass challenge and be routed (returning 100 Trying and forwarding INVITE)
        assert_eq!(datagrams_auth.len(), 2);
        let resp_100 = datagram_text(&datagrams_auth[0]);
        assert!(resp_100.starts_with("SIP/2.0 100 Trying\r\n"));

        let forwarded_invite = datagram_text(&datagrams_auth[1]);
        assert!(forwarded_invite
            .starts_with("INVITE sip:13800138000@gw1.example.com:5060;transport=udp SIP/2.0\r\n"));
    }

    #[tokio::test]
    async fn test_invite_gateway_bypass_auth() {
        let edge_state = state_with_default_route();

        // Add peer IP to test gateways bypass list
        edge_state
            .test_gateways
            .lock()
            .unwrap()
            .push("192.0.2.10".to_string());

        let config = edge_config_with_auth();

        let raw_invite = concat!(
            "INVITE sip:13800138000@edge.example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-invite-gw\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:13800138000@example.com>\r\n",
            "Call-ID: invite-gw-bypass-test@example.com\r\n",
            "CSeq: 1 INVITE\r\n",
            "Content-Length: 0\r\n\r\n"
        );

        // INVITE from gateway IP should bypass challenge completely and be routed
        let datagrams = handle_datagram(raw_invite.as_bytes(), peer(), &edge_state, &config).await;

        assert_eq!(datagrams.len(), 2);
        let resp_100 = datagram_text(&datagrams[0]);
        assert!(resp_100.starts_with("SIP/2.0 100 Trying\r\n"));
    }
