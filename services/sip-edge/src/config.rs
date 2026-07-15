use std::env;
use std::fs;
use std::path::Path;

use crate::cluster::{ClusterConfig, ClusterConfigError, MediaClusterConfig};
use crate::media;
use crate::sip::AuthConfig;

pub const DEFAULT_UDP_BUFFER_BYTES: usize = 4 * 1024 * 1024;

/// Webhook 异步投递配置。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct WebhookConfig {
    /// 是否启动 Webhook 事件流水线。
    pub enabled: bool,
    /// 接收呼叫事件的 HTTP 地址。
    pub endpoint_url: String,
    /// HMAC-SHA256 签名密钥。
    pub signing_secret: String,
    /// JetStream 名称。
    pub stream: String,
    /// NATS 事件主题。
    pub subject: String,
    /// JetStream Durable Consumer 名称。
    pub consumer: String,
    /// SIP 热路径到发布器的有界队列容量。
    pub queue_capacity: usize,
    /// 单次 HTTP 请求超时毫秒数。
    pub request_timeout_ms: u64,
    /// 单事件最大 HTTP 投递次数。
    pub max_deliveries: u32,
    /// 首次重试等待毫秒数，后续按指数增长。
    pub retry_delay_ms: u64,
    /// Redis 投递记录保留秒数。
    pub delivery_record_ttl_secs: u64,
}

impl Default for WebhookConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            endpoint_url: String::new(),
            signing_secret: String::new(),
            stream: "VOS_RS_WEBHOOKS".to_string(),
            subject: "vos_rs.webhooks.calls".to_string(),
            consumer: "vos_rs_webhook_delivery".to_string(),
            queue_capacity: 4096,
            request_timeout_ms: 3000,
            max_deliveries: 5,
            retry_delay_ms: 1000,
            delivery_record_ttl_secs: 604_800,
        }
    }
}

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
    pub ws_bind_addr: Option<String>,
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
}

#[derive(serde::Deserialize, Debug, Default)]
struct MainFileConfig {
    connections: Option<ConnectionsSection>,
    sip_edge: Option<SipEdgeConfigSection>,
}

#[derive(serde::Deserialize, Debug, Default)]
struct ConnectionsSection {
    database: Option<DatabaseSection>,
    redis: Option<RedisSection>,
    nats: Option<NatsSection>,
}

#[derive(serde::Deserialize, Debug, Default)]
struct RedisSection {
    host: Option<String>,
    port: Option<u16>,
    password: Option<String>,
    database: Option<u16>,
    max_connections: Option<u32>,
}

#[derive(serde::Deserialize, Debug, Default)]
struct DatabaseSection {
    host: Option<String>,
    port: Option<u16>,
    username: Option<String>,
    password: Option<String>,
    database: Option<String>,
    max_connections: Option<u32>,
}

#[derive(serde::Deserialize, Debug, Default)]
struct NatsSection {
    url: Option<String>,
    cdr_stream: Option<String>,
    cdr_subject: Option<String>,
}

#[derive(serde::Deserialize, Debug, Default)]
struct SipEdgeConfigSection {
    cluster: Option<ClusterConfig>,
    network: Option<NetworkSection>,
    routing: Option<RoutingSection>,
    nat_traversal: Option<NatTraversalSection>,
    media: Option<MediaSection>,
    recording: Option<RecordingSection>,
    auth: Option<AuthSection>,
    security: Option<SecuritySection>,
    performance: Option<PerformanceSection>,
    dynamic_config: Option<DynamicConfigSection>,
    billing: Option<BillingSection>,
    webhooks: Option<WebhookSection>,
}

#[derive(serde::Deserialize, Debug, Default)]
struct WebhookSection {
    enabled: Option<bool>,
    endpoint_url: Option<String>,
    signing_secret: Option<String>,
    stream: Option<String>,
    subject: Option<String>,
    consumer: Option<String>,
    queue_capacity: Option<usize>,
    request_timeout_ms: Option<u64>,
    max_deliveries: Option<u32>,
    retry_delay_ms: Option<u64>,
    delivery_record_ttl_secs: Option<u64>,
}

impl WebhookSection {
    fn into_config(self) -> WebhookConfig {
        let defaults = WebhookConfig::default();
        WebhookConfig {
            enabled: self.enabled.unwrap_or(defaults.enabled),
            endpoint_url: self.endpoint_url.unwrap_or(defaults.endpoint_url),
            signing_secret: self.signing_secret.unwrap_or(defaults.signing_secret),
            stream: self.stream.unwrap_or(defaults.stream),
            subject: self.subject.unwrap_or(defaults.subject),
            consumer: self.consumer.unwrap_or(defaults.consumer),
            queue_capacity: self.queue_capacity.unwrap_or(defaults.queue_capacity),
            request_timeout_ms: self
                .request_timeout_ms
                .unwrap_or(defaults.request_timeout_ms),
            max_deliveries: self.max_deliveries.unwrap_or(defaults.max_deliveries),
            retry_delay_ms: self.retry_delay_ms.unwrap_or(defaults.retry_delay_ms),
            delivery_record_ttl_secs: self
                .delivery_record_ttl_secs
                .unwrap_or(defaults.delivery_record_ttl_secs),
        }
    }
}

#[derive(serde::Deserialize, Debug, Default)]
struct NetworkSection {
    sip_udp_bind: Option<String>,
    advertised_addr: Option<String>,
    manage_bind: Option<String>,
    ws_bind: Option<String>,
}

#[derive(serde::Deserialize, Debug, Default)]
struct MediaSection {
    symmetric_learning: Option<bool>,
    anti_spoofing: Option<bool>,
    source_relearn_secs: Option<u64>,
    metrics_log: Option<bool>,
    allocation_strategy: Option<crate::cluster::MediaAllocationStrategy>,
    health_check_interval_secs: Option<u64>,
    unhealthy_threshold: Option<u32>,
    nodes: Option<Vec<crate::cluster::MediaNodeConfig>>,
}

#[derive(serde::Deserialize, Debug, Default)]
struct RecordingSection {
    enabled: Option<bool>,
    directory: Option<String>,
    retention_secs: Option<u64>,
    min_free_bytes: Option<u64>,
    max_file_bytes: Option<u64>,
    max_duration_secs: Option<u64>,
    workers: Option<usize>,
    queue_capacity: Option<usize>,
}

#[derive(serde::Deserialize, Debug, Default)]
struct AuthSection {
    users: Option<String>,
    realm: Option<String>,
    nonce: Option<String>,
    secret_key: Option<String>,
}

#[derive(serde::Deserialize, Debug, Default)]
struct SecuritySection {
    internal_secret: Option<String>,
    sbc_rate_limit_enabled: Option<bool>,
    sbc_rate_limit_capacity: Option<f64>,
    sbc_rate_limit_fill_rate: Option<f64>,
    sbc_max_concurrency: Option<u32>,
}

#[derive(serde::Deserialize, Debug, Default)]
struct PerformanceSection {
    cdr_queue_capacity: Option<usize>,
    cdr_persistence_enabled: Option<bool>,
    udp_workers: Option<usize>,
    udp_workers_auto: Option<bool>,
    udp_receive_buffer_bytes: Option<usize>,
    udp_send_buffer_bytes: Option<usize>,
}

#[derive(serde::Deserialize, Debug, Default)]
struct DynamicConfigSection {
    enabled: Option<bool>,
}

#[derive(serde::Deserialize, Debug, Default)]
struct BillingSection {
    balance_enforcement_enabled: Option<bool>,
    settlement_enabled: Option<bool>,
}

#[derive(serde::Deserialize, Debug, Default)]
struct RoutingSection {
    default_gateway: Option<String>,
    database_routes_enabled: Option<bool>,
    gateway_health_checks_enabled: Option<bool>,
}

#[derive(serde::Deserialize, Debug, Default)]
struct NatTraversalSection {
    stun_server: Option<String>,
    upnp_enabled: Option<bool>,
}

fn load_file_content<P: AsRef<Path>>(path: P) -> Option<String> {
    fs::read_to_string(path).ok()
}

fn find_config_file() -> String {
    if let Ok(val) = env::var("VOS_RS_CONFIG_FILE") {
        return val;
    }
    let mut path = std::env::current_dir().unwrap_or_default();
    loop {
        let config_path = path.join("config.yaml");
        if config_path.exists() {
            return config_path.to_string_lossy().into_owned();
        }
        if !path.pop() {
            break;
        }
    }
    "config.yaml".to_string()
}

impl EdgeConfig {
    pub fn from_env() -> Self {
        Self::load()
    }

    pub fn load() -> Self {
        let config_file_path = find_config_file();
        Self::load_from_file(config_file_path)
    }

    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Self {
        // 1. 读取主引导（Bootstrap）配置文件
        let main_config: MainFileConfig = if let Some(content) = load_file_content(path) {
            serde_yaml::from_str(&content).unwrap_or_default()
        } else {
            MainFileConfig::default()
        };

        let conn_section = main_config.connections.unwrap_or_default();
        let db_section = conn_section.database.unwrap_or_default();
        let nats_section = conn_section.nats.unwrap_or_default();
        let edge_section = main_config.sip_edge.unwrap_or_default();

        let database_url = if let (Some(host), Some(port), Some(username), Some(database)) = (
            db_section.host,
            db_section.port,
            db_section.username,
            db_section.database,
        ) {
            let password = db_section.password.unwrap_or_default();
            let url = if password.is_empty() {
                format!("postgres://{}@{}:{}/{}", username, host, port, database)
            } else {
                format!(
                    "postgres://{}:{}@{}:{}/{}",
                    username, password, host, port, database
                )
            };
            Some(url)
        } else {
            None
        };

        let redis_section = conn_section.redis.unwrap_or_default();
        let redis_url = if let (Some(host), Some(port)) = (redis_section.host, redis_section.port) {
            let password = redis_section.password.unwrap_or_default();
            let db = redis_section.database.unwrap_or(0);
            let url = if password.is_empty() {
                format!("redis://{}:{}/{}", host, port, db)
            } else {
                format!("redis://:{}@{}:{}/{}", password, host, port, db)
            };
            Some(url)
        } else {
            None
        };

        let net_section = edge_section.network.unwrap_or_default();
        let route_section = edge_section.routing.unwrap_or_default();
        let nat_section = edge_section.nat_traversal.unwrap_or_default();
        let media_section = edge_section.media.unwrap_or_default();
        let recording_section = edge_section.recording.unwrap_or_default();
        let auth_section = edge_section.auth.unwrap_or_default();
        let security_section = edge_section.security.unwrap_or_default();
        let performance_section = edge_section.performance.unwrap_or_default();
        let dynamic_config_section = edge_section.dynamic_config.unwrap_or_default();
        let billing_section = edge_section.billing.unwrap_or_default();
        let webhook_config = edge_section.webhooks.unwrap_or_default().into_config();
        let cluster = edge_section.cluster.unwrap_or_default();
        let media_cluster = MediaClusterConfig {
            allocation_strategy: media_section.allocation_strategy.unwrap_or_default(),
            health_check_interval_secs: media_section.health_check_interval_secs.unwrap_or(3),
            unhealthy_threshold: media_section.unhealthy_threshold.unwrap_or(3),
            nodes: media_section.nodes.unwrap_or_default(),
        };
        // RTP 地址和端口只属于节点。这里的内部 MediaConfig 仅承载全局媒体行为参数，
        // 实际分配时始终由选中的 nodes[] 配置覆盖地址与端口范围。
        let bootstrap_media_node = media_cluster.nodes.first();
        let mut media = media::MediaConfig::new_with_symmetric_learning(
            bootstrap_media_node
                .map(|node| node.advertised_addr.clone())
                .unwrap_or_else(|| "127.0.0.1".to_string()),
            bootstrap_media_node.map_or(40_000, |node| node.port_min),
            bootstrap_media_node.map_or(40_100, |node| node.port_max),
            media_section.symmetric_learning.unwrap_or(true),
        );
        media.anti_spoofing = media_section.anti_spoofing.unwrap_or(true);
        media.source_relearn_after_secs = media_section.source_relearn_secs.unwrap_or(30);
        media.recording_enabled = recording_section.enabled.unwrap_or(false);
        media.recording_dir = recording_section
            .directory
            .unwrap_or_else(|| "target/recordings".to_string())
            .into();
        media.recording_retention_secs = recording_section.retention_secs.unwrap_or(604_800);
        media.recording_min_free_bytes = recording_section.min_free_bytes.unwrap_or(536_870_912);
        media.recording_max_file_bytes = recording_section.max_file_bytes.unwrap_or(134_217_728);
        media.recording_max_duration_secs = recording_section.max_duration_secs.unwrap_or(3_600);
        let auth_users = auth_section
            .users
            .as_deref()
            .map(parse_auth_users)
            .unwrap_or_default();
        let auth = AuthConfig {
            realm: auth_section.realm.unwrap_or_else(|| "vos-rs".to_string()),
            nonce: auth_section
                .nonce
                .unwrap_or_else(|| "vos-rs-dev-nonce".to_string()),
            users: auth_users,
            secret_key: auth_section
                .secret_key
                .unwrap_or_else(|| "vos-rs-default-secret-change-me".to_string()),
        };

        // 2. 初始化核心结构，其余所有业务与媒体配置将全部由数据库中的 system_configs 表覆盖
        Self {
            sip_udp_bind: net_section
                .sip_udp_bind
                .unwrap_or_else(|| "0.0.0.0:5060".to_string()),
            advertised_addr: net_section
                .advertised_addr
                .unwrap_or_else(|| "127.0.0.1:5060".to_string()),
            default_gateway: route_section.default_gateway.unwrap_or_default(),
            database_routes_enabled: route_section.database_routes_enabled.unwrap_or(true),
            gateway_health_checks_enabled: route_section
                .gateway_health_checks_enabled
                .unwrap_or(true),
            manage_bind: net_section
                .manage_bind
                .unwrap_or_else(|| "127.0.0.1:8082".to_string()),
            stun_server: nat_section.stun_server,
            upnp_enabled: nat_section.upnp_enabled.unwrap_or(false),
            database_url,
            database_max_connections: db_section.max_connections.unwrap_or(10),
            redis_max_connections: redis_section.max_connections.unwrap_or(10),
            nats_url: nats_section.url,
            nats_cdr_stream: Some(
                nats_section
                    .cdr_stream
                    .unwrap_or_else(|| "VOS_RS_CDR".to_string()),
            ),
            nats_cdr_subject: Some(
                nats_section
                    .cdr_subject
                    .unwrap_or_else(|| "vos_rs.cdr".to_string()),
            ),
            redis_url,
            cluster,
            media_cluster,
            media,
            auth,
            session_expires_gateway: 600,
            session_expires_caller: 1800,
            sbc_allow_rules: Vec::new(),
            sbc_block_rules: Vec::new(),
            sbc_rate_limit_enabled: security_section.sbc_rate_limit_enabled.unwrap_or(true),
            sbc_rate_limit_capacity: security_section.sbc_rate_limit_capacity.unwrap_or(2000.0),
            sbc_rate_limit_fill_rate: security_section.sbc_rate_limit_fill_rate.unwrap_or(500.0),
            sbc_max_concurrency: security_section.sbc_max_concurrency.unwrap_or(2000),
            tls_cert_path: None,
            tls_key_path: None,
            tls_bind_addr: None,
            tls_allow_test_certificate: false,
            tls_ca_path: None,
            tls_insecure_skip_verify: false,
            tls_server_name: None,
            udp_workers: performance_section
                .udp_workers
                .unwrap_or_else(|| num_cpus::get().max(1)),
            udp_workers_auto: performance_section.udp_workers_auto.unwrap_or(false),
            udp_receive_buffer_bytes: performance_section
                .udp_receive_buffer_bytes
                .unwrap_or(DEFAULT_UDP_BUFFER_BYTES),
            udp_send_buffer_bytes: performance_section
                .udp_send_buffer_bytes
                .unwrap_or(DEFAULT_UDP_BUFFER_BYTES),
            ws_bind_addr: net_section.ws_bind,
            internal_secret: security_section
                .internal_secret
                .unwrap_or_else(|| "internal-dev-secret".to_string()),
            bootstrap_auth_users: auth_section.users,
            cdr_queue_capacity: performance_section
                .cdr_queue_capacity
                .unwrap_or(4096)
                .max(1),
            cdr_persistence_enabled: performance_section.cdr_persistence_enabled.unwrap_or(true),
            recording_workers: recording_section.workers.unwrap_or(4).max(1),
            recording_queue_capacity: recording_section.queue_capacity.unwrap_or(10_000).max(1),
            media_metrics_log: media_section.metrics_log.unwrap_or(false),
            dynamic_config_enabled: dynamic_config_section.enabled.unwrap_or(true),
            balance_enforcement_enabled: billing_section
                .balance_enforcement_enabled
                .unwrap_or(true),
            billing_settlement_enabled: billing_section.settlement_enabled.unwrap_or(true),
            webhooks: webhook_config,
        }
    }

    pub async fn override_from_db(&mut self, db: &cdr_core::PostgresCdrStore) {
        // Try reading from Redis first
        let mut redis_configs = std::collections::HashMap::new();
        let redis_url = self
            .redis_url
            .clone()
            .unwrap_or_else(|| "redis://127.0.0.1:6379".to_string());

        if let Ok(client) = redis::Client::open(redis_url) {
            if let Ok(mut con) = client.get_multiplexed_tokio_connection().await {
                let res: Result<std::collections::HashMap<String, String>, redis::RedisError> =
                    redis::cmd("HGETALL")
                        .arg("vos_rs:system_configs")
                        .query_async(&mut con)
                        .await;
                if let Ok(hash) = res {
                    redis_configs = hash;
                    tracing::info!("Successfully loaded system configs from Redis");
                }
            }
        }

        // Helper macro to get config either from Redis or fallback to PostgreSQL
        macro_rules! get_val {
            ($key:expr) => {
                get_config_val(&redis_configs, db, $key)
            };
        }

        if let Some(val) = get_val!("session_expires_gateway").await {
            if let Ok(v) = val.parse() {
                self.session_expires_gateway = v;
            }
        }
        if let Some(val) = get_val!("session_expires_caller").await {
            if let Ok(v) = val.parse() {
                self.session_expires_caller = v;
            }
        }
        if let Some(val) = get_val!("sbc_rate_limit_enabled").await {
            if let Ok(v) = val.parse() {
                self.sbc_rate_limit_enabled = v;
            }
        }
        if let Some(val) = get_val!("sbc_rate_limit_capacity").await {
            if let Ok(v) = val.parse() {
                self.sbc_rate_limit_capacity = v;
            }
        }
        if let Some(val) = get_val!("sbc_rate_limit_fill_rate").await {
            if let Ok(v) = val.parse() {
                self.sbc_rate_limit_fill_rate = v;
            }
        }
        if let Some(val) = get_val!("sbc_max_concurrency").await {
            if let Ok(v) = val.parse() {
                self.sbc_max_concurrency = v;
            }
        }
        if let Some(val) = get_val!("tls_cert_path").await {
            self.tls_cert_path = Some(val);
        }
        if let Some(val) = get_val!("tls_key_path").await {
            self.tls_key_path = Some(val);
        }
        if let Some(val) = get_val!("tls_bind_addr").await {
            self.tls_bind_addr = Some(val);
        }
        if let Some(val) = get_val!("tls_allow_test_certificate").await {
            self.tls_allow_test_certificate = val == "true" || val == "1";
        }
        if let Some(val) = get_val!("tls_ca_path").await {
            self.tls_ca_path = Some(val);
        }
        if let Some(val) = get_val!("tls_insecure_skip_verify").await {
            self.tls_insecure_skip_verify = val == "true" || val == "1";
        }
        if let Some(val) = get_val!("tls_server_name").await {
            self.tls_server_name = Some(val);
        }
        if let Some(val) = get_val!("udp_workers").await {
            if let Ok(v) = val.parse() {
                self.udp_workers = v;
            }
        }
        if let Some(val) = get_val!("udp_workers_auto").await {
            self.udp_workers_auto = val == "true" || val == "1";
        }
        if let Some(val) = get_val!("udp_receive_buffer_bytes").await {
            if let Ok(v) = val.parse() {
                self.udp_receive_buffer_bytes = v;
            }
        }
        if let Some(val) = get_val!("udp_send_buffer_bytes").await {
            if let Ok(v) = val.parse() {
                self.udp_send_buffer_bytes = v;
            }
        }
        if let Some(val) = get_val!("cdr_queue_capacity").await {
            if let Ok(v) = val.parse::<usize>() {
                self.cdr_queue_capacity = v.max(1);
            }
        }
        if let Some(val) = get_val!("cdr_persistence_enabled").await {
            self.cdr_persistence_enabled = val == "true" || val == "1";
        }
        if let Some(val) = get_val!("recording_workers").await {
            if let Ok(v) = val.parse::<usize>() {
                self.recording_workers = v.max(1);
            }
        }
        if let Some(val) = get_val!("recording_queue_capacity").await {
            if let Ok(v) = val.parse::<usize>() {
                self.recording_queue_capacity = v.max(1);
            }
        }
        if let Some(val) = get_val!("media_metrics_log").await {
            self.media_metrics_log = val == "true" || val == "1";
        }
        if let Some(val) = get_val!("media_cluster_json").await {
            match serde_json::from_str::<MediaClusterConfig>(&val) {
                Ok(config)
                    if self
                        .cluster
                        .validate(self.redis_url.as_deref(), self.nats_url.as_deref(), &config)
                        .is_ok() =>
                {
                    self.media_cluster = config;
                }
                Ok(_) => tracing::warn!("忽略未通过校验的动态媒体集群配置"),
                Err(error) => tracing::warn!(%error, "动态媒体集群配置 JSON 无效"),
            }
        }
        if let Some(val) = get_val!("balance_enforcement_enabled").await {
            self.balance_enforcement_enabled = val == "true" || val == "1";
        }
        if let Some(val) = get_val!("billing_settlement_enabled").await {
            self.billing_settlement_enabled = val == "true" || val == "1";
        }
        if let Some(val) = get_val!("gateway_health_checks_enabled").await {
            self.gateway_health_checks_enabled = val == "true" || val == "1";
        }

        // 地址和端口由 media_cluster_json 的 nodes[] 管理；这里只覆盖全局媒体行为。
        if let Some(val) = get_val!("rtp_symmetric_learning").await {
            self.media.symmetric_rtp_learning = val == "true" || val == "1";
        }
        if let Some(val) = get_val!("rtp_anti_spoofing").await {
            self.media.anti_spoofing = val == "true" || val == "1";
        }
        if let Some(val) = get_val!("rtp_source_relearn_secs").await {
            if let Ok(v) = val.parse() {
                self.media.source_relearn_after_secs = v;
            }
        }
        if let Some(val) = get_val!("recording_enabled").await {
            self.media.recording_enabled = val == "true" || val == "1";
        }
        if let Some(val) = get_val!("recording_dir").await {
            self.media.recording_dir = std::path::PathBuf::from(val);
        }
        if let Some(val) = get_val!("recording_retention_secs").await {
            if let Ok(v) = val.parse() {
                self.media.recording_retention_secs = v;
            }
        }
        if let Some(val) = get_val!("recording_min_free_bytes").await {
            if let Ok(v) = val.parse() {
                self.media.recording_min_free_bytes = v;
            }
        }
        if let Some(val) = get_val!("recording_max_file_bytes").await {
            if let Ok(v) = val.parse() {
                self.media.recording_max_file_bytes = v;
            }
        }
        if let Some(val) = get_val!("recording_max_duration_secs").await {
            if let Ok(v) = val.parse() {
                self.media.recording_max_duration_secs = v;
            }
        }

        // 覆盖 Auth Config 中的相关属性
        if let Some(val) = get_val!("realm").await {
            self.auth.realm = val;
        }
        if let Some(val) = get_val!("nonce").await {
            self.auth.nonce = val;
        }
        if let Some(val) = get_val!("secret_key").await {
            self.auth.secret_key = val;
        }
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

fn parse_auth_users(raw: &str) -> std::collections::HashMap<String, String> {
    raw.split(',')
        .filter_map(|entry| entry.trim().split_once(':'))
        .map(|(username, password)| (username.trim().to_string(), password.trim().to_string()))
        .filter(|(username, _)| !username.is_empty())
        .collect()
}

async fn get_config_val(
    redis_configs: &std::collections::HashMap<String, String>,
    db: &cdr_core::PostgresCdrStore,
    key: &str,
) -> Option<String> {
    if let Some(val) = redis_configs.get(key) {
        if !val.is_empty() {
            return Some(val.clone());
        }
    }
    if let Ok(Some(val)) = db.get_system_config(key).await {
        return Some(val);
    }
    None
}

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
            ws_bind_addr: None,
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
        }
    }
}
