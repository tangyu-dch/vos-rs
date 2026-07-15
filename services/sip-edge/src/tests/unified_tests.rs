
// =========================================================================
// PARAMETERIZED SCENARIO RUNNER
// =========================================================================
// This test case enables running specific test scenarios dynamically
// using environment variables (e.g. VOS_RS_TEST_SCENARIO=sbc cargo test)
#[test]
fn test_scenario_runner() {
    let scenario = std::env::var("VOS_RS_TEST_SCENARIO").unwrap_or_default();
    if scenario.is_empty() {
        return; // Skip parameter-based run if variable is not set
    }
    
    println!("Running parameter-based unified test scenario: {}", scenario);
    match scenario.as_str() {
        "register" => {
            register_stores_contact_and_returns_binding();
            register_query_returns_existing_contact();
            unregister_removes_contact();
        }
        "invite_basic" => {
            invalid_invite_receives_bad_request();
            replies_to_invite_with_trying_and_dispatches_outbound_invite();
            retransmitted_invite_replays_trying_without_duplicate_outbound_invite();
        }
        "sbc" => {
            test_sbc_ip_acl();
            test_sbc_cps_rate_limiting();
            test_sbc_concurrency_limiting();
            test_sbc_dynamic_lock_brute_force();
            test_sbc_rules_dynamic_reload();
        }
        "conference" => {
            test_sip_invite_join_and_leave_conference();
            test_conference_participant_muting();
        }
        "billing" => {
            test_balance_exhaustion_disconnect_flow();
        }
        "monitoring" => {
            test_call_monitoring();
        }
        "all" => {
            println!("Running all key scenarios...");
            register_stores_contact_and_returns_binding();
            replies_to_invite_with_trying_and_dispatches_outbound_invite();
            test_sbc_ip_acl();
            test_sip_invite_join_and_leave_conference();
            test_conference_participant_muting();
            test_call_monitoring();
            test_balance_exhaustion_disconnect_flow();
        }
        _ => {
            println!("Unknown scenario: {}", scenario);
        }
    }
}

// ========================================== 
// FILE: helpers.rs
// ========================================== 
    fn sdp_body() -> &'static str {
        concat!(
            "v=0\r\n",
            "o=caller 1 1 IN IP4 192.0.2.10\r\n",
            "s=caller\r\n",
            "c=IN IP4 192.0.2.10\r\n",
            "t=0 0\r\n",
            "m=audio 49170 RTP/AVP 0\r\n",
            "a=rtpmap:0 PCMU/8000\r\n"
        )
    }

    use super::media::MediaConfig;

    use super::config;

    fn state_with_default_route() -> EdgeState {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        EdgeState::new(CallManager::new(RouteTable::new(vec![Route::new(
            "default",
            "",
            100,
            RouteTarget::new("gw1", "gw1.example.com", Some(5060)),
        )]), tx))
    }

    fn state_with_default_route_and_config(config: &EdgeConfig) -> EdgeState {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        EdgeState::with_config(
            CallManager::new(RouteTable::new(vec![Route::new(
                "default",
                "",
                100,
                RouteTarget::new("gw1", "gw1.example.com", Some(5060)),
            )]), tx),
            config,
        )
    }

    fn state_with_gateway_uri(uri: &str) -> EdgeState {
        let parsed = SipUri::from_str(uri).unwrap();
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        EdgeState::new(CallManager::new(RouteTable::new(vec![Route::new(
            "default",
            "",
            100,
            RouteTarget::new("gw1", parsed.host, parsed.port),
        )]), tx))
    }

    fn state_without_routes() -> EdgeState {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        EdgeState::new(CallManager::new(RouteTable::default(), tx))
    }

    use std::sync::atomic::{AtomicU16, Ordering};

    thread_local! {
        static THREAD_PORT_OFFSET: u16 = PORT_COUNTER.fetch_add(10, Ordering::Relaxed);
    }

    static PORT_COUNTER: AtomicU16 = AtomicU16::new(10);

    fn get_thread_ports() -> (u16, u16) {
        let offset = THREAD_PORT_OFFSET.with(|o| *o);
        let port_min = 40_000 + offset;
        let port_max = port_min + 4;
        (port_min, port_max)
    }

    fn get_test_port_min() -> u16 {
        get_thread_ports().0
    }

    fn edge_config() -> EdgeConfig {
        let (port_min, port_max) = get_thread_ports();
        EdgeConfig {
            advertised_addr: "edge.example.com:5060".to_string(),
            database_url: None,
            nats_url: None,
            nats_cdr_stream: None,
            nats_cdr_subject: None,
            redis_url: None,
            media: MediaConfig::new("203.0.113.10", port_min, port_max),
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
        }
    }

    fn edge_config_with_auth() -> EdgeConfig {
        let (port_min, port_max) = get_thread_ports();
        EdgeConfig {
            advertised_addr: "edge.example.com:5060".to_string(),
            database_url: None,
            nats_url: None,
            nats_cdr_stream: None,
            nats_cdr_subject: None,
            redis_url: None,
            media: MediaConfig::new("203.0.113.10", port_min, port_max),
            auth: AuthConfig::new(
                "vos-rs",
                "test-nonce",
                HashMap::from([("1001".to_string(), "secret".to_string())]),
            ),
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
        }
    }

    fn peer() -> SocketAddr {
        "192.0.2.10:5060".parse().unwrap()
    }

    fn datagram_text(datagram: &PendingDatagram) -> String {
        String::from_utf8(datagram.bytes.clone()).expect("datagram should be UTF-8")
    }

    fn response_text(datagrams: &[PendingDatagram], status_line: &str) -> String {
        datagrams
            .iter()
            .map(datagram_text)
            .find(|text| text.starts_with(status_line))
            .unwrap_or_else(|| panic!("missing {status_line} datagram"))
    }

    async fn send_invite(edge_state: &EdgeState, call_id: &str) {
        let invite = format!(
            concat!(
                "INVITE sip:13800138000@example.com SIP/2.0\r\n",
                "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-{call_id}\r\n",
                "Max-Forwards: 70\r\n",
                "From: <sip:1001@example.com>;tag=from-tag\r\n",
                "To: <sip:13800138000@example.com>\r\n",
                "Call-ID: {call_id}\r\n",
                "CSeq: 1 INVITE\r\n",
                "Content-Length: 0\r\n",
                "\r\n"
            ),
            call_id = call_id
        );

        let datagrams =
            handle_datagram(invite.as_bytes(), peer(), edge_state, &edge_config()).await;
        assert_eq!(datagrams.len(), 2);
    }

    async fn register_contact(edge_state: &EdgeState, user: &str, host: &str, port: u16) {
        let register = format!(
            concat!(
                "REGISTER sip:example.com SIP/2.0\r\n",
                "Via: SIP/2.0/UDP {host}:{port};branch=z9hG4bK-reg-{user}\r\n",
                "From: <sip:{user}@example.com>;tag=from-tag\r\n",
                "To: <sip:{user}@example.com>\r\n",
                "Call-ID: reg-{user}@example.com\r\n",
                "CSeq: 1 REGISTER\r\n",
                "Contact: <sip:{user}@{host}:{port};transport=udp>;expires=120\r\n",
                "Content-Length: 0\r\n",
                "\r\n"
            ),
            user = user,
            host = host,
            port = port
        );

        let peer = format!("{host}:{port}").parse().unwrap();
        let datagrams =
            handle_datagram(register.as_bytes(), peer, edge_state, &edge_config()).await;
        assert_eq!(datagrams.len(), 1);
        assert!(datagram_text(&datagrams[0]).starts_with("SIP/2.0 200 OK\r\n"));
    }

    async fn send_gateway_ok(edge_state: &EdgeState, call_id: &str) {
        let gateway_response = format!(
            concat!(
                "SIP/2.0 200 OK\r\n",
                "Via: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-vosrs\r\n",
                "From: <sip:1001@example.com>;tag=from-tag\r\n",
                "To: <sip:13800138000@example.com>;tag=gw-tag\r\n",
                "Call-ID: {call_id}\r\n",
                "CSeq: 1 INVITE\r\n",
                "Content-Length: 0\r\n",
                "\r\n"
            ),
            call_id = call_id
        );

        let datagrams = handle_datagram(
            gateway_response.as_bytes(),
            "198.51.100.20:5060".parse().unwrap(),
            edge_state,
            &edge_config(),
        )
        .await;
        assert_eq!(datagrams.len(), 2);
        assert!(datagrams
            .iter()
            .any(|datagram| datagram_text(datagram).starts_with("ACK ")));
    }

    #[test]
    fn test_rtcp_mos_calculation() {
        use super::media::RtcpQualitySnapshot;

        // Case 1: Perfect RTCP metrics (0 delay, 0 loss)
        let caller = RtcpQualitySnapshot {
            reports: 1,
            sender_reports: 0,
            receiver_reports: 1,
            report_blocks: 1,
            last_fraction_lost: Some(0),
            max_fraction_lost: Some(0),
            last_cumulative_lost: Some(0),
            max_cumulative_lost: Some(0),
            last_jitter: Some(0),
            max_jitter: Some(0),
            last_sender_report: Some(0),
            delay_since_last_sender_report: Some(0),
            last_rtt_ms: Some(0),
            max_rtt_ms: Some(0),
        };
        let gateway = RtcpQualitySnapshot {
            reports: 1,
            sender_reports: 0,
            receiver_reports: 1,
            report_blocks: 1,
            last_fraction_lost: Some(0),
            max_fraction_lost: Some(0),
            last_cumulative_lost: Some(0),
            max_cumulative_lost: Some(0),
            last_jitter: Some(0),
            max_jitter: Some(0),
            last_sender_report: Some(0),
            delay_since_last_sender_report: Some(0),
            last_rtt_ms: Some(0),
            max_rtt_ms: Some(0),
        };

        let metrics = super::calculate_mos_for_legs(Some(&caller), Some(&gateway));
        assert!(metrics.mos.is_some());
        let mos_val = metrics.mos.unwrap();
        // Perfect MOS should be close to 4.4
        assert!(mos_val > 4.35 && mos_val <= 4.5);

        // Case 2: Degraded metrics (high loss, some delay)
        let caller_degraded = RtcpQualitySnapshot {
            reports: 1,
            sender_reports: 0,
            receiver_reports: 1,
            report_blocks: 1,
            last_fraction_lost: Some(25), // 25/256 ≈ 9.7% loss
            max_fraction_lost: Some(25),
            last_cumulative_lost: Some(0),
            max_cumulative_lost: Some(0),
            last_jitter: Some(0),
            max_jitter: Some(0),
            last_sender_report: Some(0),
            delay_since_last_sender_report: Some(0),
            last_rtt_ms: Some(150),
            max_rtt_ms: Some(150),
        };
        let gateway_degraded = RtcpQualitySnapshot {
            reports: 1,
            sender_reports: 0,
            receiver_reports: 1,
            report_blocks: 1,
            last_fraction_lost: Some(5), // 5/256 ≈ 2% loss
            max_fraction_lost: Some(5),
            last_cumulative_lost: Some(0),
            max_cumulative_lost: Some(0),
            last_jitter: Some(0),
            max_jitter: Some(0),
            last_sender_report: Some(0),
            delay_since_last_sender_report: Some(0),
            last_rtt_ms: Some(50),
            max_rtt_ms: Some(50),
        };

        let metrics_degraded =
            super::calculate_mos_for_legs(Some(&caller_degraded), Some(&gateway_degraded));
        assert!(metrics_degraded.mos.is_some());
        let mos_val_degraded = metrics_degraded.mos.unwrap();
        // High loss and delay should degrade MOS significantly
        assert!(mos_val_degraded < 3.0);
    }


// ========================================== 
// FILE: register_tests.rs
// ========================================== 
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


// ========================================== 
// FILE: invite_basic_tests.rs
// ========================================== 
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
        assert!(response.contains("To: <sip:13800138000@example.com>\r\n"));
        assert!(!response.contains("To: <sip:13800138000@example.com>;tag="));

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


// ========================================== 
// FILE: invite_advanced_tests.rs
// ========================================== 
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

        assert_eq!(datagrams.len(), 2);
        let response = response_text(&datagrams, "SIP/2.0 200 OK");
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
    async fn late_provisional_response_after_invite_ok_is_dropped() {
        let edge_state = state_with_default_route();
        let call_id = "invite-late-provisional@example.com";
        send_invite(&edge_state, call_id).await;
        send_gateway_ok(&edge_state, call_id).await;

        let late_response = format!(
            concat!(
                "SIP/2.0 183 Session Progress\r\n",
                "Via: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-vosrs\r\n",
                "From: <sip:1001@example.com>;tag=from-tag\r\n",
                "To: <sip:13800138000@example.com>;tag=gw-tag\r\n",
                "Call-ID: {call_id}\r\n",
                "CSeq: 1 INVITE\r\n",
                "Content-Length: 0\r\n",
                "\r\n"
            ),
            call_id = call_id
        );

        let datagrams = handle_datagram(
            late_response.as_bytes(),
            "198.51.100.20:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        assert!(datagrams.is_empty());
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
        assert_eq!(answer_datagrams.len(), 2);
        let response = response_text(&answer_datagrams, "SIP/2.0 200 OK");
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

        assert_eq!(answer_datagrams.len(), 2);
        let response_to_caller = response_text(&answer_datagrams, "SIP/2.0 200 OK");
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

        assert_eq!(res_datagrams.len(), 3);
        assert!(res_datagrams
            .iter()
            .any(|datagram| datagram.target == peer().to_string()));

        let cancel_datagram = res_datagrams
            .iter()
            .find(|datagram| datagram_text(datagram).starts_with("CANCEL "))
            .expect("losing fork must be cancelled");
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

    #[tokio::test]
    async fn test_tenant_domain_isolation() {
        let edge_state = state_with_default_route();

        // Register 1002@tenant2.com
        let contact_update_body = concat!(
            "v=0\r\n",
            "o=callee 1 1 IN IP4 192.0.2.11\r\n",
            "s=callee\r\n",
            "c=IN IP4 192.0.2.11\r\n",
            "t=0 0\r\n",
            "m=audio 49170 RTP/AVP 0 101\r\n"
        );
        let mut registrar = edge_state.registrar.write().await;
        let register_req = String::from(
            "REGISTER sip:tenant2.com SIP/2.0\r\n\
             Via: SIP/2.0/UDP 192.0.2.11:5060;branch=z9hG4bK-reg\r\n\
             From: <sip:1002@tenant2.com>;tag=tag1\r\n\
             To: <sip:1002@tenant2.com>\r\n\
             Call-ID: reg-call-id\r\n\
             CSeq: 1 REGISTER\r\n\
             Contact: <sip:1002@192.0.2.11:5060>\r\n\
             Expires: 3600\r\n\
             Content-Length: 0\r\n\r\n"
        );
        let req = sip_core::parse_message(register_req.as_bytes()).unwrap();
        let sip_req = match req {
            sip_core::SipMessage::Request(r) => r,
            _ => panic!("expected request"),
        };
        registrar.handle_register(&sip_req, "192.0.2.11:5060".parse().unwrap(), SystemTime::now(), None).await.unwrap();
        drop(registrar);

        // Send invite from 1001@tenant1.com to 1002@tenant2.com
        let invite_body = contact_update_body;
        let cross_invite = format!(
            "INVITE sip:1002@tenant2.com SIP/2.0\r\n\
             Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-cross\r\n\
             From: <sip:1001@tenant1.com>;tag=from-tag\r\n\
             To: <sip:1002@tenant2.com>\r\n\
             Call-ID: cross-tenant@example.com\r\n\
             CSeq: 1 INVITE\r\n\
             Content-Type: application/sdp\r\n\
             Content-Length: {}\r\n\
             \r\n\
             {}",
            invite_body.len(),
            invite_body
        );

        let datagrams = handle_datagram(
            cross_invite.as_bytes(),
            peer(),
            &edge_state,
            &edge_config(),
        )
        .await;

        assert_eq!(datagrams.len(), 1);
        let response = datagram_text(&datagrams[0]);
        assert!(response.contains("SIP/2.0 403 Forbidden"));
        assert!(response.contains("X-VOS-RS-Error: Cross-tenant calling is disabled"));
    }

    #[tokio::test]
    async fn test_modular_config_loading() {
        use crate::config::EdgeConfig;
        use std::fs;
        // Create temporary config yaml files
        let main_yaml = r#"
connections:
  database:
    host: "127.0.0.1"
    port: 5432
    username: "tangyu"
    password: ""
    database: "vos_rs"
  nats:
    url: "nats://127.0.0.1:4222"
    cdr_stream: "STREAM_A"
    cdr_subject: "SUB_A"
sip_edge:
  network:
    advertised_addr: "127.0.0.1:5070"
  routing:
    gateway_health_checks_enabled: false
  media:
    nodes:
      - id: "media-edge-test"
        type: "remote"
        control_url: "uds:///tmp/media-edge-test.sock"
        advertised_addr: "127.0.0.1"
        port_min: 40000
        port_max: 40100
        weight: 1
  billing:
    balance_enforcement_enabled: false
    settlement_enabled: false
  performance:
    cdr_persistence_enabled: false
"#;

        fs::write("test_config.yaml", main_yaml).unwrap();

        let config = EdgeConfig::load_from_file("test_config.yaml");

        // Clean up
        let _ = fs::remove_file("test_config.yaml");

        assert_eq!(config.advertised_addr, "127.0.0.1:5070");
        assert_eq!(config.database_url, Some("postgres://tangyu@127.0.0.1:5432/vos_rs".to_string()));
        assert_eq!(config.nats_url, Some("nats://127.0.0.1:4222".to_string()));
        assert_eq!(config.nats_cdr_stream, Some("STREAM_A".to_string()));
        assert_eq!(config.nats_cdr_subject, Some("SUB_A".to_string()));
        assert!(!config.gateway_health_checks_enabled);
        assert_eq!(config.media_cluster.nodes.len(), 1);
        assert_eq!(
            config.media_cluster.nodes[0].control_url.as_deref(),
            Some("uds:///tmp/media-edge-test.sock")
        );
        assert!(!config.balance_enforcement_enabled);
        assert!(!config.billing_settlement_enabled);
        assert!(!config.cdr_persistence_enabled);
    }


// ========================================== 
// FILE: media_tests.rs
// ========================================== 
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

        assert_eq!(datagrams.len(), 2);
        let response = response_text(&datagrams, "SIP/2.0 200 OK");
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
        assert_eq!(answer_datagrams.len(), 2);

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
        assert_eq!(reinvite_resp_datagrams.len(), 2);

        // Verify the outgoing response to caller has rewritten SDP presenting caller_relay (reusing the same port!)
        let forwarded_resp = response_text(&reinvite_resp_datagrams, "SIP/2.0 200 OK");
        assert!(forwarded_resp.contains(&format!("m=audio {} RTP/AVP 0\r\n", caller_relay.port)));

        // Verify target for caller_relay is still gateway target (198.51.100.20:49172)
        assert_eq!(
            edge_state.media_relay.target_for_port(caller_relay.port),
            Some("198.51.100.20:49172".parse().unwrap())
        );
    }


// ========================================== 
// FILE: nat_transport_tests.rs
// ========================================== 
    #[tokio::test]
    async fn test_nat_traversal_registered_contact_override() {
        let edge_state = state_with_default_route();

        // 1. Register contact 1001 with private contact but public received_from socket
        let register = "REGISTER sip:example.com SIP/2.0\r\n\
             Via: SIP/2.0/UDP 192.0.2.10:5070;branch=z9hG4bK-regnat\r\n\
             From: <sip:1001@example.com>;tag=from-tag\r\n\
             To: <sip:1001@example.com>\r\n\
             Call-ID: reg-nat-01\r\n\
             CSeq: 1 REGISTER\r\n\
             Contact: <sip:1001@192.168.1.100:5060;transport=udp>;expires=60\r\n\
             Content-Length: 0\r\n\r\n";
        let _ = handle_datagram(
            register.as_bytes(),
            "192.0.2.10:5070".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // 2. Receive an inbound INVITE to 1001
        let call_id = "invite-nat-01";
        let body = sdp_body();
        let invite = format!(
            concat!(
                "INVITE sip:1001@example.com SIP/2.0\r\n",
                "Via: SIP/2.0/UDP 192.0.2.20:5060;branch=z9hG4bK-invite-nat\r\n",
                "Max-Forwards: 70\r\n",
                "From: <sip:1002@example.com>;tag=caller-tag\r\n",
                "To: <sip:1001@example.com>\r\n",
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

        let datagrams = handle_datagram(
            invite.as_bytes(),
            "192.0.2.20:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // Verify INVITE is forwarded to the public NAT address of client 1001, NOT the private Contact IP!
        assert_eq!(datagrams.len(), 2);
        assert_eq!(datagrams[1].target, "192.0.2.10:5070");
        let forwarded_msg = datagram_text(&datagrams[1]);
        assert!(forwarded_msg
            .starts_with("INVITE sip:1001@192.168.1.100:5060;transport=udp SIP/2.0\r\n"));
    }

    #[tokio::test]
    async fn test_nat_traversal_in_dialog_callee_override() {
        let edge_state = state_with_default_route();

        // 1. Register contact 1001 with private Contact but public received_from socket
        let register = "REGISTER sip:example.com SIP/2.0\r\n\
             Via: SIP/2.0/UDP 198.51.100.20:5070;branch=z9hG4bK-regnat\r\n\
             From: <sip:1001@example.com>;tag=from-tag\r\n\
             To: <sip:1001@example.com>\r\n\
             Call-ID: reg-nat-01\r\n\
             CSeq: 1 REGISTER\r\n\
             Contact: <sip:1001@192.168.100.200:5060;transport=udp>;expires=60\r\n\
             Content-Length: 0\r\n\r\n";
        let _ = handle_datagram(
            register.as_bytes(),
            "198.51.100.20:5070".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // 2. Establish initial call: inbound INVITE from caller 1002 to registered contact 1001
        let call_id = "nat-indialog-01";
        let body = sdp_body();
        let invite = format!(
            concat!(
                "INVITE sip:1001@example.com SIP/2.0\r\n",
                "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-caller-1\r\n",
                "Max-Forwards: 70\r\n",
                "From: <sip:1002@example.com>;tag=caller-tag\r\n",
                "To: <sip:1001@example.com>\r\n",
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

        let datagrams = handle_datagram(
            invite.as_bytes(),
            "192.0.2.10:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // Verify INVITE is forwarded to callee 1001 at public NAT address "198.51.100.20:5070"
        assert_eq!(datagrams.len(), 2);
        assert_eq!(datagrams[1].target, "198.51.100.20:5070");

        // 3. Callee 1001 responds 200 OK from public NAT address 198.51.100.20:5070
        let ok_body = sdp_body();
        let ok_200 = format!(
            concat!(
                "SIP/2.0 200 OK\r\n",
                "Via: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-vosrs\r\n",
                "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-caller-1\r\n",
                "From: <sip:1002@example.com>;tag=caller-tag\r\n",
                "To: <sip:1001@example.com>;tag=callee-tag\r\n",
                "Call-ID: {call_id}\r\n",
                "CSeq: 1 INVITE\r\n",
                "Contact: <sip:1001@192.168.100.200:5060>\r\n",
                "Content-Type: application/sdp\r\n",
                "Content-Length: {}\r\n",
                "\r\n",
                "{}"
            ),
            ok_body.len(),
            ok_body,
            call_id = call_id
        );

        let _ = handle_datagram(
            ok_200.as_bytes(),
            "198.51.100.20:5070".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // Verify outbound_peer NAT target and callee_behind_nat flag are registered
        {
            let tx = edge_state.inbound_transactions.get(call_id).unwrap();
            assert_eq!(tx.outbound_peer.as_deref(), Some("198.51.100.20:5070"));
            assert!(tx.callee_behind_nat);
        }

        // 4. Caller sends BYE to callee
        let bye = format!(
            "BYE sip:1001@192.168.100.200:5060 SIP/2.0\r\n\
             Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-caller-2\r\n\
             From: <sip:1002@example.com>;tag=caller-tag\r\n\
             To: <sip:1001@example.com>;tag=callee-tag\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 2 BYE\r\n\
             Content-Length: 0\r\n\r\n"
        );

        let bye_datagrams = handle_datagram(
            bye.as_bytes(),
            "192.0.2.10:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // Verify BYE is routed to the public source socket address of the callee (198.51.100.20:5070), NOT the private Contact IP!
        assert_eq!(bye_datagrams.len(), 2);
        assert_eq!(bye_datagrams[1].target, "198.51.100.20:5070");
        let forwarded_bye = datagram_text(&bye_datagrams[1]);
        assert!(forwarded_bye.starts_with("BYE sip:1001@192.168.100.200:5060 SIP/2.0\r\n"));
    }

    #[tokio::test]
    async fn test_nat_keepalive_background_loop() {
        let edge_state = Arc::new(state_with_default_route());
        let socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        edge_state.set_socket(Arc::clone(&socket));

        let local_addr = socket.local_addr().unwrap();

        // 1. Register a contact pointing to local receiver port so we can capture the keepalive datagram
        let register = format!(
            "REGISTER sip:example.com SIP/2.0\r\n\
             Via: SIP/2.0/UDP {addr};branch=z9hG4bK-regka\r\n\
             From: <sip:1001@example.com>;tag=from-tag\r\n\
             To: <sip:1001@example.com>\r\n\
             Call-ID: reg-ka-01\r\n\
             CSeq: 1 REGISTER\r\n\
             Contact: <sip:1001@192.168.1.100:5060;transport=udp>;expires=60\r\n\
             Content-Length: 0\r\n\r\n",
            addr = local_addr
        );
        let _ = handle_datagram(register.as_bytes(), local_addr, &edge_state, &edge_config()).await;

        // Discard the 200 OK registration response from the socket buffer
        let mut resp_buf = [0u8; 1000];
        let (resp_size, _) =
            tokio::time::timeout(Duration::from_millis(500), socket.recv_from(&mut resp_buf))
                .await
                .expect("timeout waiting for 200 OK registration response")
                .unwrap();
        assert!(std::str::from_utf8(&resp_buf[..resp_size])
            .unwrap()
            .starts_with("SIP/2.0 200 OK\r\n"));

        // 2. Start the NAT keepalive loop
        spawn_nat_keepalive_loop(Arc::clone(&edge_state), Arc::clone(&socket));

        // 3. Receive the NAT keepalive packet
        let mut buffer = [0u8; 100];
        let (size, src) =
            tokio::time::timeout(Duration::from_millis(500), socket.recv_from(&mut buffer))
                .await
                .expect("timeout waiting for keepalive probe")
                .unwrap();

        // Verify the keepalive probe matches single CRLF "\r\n"
        assert_eq!(&buffer[..size], b"\r\n");
        assert_eq!(src, local_addr);
    }

    #[tokio::test]
    async fn test_websocket_transport() {
        use futures::{SinkExt, StreamExt};
        use tokio_tungstenite::connect_async;
        use tokio_tungstenite::tungstenite::Message as WsMessage;

        let edge_state = Arc::new(state_with_default_route());

        // Start WS listener on random port
        let ws_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let ws_addr = ws_listener.local_addr().unwrap();

        let edge_state_clone = Arc::clone(&edge_state);
        tokio::spawn(async move {
            let (stream, peer) = ws_listener.accept().await.unwrap();
            let ws_stream = tokio_tungstenite::accept_async(stream).await.unwrap();
            let (tx, rx) = tokio::sync::mpsc::channel(100);
            edge_state_clone.register_tcp_connection(peer, tx.clone());

            let on_msg_state = Arc::clone(&edge_state_clone);
            handle_ws_connection(
                ws_stream,
                peer,
                tx,
                rx,
                move |msg_bytes: Vec<u8>,
                      peer_addr: SocketAddr,
                      connection_tx: tokio::sync::mpsc::Sender<Vec<u8>>| {
                    let state = Arc::clone(&on_msg_state);
                    async move {
                        let datagrams =
                            handle_datagram(&msg_bytes, peer_addr, &state, &edge_config()).await;
                        for d in datagrams {
                            let _ = connection_tx.send(d.bytes).await;
                        }
                    }
                },
            )
            .await;
        });

        // Connect client
        let (mut client_ws, _) = connect_async(format!("ws://{}", ws_addr)).await.unwrap();

        // Send REGISTER request over WS
        let register = concat!(
            "REGISTER sip:example.com SIP/2.0\r\n",
            "Via: SIP/2.0/WS 127.0.0.1:5062;branch=z9hG4bK-ws-reg\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:1001@example.com>\r\n",
            "Call-ID: ws-reg-001\r\n",
            "CSeq: 1 REGISTER\r\n",
            "Contact: <sip:1001@127.0.0.1:5062;transport=ws>;expires=60\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        );

        client_ws
            .send(WsMessage::Text(register.to_string()))
            .await
            .unwrap();

        // Receive response over WS
        let msg = tokio::time::timeout(Duration::from_millis(1000), client_ws.next())
            .await
            .expect("timeout waiting for WS response")
            .unwrap()
            .unwrap();

        let resp_text = match msg {
            WsMessage::Text(t) => t,
            _ => panic!("expected text frame"),
        };

        assert!(resp_text.starts_with("SIP/2.0 200 OK\r\n"));
        assert!(resp_text.contains("Call-ID: ws-reg-001\r\n"));
    }

    #[tokio::test]
    async fn test_tcp_stream_framing() {
        use crate::transport::read_frame;

        // Case 1: Complete single message
        let mut buf = b"SIP/2.0 200 OK\r\nContent-Length: 5\r\n\r\nhello".to_vec();
        let frame = read_frame(&mut buf).unwrap();
        assert_eq!(
            frame,
            b"SIP/2.0 200 OK\r\nContent-Length: 5\r\n\r\nhello".to_vec()
        );
        assert!(buf.is_empty());

        // Case 2: Compact length header l
        let mut buf = b"SIP/2.0 200 OK\r\nl: 5\r\n\r\nhello".to_vec();
        let frame = read_frame(&mut buf).unwrap();
        assert_eq!(frame, b"SIP/2.0 200 OK\r\nl: 5\r\n\r\nhello".to_vec());
        assert!(buf.is_empty());

        // Case 3: Partial message (header not complete)
        let mut buf = b"SIP/2.0 200 OK\r\nContent-L".to_vec();
        assert!(read_frame(&mut buf).is_none());
        assert_eq!(buf.len(), 25);

        // Case 4: Header complete but body incomplete
        let mut buf = b"SIP/2.0 200 OK\r\nContent-Length: 10\r\n\r\nbody".to_vec();
        assert!(read_frame(&mut buf).is_none());
        assert_eq!(buf.len(), 42);

        // Feed rest of body
        buf.extend_from_slice(b" rest1");
        let frame = read_frame(&mut buf).unwrap();
        assert_eq!(
            frame,
            b"SIP/2.0 200 OK\r\nContent-Length: 10\r\n\r\nbody rest1".to_vec()
        );
        assert!(buf.is_empty());

        // Case 5: Multiple concatenated messages
        let mut buf = b"SIP/2.0 200 OK\r\nContent-Length: 3\r\n\r\n123SIP/2.0 200 OK\r\nContent-Length: 3\r\n\r\n456".to_vec();
        let frame1 = read_frame(&mut buf).unwrap();
        assert_eq!(
            frame1,
            b"SIP/2.0 200 OK\r\nContent-Length: 3\r\n\r\n123".to_vec()
        );
        let frame2 = read_frame(&mut buf).unwrap();
        assert_eq!(
            frame2,
            b"SIP/2.0 200 OK\r\nContent-Length: 3\r\n\r\n456".to_vec()
        );
        assert!(buf.is_empty());
    }

    #[tokio::test]
    async fn test_tcp_tls_transport_dispatch_and_reuse() {
        use tokio::io::AsyncReadExt;

        let edge_state = Arc::new(state_with_default_route());
        let edge_config = edge_config();

        // 1. Setup local TCP listener
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let listen_addr = listener.local_addr().unwrap();

        // Spawn local test TCP server task
        let (server_tx, mut server_rx) = tokio::sync::mpsc::channel(10);
        tokio::spawn(async move {
            if let Ok((mut stream, _)) = listener.accept().await {
                let mut buf = vec![0u8; 1024];
                while let Ok(n) = stream.read(&mut buf).await {
                    if n == 0 {
                        break;
                    }
                    let _ = server_tx.send(buf[..n].to_vec()).await;
                }
            }
        });

        // Send a mock SIP request targeting this server using TCP transport (Via has SIP/2.0/TCP)
        let request_bytes = format!(
            "INVITE sip:1002@example.com SIP/2.0\r\n\
             Via: SIP/2.0/TCP {listen_addr};branch=z9hG4bK-tcp-001\r\n\
             Content-Length: 0\r\n\r\n"
        )
        .into_bytes();

        let datagram = PendingDatagram::new(listen_addr.to_string(), request_bytes.clone());
        let dummy_udp = UdpSocket::bind("127.0.0.1:0").await.unwrap();

        let res = edge_state
            .send_sip_datagram(datagram, &dummy_udp, &edge_config)
            .await;
        assert!(res.is_ok());

        // Verify message received by the server
        let received = tokio::time::timeout(Duration::from_millis(500), server_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(received, request_bytes);

        // Check that the connection was registered in the connection pool
        {
            assert!(edge_state.tcp_connections.contains_key(&listen_addr));
        }

        // Send a second message to check connection reuse
        let request_bytes2 = format!(
            "INVITE sip:1002@example.com SIP/2.0\r\n\
             Via: SIP/2.0/TCP {listen_addr};branch=z9hG4bK-tcp-002\r\n\
             Content-Length: 0\r\n\r\n"
        )
        .into_bytes();

        let datagram2 = PendingDatagram::new(listen_addr.to_string(), request_bytes2.clone());
        let res2 = edge_state
            .send_sip_datagram(datagram2, &dummy_udp, &edge_config)
            .await;
        assert!(res2.is_ok());

        let received2 = tokio::time::timeout(Duration::from_millis(500), server_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(received2, request_bytes2);
    }


// ========================================== 
// FILE: sbc_tests.rs
// ========================================== 
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

    #[tokio::test]
    async fn test_sbc_rules_dynamic_reload() {
        let config = edge_config();
        let edge_state = state_with_default_route_and_config(&config);

        let ip = "192.0.2.200".parse().unwrap();
        // 默认应当允许
        assert!(edge_state.sbc_engine.is_allowed(ip));

        // 动态添加至黑名单
        edge_state.sbc_engine.update_rules(&[], &["192.0.2.200/32"]);
        assert!(!edge_state.sbc_engine.is_allowed(ip));

        // 动态添加至白名单，使其他 IP 被过滤
        edge_state.sbc_engine.update_rules(&["192.0.2.50/32"], &[]);
        assert!(!edge_state.sbc_engine.is_allowed(ip));
        assert!(edge_state.sbc_engine.is_allowed("192.0.2.50".parse().unwrap()));

        // 清理规则，重新允许
        edge_state.sbc_engine.update_rules(&[], &[]);
        assert!(edge_state.sbc_engine.is_allowed(ip));
    }

    #[tokio::test]
    async fn test_sbc_dynamic_lock_brute_force() {
        let config = edge_config_with_auth();
        let edge_state = state_with_default_route_and_config(&config);

        let peer_addr = "192.0.2.50:5060".parse().unwrap();
        let packet_no_auth = b"REGISTER sip:edge.example.com SIP/2.0\r\n\
                               Via: SIP/2.0/UDP 192.0.2.50:5060;branch=z9hG4bK-reg1\r\n\
                               From: <sip:1001@edge.example.com>;tag=tag1\r\n\
                               To: <sip:1001@edge.example.com>\r\n\
                               Call-ID: brute-force-test-call-id\r\n\
                               CSeq: 1 REGISTER\r\n\
                               Content-Length: 0\r\n\r\n";

        // 1. 发送不带 Authorization 头的包，应该是常规 Challenge (401)，且不触发失败计数
        let d1 = handle_datagram(packet_no_auth, peer_addr, &edge_state, &config).await;
        assert!(!d1.is_empty());
        assert!(datagram_text(&d1[0]).starts_with("SIP/2.0 401 Unauthorized\r\n"));

        // 2. 连续 5 次发送带有错误密码凭证的包，触发封禁
        for i in 0..5 {
            let packet_wrong_auth = format!(
                "REGISTER sip:edge.example.com SIP/2.0\r\n\
                 Via: SIP/2.0/UDP 192.0.2.50:5060;branch=z9hG4bK-regwrong-{i}\r\n\
                 From: <sip:1001@edge.example.com>;tag=tag1\r\n\
                 To: <sip:1001@edge.example.com>\r\n\
                 Call-ID: brute-force-test-call-id-{i}\r\n\
                 CSeq: {} REGISTER\r\n\
                 Authorization: Digest username=\"1001\", realm=\"vos-rs\", nonce=\"test-nonce\", uri=\"sip:edge.example.com\", response=\"wrongresponse\"\r\n\
                 Content-Length: 0\r\n\r\n",
                i + 2
            );
            let res = handle_datagram(packet_wrong_auth.as_bytes(), peer_addr, &edge_state, &config).await;
            assert!(!res.is_empty());
            assert!(datagram_text(&res[0]).starts_with("SIP/2.0 401 Unauthorized\r\n"));
        }

        // 3. 第 6 次请求，此时 IP 已被锁定，SBC check_sbc_filter 直接丢弃，handle_datagram 应返回空
        let d2 = handle_datagram(packet_no_auth, peer_addr, &edge_state, &config).await;
        assert!(d2.is_empty(), "Locked IP should be blocked at SBC level");
    }



// ========================================== 
// FILE: session_timer_tests.rs
// ========================================== 
    /// Verify that the outbound INVITE carries Session-Expires and Supported: timer headers.
    #[tokio::test]
    async fn test_session_timer_header_injected_in_invite() {
        let edge_state = state_with_gateway_uri("sip:198.51.100.20:5060");
        let call_id = "session-timer-header-test-001";

        let sdp_body = "v=0\r\no=- 0 0 IN IP4 192.0.2.10\r\ns=-\r\nc=IN IP4 192.0.2.10\r\nt=0 0\r\nm=audio 49170 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n";
        let invite = format!(
            "INVITE sip:13800138000@example.com SIP/2.0\r\nVia: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-st-hdr\r\nMax-Forwards: 70\r\nFrom: <sip:1001@example.com>;tag=from-tag\r\nTo: <sip:13800138000@example.com>\r\nCall-ID: {call_id}\r\nCSeq: 1 INVITE\r\nContact: <sip:1001@192.0.2.10>\r\nContent-Type: application/sdp\r\nContent-Length: {len}\r\n\r\n{sdp_body}",
            len = sdp_body.len()
        );

        let datagrams = handle_datagram(
            invite.as_bytes(),
            "192.0.2.10:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;
        // Should produce 2 datagrams: 100 Trying to caller + outbound INVITE to gateway
        assert_eq!(datagrams.len(), 2);

        let outbound_invite = datagram_text(&datagrams[1]);
        assert!(
            outbound_invite.contains("Session-Expires: 600;refresher=uac"),
            "outbound INVITE must carry Session-Expires header\n{outbound_invite}"
        );
        assert!(
            outbound_invite.contains("Supported: timer"),
            "outbound INVITE must carry Supported: timer header\n{outbound_invite}"
        );
        assert!(
            outbound_invite.contains("Min-SE: 90"),
            "outbound INVITE must carry Min-SE header\n{outbound_invite}"
        );
    }

    /// Verify that a 200 OK containing Session-Expires stores the value on the transaction.
    #[tokio::test]
    async fn test_session_expires_stored_from_200_ok() {
        let edge_state = state_with_gateway_uri("sip:198.51.100.20:5060");
        let call_id = "session-timer-store-test-001";
        let sdp_body = "v=0\r\no=- 0 0 IN IP4 192.0.2.10\r\ns=-\r\nc=IN IP4 192.0.2.10\r\nt=0 0\r\nm=audio 49170 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n";

        // Step 1: send INVITE to establish transaction
        let invite = format!(
            "INVITE sip:13800138000@example.com SIP/2.0\r\n\
             Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-se-store\r\n\
             Max-Forwards: 70\r\n\
             From: <sip:1001@example.com>;tag=from-tag\r\n\
             To: <sip:13800138000@example.com>\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 1 INVITE\r\n\
             Contact: <sip:1001@192.0.2.10>\r\n\
             Content-Type: application/sdp\r\n\
             Content-Length: {len}\r\n\r\n{sdp_body}",
            len = sdp_body.len()
        );
        handle_datagram(
            invite.as_bytes(),
            "192.0.2.10:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // Step 2: gateway returns 200 OK with Session-Expires
        let sdp_answer = "v=0\r\no=- 0 0 IN IP4 198.51.100.20\r\ns=-\r\nc=IN IP4 198.51.100.20\r\nt=0 0\r\nm=audio 49172 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n";
        let ok_200 = format!(
            "SIP/2.0 200 OK\r\n\
             Via: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-se-store\r\n\
             From: <sip:1001@example.com>;tag=from-tag\r\n\
             To: <sip:13800138000@example.com>;tag=gw-tag\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 1 INVITE\r\n\
             Session-Expires: 600;refresher=uac\r\n\
             Content-Type: application/sdp\r\n\
             Content-Length: {len}\r\n\r\n{sdp_answer}",
            len = sdp_answer.len()
        );
        handle_datagram(
            ok_200.as_bytes(),
            "198.51.100.20:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // Verify session timer was stored on the transaction
        let tx = edge_state.inbound_transactions.get(call_id).expect("transaction must exist");
        assert_eq!(
            tx.session_expires,
            Some(600),
            "session_expires must be stored"
        );
        assert_eq!(
            tx.session_refresher.as_deref(),
            Some("uac"),
            "refresher must be stored"
        );
        assert!(
            tx.last_session_refresh.is_some(),
            "last_session_refresh must be set"
        );
    }

    /// Verify that Re-INVITE resets the last_session_refresh timestamp.
    #[tokio::test]
    async fn test_session_refresh_resets_on_reinvite() {
        let edge_state = state_with_gateway_uri("sip:198.51.100.20:5060");
        let call_id = "session-timer-refresh-test-001";
        let sdp_body = "v=0\r\no=- 0 0 IN IP4 192.0.2.10\r\ns=-\r\nc=IN IP4 192.0.2.10\r\nt=0 0\r\nm=audio 49170 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n";

        // Establish the call
        let invite = format!(
            "INVITE sip:13800138000@example.com SIP/2.0\r\n\
             Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-se-refresh\r\n\
             Max-Forwards: 70\r\n\
             From: <sip:1001@example.com>;tag=from-tag\r\n\
             To: <sip:13800138000@example.com>\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 1 INVITE\r\n\
             Contact: <sip:1001@192.0.2.10>\r\n\
             Content-Type: application/sdp\r\n\
             Content-Length: {len}\r\n\r\n{sdp_body}",
            len = sdp_body.len()
        );
        handle_datagram(
            invite.as_bytes(),
            "192.0.2.10:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        let sdp_answer = "v=0\r\no=- 0 0 IN IP4 198.51.100.20\r\ns=-\r\nc=IN IP4 198.51.100.20\r\nt=0 0\r\nm=audio 49172 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n";
        let ok_200 = format!(
            "SIP/2.0 200 OK\r\n\
             Via: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-se-refresh\r\n\
             From: <sip:1001@example.com>;tag=from-tag\r\n\
             To: <sip:13800138000@example.com>;tag=gw-tag\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 1 INVITE\r\n\
             Session-Expires: 600;refresher=uac\r\n\
             Content-Type: application/sdp\r\n\
             Content-Length: {len}\r\n\r\n{sdp_answer}",
            len = sdp_answer.len()
        );
        handle_datagram(
            ok_200.as_bytes(),
            "198.51.100.20:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // Capture last_session_refresh time before Re-INVITE
        let before_reinvite = {
            let tx_guard = edge_state.inbound_transactions.get(call_id).unwrap(); tx_guard.last_session_refresh
        };

        // Small delay so the timestamp will differ
        tokio::time::sleep(Duration::from_millis(5)).await;

        // Send Re-INVITE (To has tag) — this acts as session refresh
        let reinvite_sdp = sdp_body;
        let reinvite = format!(
            "INVITE sip:13800138000@example.com SIP/2.0\r\n\
             Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-se-refresh-2\r\n\
             Max-Forwards: 70\r\n\
             From: <sip:1001@example.com>;tag=from-tag\r\n\
             To: <sip:13800138000@example.com>;tag=gw-tag\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 2 INVITE\r\n\
             Contact: <sip:1001@192.0.2.10>\r\n\
             Content-Type: application/sdp\r\n\
             Content-Length: {len}\r\n\r\n{reinvite_sdp}",
            len = reinvite_sdp.len()
        );
        handle_datagram(
            reinvite.as_bytes(),
            "192.0.2.10:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        let after_reinvite = {
            let tx_guard = edge_state.inbound_transactions.get(call_id).unwrap(); tx_guard.last_session_refresh
        };

        assert!(before_reinvite.is_some(), "initial timestamp must be set");
        assert!(
            after_reinvite.is_some(),
            "post-reinvite timestamp must be set"
        );
        assert!(
            after_reinvite.unwrap() >= before_reinvite.unwrap(),
            "last_session_refresh must be updated after Re-INVITE"
        );
    }

    #[tokio::test]
    async fn test_session_timer_response_forwarding() {
        let raw_resp = concat!(
            "SIP/2.0 200 OK\r\n",
            "Via: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-vosrs\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:13800138000@example.com>;tag=gw-tag\r\n",
            "Call-ID: test-session-expires-forwarding@example.com\r\n",
            "CSeq: 1 INVITE\r\n",
            "Session-Expires: 600;refresher=uac\r\n",
            "Min-SE: 90\r\n",
            "Supported: timer\r\n",
            "Content-Length: 0\r\n\r\n"
        );

        let SipMessage::Response(resp) = parse_message(raw_resp.as_bytes()).unwrap() else {
            panic!("expected response");
        };

        let vias = vec!["SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-inbound".to_string()];
        let route_set = vec![];
        let forwarded =
            response::forward_response_to_inbound_with_body(&resp, &vias, &route_set, &[]);
        let forwarded_str = String::from_utf8(forwarded).unwrap();

        assert!(forwarded_str.contains("Session-Expires: 600;refresher=uac\r\n"));
        assert!(forwarded_str.contains("Min-SE: 90\r\n"));
        assert!(forwarded_str.contains("Supported: timer\r\n"));
    }

    #[tokio::test]
    async fn test_active_session_refresh_triggering() {
        let edge_state = Arc::new(state_with_default_route());
        let call_id = "test-active-refresh-trigger@example.com";

        // Setup a tracked established call
        let raw_invite = concat!(
            "INVITE sip:13800138000@example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-refresh-invite\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:13800138000@example.com>\r\n",
            "Call-ID: test-active-refresh-trigger@example.com\r\n",
            "CSeq: 1 INVITE\r\n",
            "Content-Length: 0\r\n\r\n"
        );

        // 1. Receive INVITE
        let _ = handle_datagram(raw_invite.as_bytes(), peer(), &edge_state, &edge_config()).await;

        // 2. Setup refresher = Some("uac") and last_session_refresh = 10 seconds ago
        {
            // DashMap get_mut returns RefMut
            let mut tx = edge_state.inbound_transactions.get_mut(call_id).unwrap();
            tx.session_expires = Some(10); // Expires in 10s, refresh at 5s
            tx.session_refresher = Some("uac".to_string());
            tx.last_session_refresh =
                Some(std::time::Instant::now() - std::time::Duration::from_secs(6));
            tx.callee_contact =
                Some(SipUri::from_str("sip:13800138000@gw-real-ip.com:5060").unwrap());
        }

        // 3. Setup a mock socket to capture outbound refresh UPDATE packet
        let tokio_socket = Arc::new(tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let port = tokio_socket.local_addr().unwrap().port();

        // Spawn watchdog with short interval
        let mut config = edge_config();
        config.advertised_addr = format!("127.0.0.1:{}", port);

        spawn_session_timer_watchdog(Arc::clone(&edge_state), Arc::clone(&tokio_socket), Arc::new(config));

        // Wait a bit for watchdog loop to tick
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;

        // Verify that the call's last_session_refresh was reset (throttled)
        {
            let tx = edge_state.inbound_transactions.get(call_id).unwrap();
            let elapsed = tx.last_session_refresh.unwrap().elapsed().as_secs();
            assert!(elapsed < 2);
        }
    }

    #[tokio::test]
    async fn test_self_refresh_response_drop() {
        let edge_state = state_with_default_route();
        let call_id = "test-self-refresh-response-drop@example.com";

        // Setup a tracked established call
        let raw_invite = concat!(
            "INVITE sip:13800138000@example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-drop-invite\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:13800138000@example.com>\r\n",
            "Call-ID: test-self-refresh-response-drop@example.com\r\n",
            "CSeq: 1 INVITE\r\n",
            "Content-Length: 0\r\n\r\n"
        );

        let _ = handle_datagram(raw_invite.as_bytes(), peer(), &edge_state, &edge_config()).await;

        // Setup mock session timer state
        {
            // DashMap get_mut returns RefMut
            let mut tx = edge_state.inbound_transactions.get_mut(call_id).unwrap();
            tx.session_expires = Some(600);
            tx.session_refresher = Some("uac".to_string());
            tx.last_session_refresh =
                Some(std::time::Instant::now() - std::time::Duration::from_secs(400));
        }

        // Send a 200 OK response corresponding to our self-generated refresh request (Via contains branch=z9hG4bK-refresh-)
        let raw_200 = format!(
            "SIP/2.0 200 OK\r\n\
             Via: SIP/2.0/UDP 127.0.0.1:5060;branch=z9hG4bK-refresh-true-2\r\n\
             From: <sip:1001@example.com>;tag=from-tag\r\n\
             To: <sip:13800138000@example.com>;tag=gw-tag\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 2 UPDATE\r\n\
             Content-Length: 0\r\n\r\n",
            call_id = call_id
        );
        let datagrams = handle_datagram(
            raw_200.as_bytes(),
            "203.0.113.20:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // The response must be consumed (empty datagram list returned)
        assert!(datagrams.is_empty());

        // The last_session_refresh must be reset to now (elapsed < 2 seconds)
        {
            let tx = edge_state.inbound_transactions.get(call_id).unwrap();
            let elapsed = tx.last_session_refresh.unwrap().elapsed().as_secs();
            assert!(elapsed < 2);
        }
    }


// ========================================== 
// FILE: prack_early_media_tests.rs
// ========================================== 
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

        assert_eq!(datagrams_200.len(), 2, "gateway ACK and caller 200 are required");
        let forwarded_200 = response_text(&datagrams_200, "SIP/2.0 200 OK");
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

        let call = edge_state
            .call_manager
            .get(&CallId::new(call_id))
            .expect("call should remain tracked after final 200 OK");
        assert_eq!(call.state, CallState::Established);
        assert!(call.answered_at.is_some());
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


// ========================================== 
// FILE: refer_transfer_tests.rs
// ========================================== 
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

    #[tokio::test]
    async fn test_refer_attended_transfer_lifecycle() {
        let edge_state = state_with_default_route();
        let call_id = "refer-attended-001";
        let offer_sdp = "v=0\r\no=- 0 0 IN IP4 192.0.2.10\r\ns=-\r\nc=IN IP4 192.0.2.10\r\nt=0 0\r\nm=audio 49170 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n";

        let invite = format!(
            "INVITE sip:13800138000@example.com SIP/2.0\r\nVia: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-inv-att\r\nMax-Forwards: 70\r\nFrom: <sip:1001@example.com>;tag=from-tag\r\nTo: <sip:13800138000@example.com>\r\nCall-ID: {call_id}\r\nCSeq: 1 INVITE\r\nContact: <sip:1001@192.0.2.10>\r\nContent-Type: application/sdp\r\nContent-Length: {len}\r\n\r\n{offer_sdp}",
            len = offer_sdp.len()
        );
        handle_datagram(
            invite.as_bytes(),
            "192.0.2.10:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        let answer_sdp = "v=0\r\no=- 0 0 IN IP4 198.51.100.20\r\ns=-\r\nc=IN IP4 198.51.100.20\r\nt=0 0\r\nm=audio 49200 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n";
        let ok_200 = format!(
            "SIP/2.0 200 OK\r\nVia: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-inv-att\r\nFrom: <sip:1001@example.com>;tag=from-tag\r\nTo: <sip:13800138000@example.com>;tag=gw-tag\r\nCall-ID: {call_id}\r\nCSeq: 1 INVITE\r\nContact: <sip:13800138000@gw1.example.com:5060>\r\nContent-Type: application/sdp\r\nContent-Length: {len}\r\n\r\n{answer_sdp}",
            len = answer_sdp.len()
        );
        handle_datagram(
            ok_200.as_bytes(),
            "203.0.113.20:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        let refer = format!(
            "REFER sip:edge.example.com SIP/2.0\r\n\
             Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-ref-att\r\n\
             Max-Forwards: 70\r\n\
             From: <sip:1001@example.com>;tag=from-tag\r\n\
             To: <sip:13800138000@example.com>;tag=gw-tag\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 2 REFER\r\n\
             Refer-To: <sip:1002@example.com?Replaces=replaced-call-id%3Bto-tag%3Dxyz%3Bfrom-tag%3Dabc>\r\n\
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
        assert!(invite_c.contains("Replaces: replaced-call-id;to-tag=xyz;from-tag=abc\r\n"));

        let transfer_call_id = invite_c
            .lines()
            .find(|l| l.starts_with("Call-ID:"))
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap()
            .to_string();

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

        assert_eq!(err_datagrams.len(), 1);
        let notify_err = datagram_text(&err_datagrams[0]);
        assert!(notify_err.contains("SIP/2.0 486 Busy Here\r\n"));

        let tx = edge_state.inbound_transactions.get(call_id).unwrap();
        let caller_relay = tx.caller_relay_rtp.as_ref().unwrap();
        let gw_relay = tx.gateway_relay_rtp.as_ref().unwrap();
        assert_eq!(
            edge_state.media_relay.target_for_port(caller_relay.port),
            Some("198.51.100.20:49200".parse().unwrap())
        );
        assert_eq!(
            edge_state.media_relay.target_for_port(gw_relay.port),
            Some("192.0.2.10:49170".parse().unwrap())
        );
    }


// ========================================== 
// FILE: security_auth_tests.rs
// ========================================== 
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

        // Replay attempt fails (returns ChallengeWithFailure due to replay block)
        assert_eq!(
            auth.verify_request(&request_correct, None, Some(&cache))
                .await,
            crate::AuthDecision::ChallengeWithFailure
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


// ========================================== 
// FILE: topology_hiding_tests.rs
// ========================================== 
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
        assert!(
            uuid::Uuid::parse_str(&external_call_id).is_ok(),
            "outbound Call-ID must be a UUID: {external_call_id}"
        );
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

        assert_eq!(
            resp_dg.len(),
            2,
            "gateway leg should be ACKed while 200 OK is forwarded to caller"
        );
        let gateway_ack = resp_dg
            .iter()
            .find(|datagram| datagram_text(datagram).starts_with("ACK "))
            .expect("gateway ACK not generated");
        let gateway_ack_text = datagram_text(gateway_ack);
        assert!(gateway_ack_text.contains(&format!("Call-ID: {external_call_id}\r\n")));
        let forwarded = resp_dg
            .iter()
            .map(datagram_text)
            .find(|text| text.starts_with("SIP/2.0 200 OK"))
            .expect("caller 200 OK not generated");

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


// ========================================== 
// FILE: transaction_retransmit_tests.rs
// ========================================== 
    #[tokio::test]
    async fn test_client_transaction_retransmission() {
        let edge_state = Arc::new(state_with_default_route());
        let socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let local_port = socket.local_addr().unwrap().port();
        let target = format!("127.0.0.1:{}", local_port);

        let req_bytes = b"INVITE sip:gw@127.0.0.1:5060 SIP/2.0\r\n\
                          Via: SIP/2.0/UDP 127.0.0.1:5060;branch=z9hG4bK-tx-test\r\n\
                          Call-ID: tx-test@example.com\r\n\
                          CSeq: 1 INVITE\r\n\
                          Content-Length: 0\r\n\r\n";
        let req = parse_message(req_bytes).unwrap();
        let SipMessage::Request(req) = req else {
            panic!("expected request");
        };
        let key = ClientTransactionKey::from_request(&req).unwrap();

        spawn_client_transaction_retransmission(
            Arc::clone(&edge_state),
            Arc::clone(&socket),
            target.clone(),
            req_bytes.to_vec(),
            key.clone(),
            Arc::new(edge_config()),
        );
        assert!(edge_state.client_transactions.contains_key(&key));

        let resp = parse_message(
            b"SIP/2.0 180 Ringing\r\n\
              Via: SIP/2.0/UDP 127.0.0.1:5060;branch=z9hG4bK-tx-test\r\n\
              Call-ID: tx-test@example.com\r\n\
              CSeq: 1 INVITE\r\n\
              Content-Length: 0\r\n\r\n",
        )
        .unwrap();
        let SipMessage::Response(resp) = resp else {
            panic!("expected response");
        };
        let resp_key = ClientTransactionKey::from_response(&resp).unwrap();

        edge_state.cancel_client_transaction(&resp_key);

        tokio::time::sleep(Duration::from_millis(5)).await;
        assert!(!edge_state
            .client_transactions
            .contains_key(&key));
    }

    #[tokio::test]
    async fn test_client_transaction_timeout_triggers_failover() {
        let routes = RouteTable::new(vec![
            Route::new(
                "primary",
                "".to_string(),
                100,
                RouteTarget::new("gw1".to_string(), "127.0.0.1".to_string(), Some(12345)),
            ),
            Route::new(
                "secondary",
                "".to_string(),
                200,
                RouteTarget::new("gw2".to_string(), "127.0.0.1".to_string(), Some(23456)),
            ),
        ]);
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let edge_state = Arc::new(EdgeState::new(CallManager::new(routes, tx)));
        let socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let call_id = "timeout-failover-test@example.com";

        let invite = format!(
            concat!(
                "INVITE sip:13800138000@example.com SIP/2.0\r\n",
                "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-timeout\r\n",
                "Max-Forwards: 70\r\n",
                "From: <sip:1001@example.com>;tag=from-tag\r\n",
                "To: <sip:13800138000@example.com>\r\n",
                "Call-ID: {call_id}\r\n",
                "CSeq: 1 INVITE\r\n",
                "Content-Length: 0\r\n",
                "\r\n"
            ),
            call_id = call_id
        );

        let datagrams =
            handle_datagram(invite.as_bytes(), peer(), &edge_state, &edge_config()).await;
        assert_eq!(datagrams.len(), 2);
        assert_eq!(datagrams[1].target, "127.0.0.1:23456");

        let outbound_invite = parse_message(&datagrams[1].bytes).unwrap();
        let SipMessage::Request(outbound_req) = outbound_invite else {
            panic!("expected request");
        };
        let key = ClientTransactionKey::from_request(&outbound_req).unwrap();

        spawn_client_transaction_retransmission(
            Arc::clone(&edge_state),
            Arc::clone(&socket),
            "127.0.0.1:23456".to_string(),
            datagrams[1].bytes.clone(),
            key,
            Arc::new(edge_config()),
        );

        let mut success = false;
        for _ in 0..15 {
            tokio::time::sleep(Duration::from_millis(20)).await;
            let call_guard = &edge_state.call_manager;
            if let Some(call) = call_guard.get(&CallId::new(call_id)) {
                if call.current_candidate_index == 1 {
                    success = true;
                    break;
                }
            }
        }
        assert!(success, "failed to trigger failover within timeout");

        let call_guard = &edge_state.call_manager;
        let call = call_guard.get(&CallId::new(call_id)).unwrap();
        assert_eq!(call.state, CallState::Routing);
        assert_eq!(
            call.outbound.as_ref().unwrap().remote_uri.to_string(),
            "sip:13800138000@127.0.0.1:12345;transport=udp"
        );
    }


// ========================================== 
// FILE: dtmf_cdr_tests.rs
// ========================================== 
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


// ========================================== 
// FILE: gateway_health_probe_tests.rs
// ========================================== 
#[tokio::test]
async fn gateway_options_response_updates_probe_health() {
    let (cdr_tx, _cdr_rx) = tokio::sync::mpsc::unbounded_channel();
    let call_manager = CallManager::new(
        RouteTable::new(vec![Route::new(
            "gw1-route",
            "",
            100,
            RouteTarget::new("gw1", "127.0.0.1", Some(5060)),
        )]),
        cdr_tx,
    );
    let edge_state = EdgeState::new(call_manager);
    let config = EdgeConfig::from_env();
    let call_id = "health-probe-gw1-test";
    edge_state
        .gateway_probes
        .insert(call_id.to_string(), "gw1".to_string());

    let response = format!(
        "SIP/2.0 200 OK\r\n\
         Via: SIP/2.0/UDP 127.0.0.1:5060;branch=z9hG4bK-health-1\r\n\
         From: <sip:health-check@127.0.0.1>;tag=health-1\r\n\
         To: <sip:health-check@127.0.0.1>;tag=gw\r\n\
         Call-ID: {call_id}\r\n\
         CSeq: 1 OPTIONS\r\n\
         Content-Length: 0\r\n\r\n"
    );

    let datagrams = handle_datagram(
        response.as_bytes(),
        "127.0.0.1:5060".parse().unwrap(),
        &edge_state,
        &config,
    )
    .await;

    assert!(datagrams.is_empty());
    assert!(!edge_state.gateway_probes.contains_key(call_id));
    let health = edge_state.gateway_health.lock().unwrap();
    let status = health.get_gateway_status("gw1");
    assert!(status.is_some());
    let (open, failures, _, _, _, _) = status.unwrap();
    assert!(!open);
    assert_eq!(failures, 0);
}


// ========================================== 
// FILE: realtime_billing_tests.rs
// ========================================== 
use tokio::time::sleep;

#[tokio::test]
async fn test_balance_exhaustion_disconnect_flow() {
    let edge_state = std::sync::Arc::new(state_with_default_route());
    
    // Bind mock socket for the test state
    let test_socket = std::sync::Arc::new(tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap());
    edge_state.set_socket(test_socket.clone());

    // 1. Send inbound INVITE from caller 1001 with test max duration limit = 1 second.
    let call_id = "billing-limit-call@example.com";
    let invite = format!(
        "INVITE sip:13800000000@edge.example.com SIP/2.0\r\n\
         Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-inv-bill\r\n\
         Max-Forwards: 70\r\n\
         From: <sip:1001@example.com>;tag=from-tag\r\n\
         To: <sip:13800000000@edge.example.com>\r\n\
         Call-ID: {call_id}\r\n\
         CSeq: 1 INVITE\r\n\
         X-Test-Max-Duration: 1\r\n\
         Content-Length: 0\r\n\r\n",
        call_id = call_id
    );

    let datagrams = handle_datagram(
        invite.as_bytes(),
        peer(),
        &edge_state,
        &edge_config(),
    )
    .await;

    // Should route outbound invite to gateway gw1
    assert!(!datagrams.is_empty());

    // Extract the dynamically generated external Call-ID
    let outbound_invite_txt = String::from_utf8_lossy(&datagrams[0].bytes);
    let external_call_id = outbound_invite_txt
        .lines()
        .find(|l| l.starts_with("Call-ID:"))
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .to_string();

    // 2. Send 200 OK back from gateway to answer the call
    let ok_200 = format!(
        "SIP/2.0 200 OK\r\n\
         Via: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-vosrs\r\n\
         From: <sip:1001@example.com>;tag=from-tag\r\n\
         To: <sip:13800000000@edge.example.com>;tag=gw-tag\r\n\
         Call-ID: {external_call_id}\r\n\
         CSeq: 1 INVITE\r\n\
         Content-Length: 0\r\n\r\n",
        external_call_id = external_call_id
    );

    let _ = handle_datagram(
        ok_200.as_bytes(),
        "198.51.100.20:5060".parse().unwrap(),
        &edge_state,
        &edge_config(),
    )
    .await;

    // Verify call is established and max_duration_secs is set to 1s
    {
        let tx = edge_state.inbound_transactions.get(call_id).unwrap();
        assert_eq!(tx.max_duration_secs, Some(1));
        assert!(tx.established_at.is_some());
    }

    // 3. Start the watchdog timer loop in background to check expiration (simulate elapse of 1s)
    let edge_config_arc = std::sync::Arc::new(edge_config());

    // Wait 1.1s for balance to exhaust
    sleep(Duration::from_millis(1100)).await;

    // Trigger one watchdog pass manually by invoking the logic or running the loop.
    spawn_session_timer_watchdog(edge_state.clone(), test_socket, edge_config_arc);

    // Wait a brief moment for the timer loop to execute its check pass
    sleep(Duration::from_millis(100)).await;

    // Verify transaction has been cleaned up due to balance disconnect
    assert!(!edge_state.inbound_transactions.contains_key(call_id));
}


// ========================================== 
// FILE: media_control_tests.rs
// ========================================== 
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use crate::media::relay::{MediaRelayState, PlaybackMode};

/// 生成一个标准的 8000Hz 16-bit Mono PCM 1秒钟的测试 WAV 文件
fn create_test_wav(path: &std::path::Path) {
    let mut file = File::create(path).unwrap();
    // 44字节标准的 WAV 文件头
    file.write_all(b"RIFF").unwrap();
    let file_size = 44_u32 + 16000_u32 - 8_u32;
    file.write_all(&file_size.to_le_bytes()).unwrap();
    file.write_all(b"WAVE").unwrap();
    file.write_all(b"fmt ").unwrap();
    file.write_all(&16_u32.to_le_bytes()).unwrap(); // Chunk size
    file.write_all(&1_u16.to_le_bytes()).unwrap();  // PCM format
    file.write_all(&1_u16.to_le_bytes()).unwrap();  // Channels: 1 (Mono)
    file.write_all(&8000_u32.to_le_bytes()).unwrap(); // Sample rate: 8000
    let byte_rate = 8000_u32 * 2_u32;
    file.write_all(&byte_rate.to_le_bytes()).unwrap();
    file.write_all(&2_u16.to_le_bytes()).unwrap(); // Block align
    file.write_all(&16_u16.to_le_bytes()).unwrap(); // Bits per sample
    file.write_all(b"data").unwrap();
    file.write_all(&16000_u32.to_le_bytes()).unwrap(); // Data segment size (8000 samples * 2 bytes)
    
    // 写入 8000 个静音采样点 (0)
    let pcm_data = vec![0_u8; 16000];
    file.write_all(&pcm_data).unwrap();
}

#[tokio::test]
async fn test_playback_and_mute_control_flow() {
    let wav_path = PathBuf::from("test_playback.wav");
    create_test_wav(&wav_path);

    let relay = MediaRelayState::new();
    let port = 45000;

    // 绑定本地 Socket
    let socket = UdpSocket::bind("127.0.0.1:45000").await.unwrap();
    let socket = Arc::new(socket);
    relay.active_sockets.insert(port, Arc::clone(&socket));

    // 设置对端路由接收地址（模拟主叫）
    let receiver = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let receiver_addr = receiver.local_addr().unwrap();
    relay.targets.insert(port, receiver_addr);

    // 1. 验证静音插入与移出
    assert!(!relay.muted_ports.contains(&port));
    relay.muted_ports.insert(port);
    assert!(relay.muted_ports.contains(&port));
    relay.muted_ports.remove(&port);
    assert!(!relay.muted_ports.contains(&port));

    // 2. 启动音频播放
    relay.start_playback(port, wav_path.clone(), PlaybackMode::Exclusive, true).unwrap();
    assert!(relay.playbacks.contains_key(&port));

    // 3. 验证接收播放的 RTP 数据包，并且验证首包 Marker Bit 为 true
    let mut buffer = [0_u8; 1500];
    let (len, from) = tokio::time::timeout(Duration::from_millis(500), receiver.recv_from(&mut buffer))
        .await
        .expect("接收 RTP 播放包超时")
        .expect("读取数据失败");

    assert!(len > 12); // RTP 头至少有 12 字节
    assert_eq!(from, socket.local_addr().unwrap());
    
    // 解析 RTP 包验证 Marker Bit
    let rtp_view = rtp_core::RtpPacketView::parse(&buffer[..len]).unwrap();
    assert!(rtp_view.marker); // 首包 Marker Bit 应为 true

    // 4. 停止播放
    relay.stop_playback(port);
    assert!(!relay.playbacks.contains_key(&port));

    // 清理测试文件
    let _ = std::fs::remove_file(wav_path);
}

#[tokio::test]
async fn test_smooth_sequence_and_timestamp_transition() {
    let relay = MediaRelayState::new();
    let local_port = 45010;
    let peer_port = 45012;

    relay.peer_ports.insert(local_port, peer_port);
    relay.peer_ports.insert(peer_port, local_port);

    // 1. 记录初始流发送
    relay.seed_continuity_for_test(local_port, 100, 1000);

    // 2. 模拟 Exclusive 播放启动与停止（表示发生了 Exclusive 拦截）
    // 此时 was_in_exclusive 会被标记为 true
    relay.resume_continuity_for_test(local_port);

    // 3. 模拟在停止 Exclusive 播放后，收到原音频发送端发来的下一个非连续包（如 seq=105, ts=2000）
    let incoming_rtp = rtp_core::RtpPacket {
        marker: false,
        payload_type: 8,
        sequence_number: 105,
        timestamp: 2000,
        ssrc: 12345,
        csrcs: Vec::new(),
        extension: None,
        payload: vec![0; 160],
        padding_len: 0,
    };
    let encoded = incoming_rtp.encode().unwrap();

    // 4. 使用生产代码的连续性计算，避免测试复刻内部数据结构。
    let (sequence_offset, timestamp_offset) =
        relay.continuity_offsets_for_test(local_port, 105, 2000);
    assert_eq!(sequence_offset, 105 - 101);
    assert_eq!(timestamp_offset, 2000 - 1160);

    let mut rewritten = rtp_core::RtpPacket::parse(&encoded).unwrap();
    rewritten.sequence_number = rewritten.sequence_number.wrapping_sub(sequence_offset);
    rewritten.timestamp = rewritten.timestamp.wrapping_sub(timestamp_offset);
    let parsed_rewritten = rtp_core::RtpPacket::parse(&rewritten.encode().unwrap()).unwrap();
    // 验证修改后的包序列号与时间戳是完全连续的（接上 100 和 1000 + 160）
    assert_eq!(parsed_rewritten.sequence_number, 101);
    assert_eq!(parsed_rewritten.timestamp, 1160);
}

#[tokio::test]
async fn test_audio_resampling_from_higher_rate() {
    let wav_path = PathBuf::from("test_resample_16k.wav");
    
    // 生成一个 16000Hz (16kHz) 16-bit Mono LPCM WAV 文件，包含 16000 个采样（即 1秒钟长度）
    let mut file = File::create(&wav_path).unwrap();
    file.write_all(b"RIFF").unwrap();
    let file_size = 44_u32 + 32000_u32 - 8_u32;
    file.write_all(&file_size.to_le_bytes()).unwrap();
    file.write_all(b"WAVE").unwrap();
    file.write_all(b"fmt ").unwrap();
    file.write_all(&16_u32.to_le_bytes()).unwrap();
    file.write_all(&1_u16.to_le_bytes()).unwrap(); // LPCM
    file.write_all(&1_u16.to_le_bytes()).unwrap(); // Mono
    file.write_all(&16000_u32.to_le_bytes()).unwrap(); // Sample rate: 16000 Hz
    let byte_rate = 16000_u32 * 2_u32;
    file.write_all(&byte_rate.to_le_bytes()).unwrap();
    file.write_all(&2_u16.to_le_bytes()).unwrap();
    file.write_all(&16_u16.to_le_bytes()).unwrap();
    file.write_all(b"data").unwrap();
    file.write_all(&32000_u32.to_le_bytes()).unwrap(); // 16000 samples * 2 bytes
    
    let pcm_data = vec![0_u8; 32000];
    file.write_all(&pcm_data).unwrap();
    drop(file);

    // 载入 WAV 并自动进行重采样
    let samples = crate::media::wav::load_wav_pcm(&wav_path).unwrap();
    
    // 验证经过重采样（16000Hz -> 8000Hz）后，采样点个数正好减半为 8000 个左右
    assert_eq!(samples.len(), 8000);

    // 清理测试文件
    let _ = std::fs::remove_file(wav_path);
}


// ========================================== 
// FILE: connection_check_tests.rs
// ========================================== 
#[tokio::test]
async fn test_database_connection() {
    let config = EdgeConfig::load();
    if let Some(ref db_url) = config.database_url {
        let connect_result = cdr_core::PostgresCdrStore::connect(db_url, config.database_max_connections).await;
        assert!(connect_result.is_ok(), "Database connection test failed: {:?}", connect_result.err());
    } else {
        panic!("Database URL is not configured in config.yaml");
    }
}

#[tokio::test]
async fn test_redis_connection() {
    let config = EdgeConfig::load();
    if let Some(ref redis_url) = config.redis_url {
        let redis_client = redis::Client::open(redis_url.clone());
        assert!(redis_client.is_ok(), "Failed to open Redis client");
        let client = redis_client.unwrap();
        let conn = client.get_multiplexed_tokio_connection().await;
        assert!(conn.is_ok(), "Failed to establish Redis multiplexed connection: {:?}", conn.err());
    } else {
        panic!("Redis URL is not configured in config.yaml");
    }
}

#[tokio::test]
async fn test_nats_connection() {
    let config = EdgeConfig::load();
    if let Some(ref nats_url) = config.nats_url {
        let nats_result = async_nats::connect(nats_url).await;
        assert!(nats_result.is_ok(), "NATS connection test failed: {:?}", nats_result.err());
    }
}

#[tokio::test]
async fn test_s3_storage_connection() {
    let storage_config = storage_core::StorageConfig::from_env();
    let create_result = storage_core::create_storage(&storage_config).await;
    assert!(create_result.is_ok(), "S3/Storage backend creation test failed: {:?}", create_result.err());
}


// ========================================== 
// FILE: conference_tests.rs
// ========================================== 
#[tokio::test]
async fn test_sip_invite_join_and_leave_conference() {
    let edge_state = state_without_routes();
    let edge_config = edge_config();

    // 1. 发送 INVITE 拨打会议室 conf_room1
    let invite = concat!(
        "INVITE sip:conf_room1@example.com SIP/2.0\r\n",
        "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-invite-conf\r\n",
        "Max-Forwards: 70\r\n",
        "From: <sip:1001@example.com>;tag=from-tag\r\n",
        "To: <sip:conf_room1@example.com>\r\n",
        "Call-ID: invite-conf-1@example.com\r\n",
        "CSeq: 1 INVITE\r\n",
        "Contact: <sip:1001@192.0.2.10:5060>\r\n",
        "Content-Type: application/sdp\r\n",
        "Content-Length: 125\r\n",
        "\r\n",
        "v=0\r\n",
        "o=alice 2890844526 2890844526 IN IP4 192.0.2.10\r\n",
        "s=-\r\n",
        "c=IN IP4 192.0.2.10\r\n",
        "t=0 0\r\n",
        "m=audio 49170 RTP/AVP 8\r\n",
        "a=rtpmap:8 PCMA/8000\r\n"
    );

    let datagrams = handle_datagram(invite.as_bytes(), peer(), &edge_state, &edge_config).await;
    assert_eq!(datagrams.len(), 1);
    
    let response = datagram_text(&datagrams[0]);
    assert!(response.starts_with("SIP/2.0 200 OK\r\n"));
    assert!(response.contains("Content-Type: application/sdp\r\n"));
    assert!(response.contains("s=vos-rs-conference\r\n"));

    // 验证参会成员已被加入会议
    let conf_exists = edge_state.media_relay.conference_manager.conferences.contains_key("conf_room1");
    assert!(conf_exists);

    // 获得分配的媒体端口
    let mut local_port = 0;
    if let Some(entry) = edge_state.media_relay.conference_manager.conferences.get("conf_room1") {
        let conf = entry.value().lock().await;
        assert_eq!(conf.participants.len(), 1);
        local_port = *conf.participants.keys().next().unwrap();
    }
    assert!(local_port > 0);

    // 提取 To 标签以构建正确的 BYE 报文
    let to_tag = {
        let to_line = response
            .lines()
            .find(|l| l.to_lowercase().starts_with("to:"))
            .expect("To header line not found");
        let tag_idx = to_line.find(";tag=").expect("To tag not found in 200 OK");
        let rest = &to_line[tag_idx + 5..];
        let mut tag = rest.trim().to_string();
        if let Some(semi) = tag.find(';') {
            tag.truncate(semi);
        }
        tag
    };


    // 2. 发送 BYE 挂断电话，退出会议
    let bye = format!(
        "BYE sip:conf_room1@example.com SIP/2.0\r\n\
         Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-bye-conf\r\n\
         Max-Forwards: 70\r\n\
         From: <sip:1001@example.com>;tag=from-tag\r\n\
         To: <sip:conf_room1@example.com>;tag={}\r\n\
         Call-ID: invite-conf-1@example.com\r\n\
         CSeq: 2 BYE\r\n\
         Content-Length: 0\r\n\
         \r\n",
        to_tag
    );

    let datagrams_bye = handle_datagram(bye.as_bytes(), peer(), &edge_state, &edge_config).await;
    assert_eq!(datagrams_bye.len(), 1);
    let response_bye = datagram_text(&datagrams_bye[0]);
    assert!(
        response_bye.starts_with("SIP/2.0 200 OK\r\n"),
        "BYE failed, response was: {}",
        response_bye
    );


    // 给后台的 leave_conference 任务一些执行时间
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // 验证参会成员已被安全移除，且会议室已被自动销毁/清理
    let conf_exists_after = edge_state.media_relay.conference_manager.conferences.contains_key("conf_room1");

    assert!(!conf_exists_after);
}

#[tokio::test]
async fn test_conference_participant_muting() {
    let conference_id = "conf_mute_test";
    let conf_manager = Arc::new(crate::media::conference::ConferenceManager::new());
    
    let port1 = 41002;
    let target1: std::net::SocketAddr = "127.0.0.1:50002".parse().unwrap();
    let socket1 = Arc::new(tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap());
    
    let port2 = 41004;
    let target2: std::net::SocketAddr = "127.0.0.1:50004".parse().unwrap();
    let socket2 = Arc::new(tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap());

    // Join participants
    conf_manager.join_conference(conference_id, port1, rtp_core::AudioCodec::Pcma, target1, socket1).await;
    conf_manager.join_conference(conference_id, port2, rtp_core::AudioCodec::Pcma, target2, socket2).await;

    // Send audio packet for participant 1
    let payload = vec![0x55; 160];
    conf_manager.handle_rtp_packet(port1, &payload, rtp_core::AudioCodec::Pcma);

    // Mute participant 1
    let success = conf_manager.set_participant_mute(conference_id, port1, true).await;
    assert!(success);

    // After muting, the buffer should be processed as silence
    let is_muted = {
        if let Some(entry) = conf_manager.conferences.get(conference_id) {
            let conf = entry.value().lock().await;
            conf.participants.get(&port1).unwrap().muted
        } else {
            false
        }
    };
    assert!(is_muted);
}

#[tokio::test]
async fn test_call_monitoring() {
    use crate::media::relay::MediaRelayState;
    use std::net::UdpSocket;
    use std::time::Duration;

    let relay = MediaRelayState::new();
    let port = 42002;
    
    // Bind a socket to mock the supervisor receiver
    let supervisor_socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let supervisor_addr = supervisor_socket.local_addr().unwrap();
    
    // Bind socket for local port
    let local_socket = UdpSocket::bind(("127.0.0.1", port)).unwrap();
    local_socket.set_nonblocking(true).unwrap();
    let socket_arc = Arc::new(tokio::net::UdpSocket::from_std(local_socket).unwrap());
    
    relay.active_sockets.insert(port, socket_arc.clone());
    
    // Start monitoring
    relay.start_monitoring(port, supervisor_addr);
    
    // Set target for the port
    let dummy_target: SocketAddr = "127.0.0.1:23456".parse().unwrap();
    relay.targets.insert(port, dummy_target);
    relay.peer_ports.insert(port, port + 1);
    relay.codecs.insert(port, rtp_core::AudioCodec::Pcma);
    
    // Spawn the media port relay loop in background
    let (tx, rx) = tokio::sync::oneshot::channel();
    let relay_clone = relay.clone();
    let socket_clone = socket_arc.clone();
    tokio::spawn(async move {
        crate::media::relay::relay_media_port(
            socket_clone,
            port,
            relay_clone,
            false,
            false,
            60,
            crate::media::rtcp_processor::MediaPacketKind::Rtp,
            rx,
        ).await;
    });

    // Send a mock RTP packet to our local port
    let client_sender = UdpSocket::bind("127.0.0.1:0").unwrap();
    let rtp = rtp_core::RtpPacket::new(8, 1, 160, 999, vec![0x55; 10]).unwrap();
    let rtp_payload = rtp.encode().unwrap();
    
    client_sender.send_to(&rtp_payload, format!("127.0.0.1:{}", port)).unwrap();

    // The media loop will duplicate and forward it to supervisor_addr
    let mut receive_buf = vec![0_u8; 1024];
    let read_future = supervisor_socket.recv_from(&mut receive_buf);
    let Ok(Ok((received_size, _from_addr))) = tokio::time::timeout(Duration::from_millis(1000), read_future).await else {
        panic!("Timed out waiting for supervisor to receive monitored RTP packet");
    };
    
    assert!(received_size > 0);
    assert_eq!(&receive_buf[..received_size], &rtp_payload);
    
    // Stop monitoring
    relay.stop_monitoring(port, supervisor_addr);
    
    // Clean up
    let _ = tx.send(());
}
