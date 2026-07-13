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

    static PORT_COUNTER: AtomicU16 = AtomicU16::new(0);

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
            media: MediaConfig::new("203.0.113.10", port_min, port_max),
            auth: AuthConfig::disabled(),
            session_expires_gateway: 600,
            session_expires_caller: 1800,
            sbc_allow_rules: std::env::var("VOS_RS_SBC_ALLOW")
                .unwrap_or_default()
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
            sbc_block_rules: std::env::var("VOS_RS_SBC_BLOCK")
                .unwrap_or_default()
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
            sbc_rate_limit_capacity: std::env::var("VOS_RS_SBC_LIMIT_CAPACITY")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(100.0),
            sbc_rate_limit_fill_rate: std::env::var("VOS_RS_SBC_LIMIT_FILL_RATE")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(10.0),
            sbc_max_concurrency: std::env::var("VOS_RS_SBC_MAX_CONCURRENCY")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(10),
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
        }
    }

    fn peer() -> SocketAddr {
        "192.0.2.10:5060".parse().unwrap()
    }

    fn datagram_text(datagram: &PendingDatagram) -> String {
        String::from_utf8(datagram.bytes.clone()).expect("datagram should be UTF-8")
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
        assert_eq!(datagrams.len(), 1);
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
