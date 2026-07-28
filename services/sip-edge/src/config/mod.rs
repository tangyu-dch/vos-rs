use crate::cluster::{ClusterConfig, ClusterConfigError, MediaClusterConfig};
use crate::media;
use crate::sip::AuthConfig;

mod default;
mod dynamic_override;
mod loader;
mod sections;
mod webhook;

pub use webhook::WebhookConfig;

pub const DEFAULT_UDP_BUFFER_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EdgeConfig {
    pub sip_udp_bind: String,
    pub advertised_addr: String,
    pub default_gateway: String,
    pub database_routes_enabled: bool,
    pub gateway_health_checks_enabled: bool,
    pub manage_bind: String,
    pub stun_server: Option<String>,
    pub upnp_enabled: bool,
    pub database_url: Option<String>,
    pub database_max_connections: u32,
    pub redis_max_connections: u32,
    pub nats_url: Option<String>,
    pub nats_cdr_stream: Option<String>,
    pub nats_cdr_subject: Option<String>,
    pub redis_url: Option<String>,
    pub cluster: ClusterConfig,
    pub media_cluster: MediaClusterConfig,
    pub media: media::MediaConfig,
    pub auth: AuthConfig,
    pub session_expires_gateway: u32,
    pub session_expires_caller: u32,
    pub sbc_allow_rules: Vec<String>,
    pub sbc_block_rules: Vec<String>,
    pub sbc_rate_limit_enabled: bool,
    pub sbc_rate_limit_capacity: f64,
    pub sbc_rate_limit_fill_rate: f64,
    pub sbc_max_concurrency: u32,
    pub tls_cert_path: Option<String>,
    pub tls_key_path: Option<String>,
    pub tls_bind_addr: Option<String>,
    pub tls_allow_test_certificate: bool,
    pub tls_ca_path: Option<String>,
    pub tls_insecure_skip_verify: bool,
    pub tls_server_name: Option<String>,
    pub udp_workers: usize,
    pub udp_workers_auto: bool,
    pub udp_receive_buffer_bytes: usize,
    pub udp_send_buffer_bytes: usize,
    /// DSCP/TOS 值用于 SIP UDP 信令包标记（0 表示不设置，46=EF, 34=AF41）
    pub sip_dscp: u8,
    /// DSCP/TOS 值用于 RTP 媒体包标记（0 表示不设置，46=EF Expedited Forwarding）
    pub rtp_dscp: u8,
    pub ws_bind_addr: Option<String>,
    /// WebSocket Secure (WSS) SIP 信令监听地址，例如 "0.0.0.0:5061"。
    /// 配置后 sip-edge 会启动 TLS 加密的 WebSocket SIP 信令入站监听器，
    /// 用于浏览器 WebRTC 客户端接入。
    pub wss_bind_addr: Option<String>,
    pub internal_secret: String,
    pub bootstrap_auth_users: Option<String>,
    pub cdr_queue_capacity: usize,
    pub cdr_persistence_enabled: bool,
    pub recording_workers: usize,
    pub recording_queue_capacity: usize,
    pub media_metrics_log: bool,
    pub dynamic_config_enabled: bool,
    pub balance_enforcement_enabled: bool,
    pub billing_settlement_enabled: bool,
    pub webhooks: WebhookConfig,
    pub sipflow_enabled: bool,
    pub sipflow_whitelist: String,
    pub sipflow_retention_days: i32,
    /// SIP 客户端事务超时秒数（RFC 3261 Timer B 默认 32s）。
    /// 在高 CPS 场景下可适当缩短以快速触发故障转移。
    pub sip_transaction_timeout_secs: u64,
    /// SIP 事务 T1 重传初始间隔毫秒（RFC 3261 默认 500ms）。
    pub sip_t1_initial_ms: u64,
}

impl EdgeConfig {
    pub fn from_env() -> Self {
        Self::load()
    }

    pub fn load() -> Self {
        let config_file_path = loader::find_config_file();
        Self::load_from_file(config_file_path)
    }

    /// 在启动网络监听前校验集群拓扑。
    pub fn validate_cluster(&self) -> Result<(), ClusterConfigError> {
        self.cluster.validate(
            self.redis_url.as_deref(),
            self.nats_url.as_deref(),
            &self.media_cluster,
        )
    }
}
