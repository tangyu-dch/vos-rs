use super::webhook::WebhookSection;

#[derive(serde::Deserialize, Debug, Default)]
pub(super) struct MainFileConfig {
    pub(super) connections: Option<ConnectionsSection>,
    pub(super) sip_edge: Option<SipEdgeConfigSection>,
}

#[derive(serde::Deserialize, Debug, Default)]
pub(super) struct ConnectionsSection {
    pub(super) database: Option<DatabaseSection>,
    pub(super) redis: Option<RedisSection>,
    pub(super) nats: Option<NatsSection>,
}

#[derive(serde::Deserialize, Debug, Default)]
pub(super) struct RedisSection {
    pub(super) host: Option<String>,
    pub(super) port: Option<u16>,
    pub(super) password: Option<String>,
    pub(super) database: Option<u16>,
    pub(super) max_connections: Option<u32>,
}

#[derive(serde::Deserialize, Debug, Default)]
pub(super) struct DatabaseSection {
    pub(super) host: Option<String>,
    pub(super) port: Option<u16>,
    pub(super) username: Option<String>,
    pub(super) password: Option<String>,
    pub(super) database: Option<String>,
    pub(super) max_connections: Option<u32>,
}

#[derive(serde::Deserialize, Debug, Default)]
pub(super) struct NatsSection {
    pub(super) url: Option<String>,
    pub(super) cdr_stream: Option<String>,
    pub(super) cdr_subject: Option<String>,
}

#[derive(serde::Deserialize, Debug, Default)]
pub(super) struct SipEdgeConfigSection {
    pub(super) cluster: Option<crate::cluster::ClusterConfig>,
    pub(super) network: Option<NetworkSection>,
    pub(super) routing: Option<RoutingSection>,
    pub(super) nat_traversal: Option<NatTraversalSection>,
    pub(super) media: Option<MediaSection>,
    pub(super) recording: Option<RecordingSection>,
    pub(super) auth: Option<AuthSection>,
    pub(super) security: Option<SecuritySection>,
    pub(super) performance: Option<PerformanceSection>,
    pub(super) dynamic_config: Option<DynamicConfigSection>,
    pub(super) billing: Option<BillingSection>,
    pub(super) webhooks: Option<WebhookSection>,
}

#[derive(serde::Deserialize, Debug, Default)]
pub(super) struct NetworkSection {
    pub(super) sip_udp_bind: Option<String>,
    pub(super) advertised_addr: Option<String>,
    pub(super) manage_bind: Option<String>,
    pub(super) ws_bind: Option<String>,
    pub(super) wss_bind: Option<String>,
    pub(super) tls_cert_path: Option<String>,
    pub(super) tls_key_path: Option<String>,
    pub(super) tls_ca_path: Option<String>,
    pub(super) tls_server_name: Option<String>,
    pub(super) tls_allow_test_certificate: Option<bool>,
    pub(super) tls_insecure_skip_verify: Option<bool>,
}

#[derive(serde::Deserialize, Debug, Default)]
pub(super) struct MediaSection {
    pub(super) symmetric_learning: Option<bool>,
    pub(super) anti_spoofing: Option<bool>,
    pub(super) source_relearn_secs: Option<u64>,
    pub(super) metrics_log: Option<bool>,
    pub(super) allocation_strategy: Option<crate::cluster::MediaAllocationStrategy>,
    pub(super) health_check_interval_secs: Option<u64>,
    pub(super) unhealthy_threshold: Option<u32>,
    pub(super) nodes: Option<Vec<crate::cluster::MediaNodeConfig>>,
}

#[derive(serde::Deserialize, Debug, Default)]
pub(super) struct RecordingSection {
    pub(super) enabled: Option<bool>,
    pub(super) directory: Option<String>,
    pub(super) retention_secs: Option<u64>,
    pub(super) min_free_bytes: Option<u64>,
    pub(super) max_file_bytes: Option<u64>,
    pub(super) max_duration_secs: Option<u64>,
    pub(super) workers: Option<usize>,
    pub(super) queue_capacity: Option<usize>,
}

#[derive(serde::Deserialize, Debug, Default)]
pub(super) struct AuthSection {
    pub(super) enabled: Option<bool>,
    pub(super) users: Option<String>,
    pub(super) realm: Option<String>,
    pub(super) nonce: Option<String>,
    pub(super) secret_key: Option<String>,
}

#[derive(serde::Deserialize, Debug, Default)]
pub(super) struct SecuritySection {
    pub(super) internal_secret: Option<String>,
    pub(super) sbc_rate_limit_enabled: Option<bool>,
    pub(super) sbc_rate_limit_capacity: Option<f64>,
    pub(super) sbc_rate_limit_fill_rate: Option<f64>,
    pub(super) sbc_max_concurrency: Option<u32>,
}

#[derive(serde::Deserialize, Debug, Default)]
pub(super) struct PerformanceSection {
    pub(super) cdr_queue_capacity: Option<usize>,
    pub(super) cdr_persistence_enabled: Option<bool>,
    pub(super) udp_workers: Option<usize>,
    pub(super) udp_workers_auto: Option<bool>,
    pub(super) udp_receive_buffer_bytes: Option<usize>,
    pub(super) udp_send_buffer_bytes: Option<usize>,
    pub(super) sip_dscp: Option<u8>,
    pub(super) rtp_dscp: Option<u8>,
    pub(super) sip_transaction_timeout_secs: Option<u64>,
    pub(super) sip_t1_initial_ms: Option<u64>,
}

#[derive(serde::Deserialize, Debug, Default)]
pub(super) struct DynamicConfigSection {
    pub(super) enabled: Option<bool>,
}

#[derive(serde::Deserialize, Debug, Default)]
pub(super) struct BillingSection {
    pub(super) balance_enforcement_enabled: Option<bool>,
    pub(super) settlement_enabled: Option<bool>,
}

#[derive(serde::Deserialize, Debug, Default)]
pub(super) struct RoutingSection {
    pub(super) default_gateway: Option<String>,
    pub(super) database_routes_enabled: Option<bool>,
    pub(super) gateway_health_checks_enabled: Option<bool>,
}

#[derive(serde::Deserialize, Debug, Default)]
pub(super) struct NatTraversalSection {
    pub(super) stun_server: Option<String>,
    pub(super) upnp_enabled: Option<bool>,
}
