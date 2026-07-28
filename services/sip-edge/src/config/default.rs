use crate::cluster::{ClusterConfig, MediaClusterConfig};
use crate::media;
use crate::sip::AuthConfig;

use super::{EdgeConfig, WebhookConfig, DEFAULT_UDP_BUFFER_BYTES};

impl Default for EdgeConfig {
    fn default() -> Self {
        let media_cluster = MediaClusterConfig {
            nodes: vec![crate::cluster::MediaNodeConfig {
                id: "local-media".to_string(),
                node_type: crate::cluster::MediaNodeType::Local,
                control_url: None,
                advertised_addr: "127.0.0.1".to_string(),
                port_min: 40_000,
                port_max: 40_100,
                weight: 1,
                control_token: String::new(),
            }],
            ..MediaClusterConfig::default()
        };
        Self {
            sip_udp_bind: "0.0.0.0:5060".to_string(),
            advertised_addr: "127.0.0.1:5060".to_string(),
            default_gateway: String::new(),
            database_routes_enabled: true,
            gateway_health_checks_enabled: true,
            manage_bind: "127.0.0.1:8082".to_string(),
            stun_server: None,
            upnp_enabled: false,
            database_url: None,
            database_max_connections: 10,
            redis_max_connections: 10,
            nats_url: None,
            nats_cdr_stream: None,
            nats_cdr_subject: None,
            redis_url: None,
            cluster: ClusterConfig::default(),
            media_cluster,
            media: media::MediaConfig::new_with_symmetric_learning("127.0.0.1", 40000, 40100, true),
            auth: AuthConfig::disabled(),
            session_expires_gateway: 600,
            session_expires_caller: 1800,
            sbc_allow_rules: Vec::new(),
            sbc_block_rules: Vec::new(),
            sbc_rate_limit_enabled: true,
            sbc_rate_limit_capacity: 2000.0,
            sbc_rate_limit_fill_rate: 500.0,
            sbc_max_concurrency: 2000,
            tls_cert_path: None,
            tls_key_path: None,
            tls_bind_addr: None,
            tls_allow_test_certificate: false,
            tls_ca_path: None,
            tls_insecure_skip_verify: false,
            tls_server_name: None,
            udp_workers: 1,
            udp_workers_auto: false,
            udp_receive_buffer_bytes: DEFAULT_UDP_BUFFER_BYTES,
            udp_send_buffer_bytes: DEFAULT_UDP_BUFFER_BYTES,
            sip_dscp: 0,
            rtp_dscp: 0,
            ws_bind_addr: None,
            wss_bind_addr: None,
            internal_secret: "internal-dev-secret".to_string(),
            bootstrap_auth_users: None,
            cdr_queue_capacity: 4096,
            cdr_persistence_enabled: true,
            recording_workers: 4,
            recording_queue_capacity: 10_000,
            media_metrics_log: false,
            dynamic_config_enabled: true,
            balance_enforcement_enabled: true,
            billing_settlement_enabled: true,
            webhooks: WebhookConfig::default(),
            sipflow_enabled: true,
            sipflow_whitelist: "1001,1002".to_string(),
            sipflow_retention_days: 7,
            sip_transaction_timeout_secs: 32,
            sip_t1_initial_ms: 500,
        }
    }
}
