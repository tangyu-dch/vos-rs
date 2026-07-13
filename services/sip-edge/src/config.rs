use std::env;
use std::fs;
use std::path::Path;

use crate::media;
use crate::sip::AuthConfig;

pub const ADVERTISED_ADDR_ENV: &str = "VOS_RS_SIP_ADVERTISED_ADDR";
pub const DATABASE_URL_ENV: &str = "VOS_RS_DATABASE_URL";
pub const NATS_URL_ENV: &str = "VOS_RS_NATS_URL";
pub const NATS_CDR_STREAM_ENV: &str = "VOS_RS_NATS_CDR_STREAM";
pub const NATS_CDR_SUBJECT_ENV: &str = "VOS_RS_NATS_CDR_SUBJECT";
pub const DEFAULT_ADVERTISED_ADDR: &str = "127.0.0.1:5060";
pub const DEFAULT_GATEWAY_ENV: &str = "VOS_RS_SIP_DEFAULT_GATEWAY";
pub const TLS_BIND_ENV: &str = "VOS_RS_SIP_TLS_BIND";
pub const TLS_CERT_PATH_ENV: &str = "VOS_RS_SIP_TLS_CERT_PATH";
pub const TLS_KEY_PATH_ENV: &str = "VOS_RS_SIP_TLS_KEY_PATH";
pub const TLS_CA_PATH_ENV: &str = "VOS_RS_SIP_TLS_CA_PATH";
pub const TLS_ALLOW_TEST_CERT_ENV: &str = "VOS_RS_SIP_TLS_ALLOW_TEST_CERT";
pub const TLS_INSECURE_SKIP_VERIFY_ENV: &str = "VOS_RS_SIP_TLS_INSECURE_SKIP_VERIFY";
pub const TLS_SERVER_NAME_ENV: &str = "VOS_RS_SIP_TLS_SERVER_NAME";
pub const UDP_RECEIVE_BUFFER_ENV: &str = "VOS_RS_SIP_UDP_RECEIVE_BUFFER";
pub const UDP_SEND_BUFFER_ENV: &str = "VOS_RS_SIP_UDP_SEND_BUFFER";
pub const DEFAULT_UDP_BUFFER_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EdgeConfig {
    pub sip_udp_bind: String,
    pub advertised_addr: String,
    pub default_gateway: String,
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
    pub media: media::MediaConfig,
    pub auth: AuthConfig,
    pub session_expires_gateway: u32,
    pub session_expires_caller: u32,
    pub sbc_allow_rules: Vec<String>,
    pub sbc_block_rules: Vec<String>,
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
    network: Option<NetworkSection>,
    routing: Option<RoutingSection>,
    nat_traversal: Option<NatTraversalSection>,
}

#[derive(serde::Deserialize, Debug, Default)]
struct NetworkSection {
    sip_udp_bind: Option<String>,
    advertised_addr: Option<String>,
    manage_bind: Option<String>,
}

#[derive(serde::Deserialize, Debug, Default)]
struct RoutingSection {
    default_gateway: Option<String>,
}

#[derive(serde::Deserialize, Debug, Default)]
struct NatTraversalSection {
    stun_server: Option<String>,
    upnp_enabled: Option<bool>,
}

pub fn interpolate_env_vars(content: &str) -> String {
    let mut result = String::new();
    let mut chars = content.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '$' && chars.peek() == Some(&'{') {
            chars.next(); // Consume '{'
            let mut var_expr = String::new();
            for next_c in chars.by_ref() {
                if next_c == '}' {
                    break;
                }
                var_expr.push(next_c);
            }
            if let Some(colon_pos) = var_expr.find(':') {
                let var_name = &var_expr[..colon_pos];
                let default_val = &var_expr[colon_pos + 1..];
                let val = env::var(var_name.trim()).unwrap_or_else(|_| default_val.to_string());
                result.push_str(&val);
            } else {
                let val = env::var(var_expr.trim()).unwrap_or_default();
                result.push_str(&val);
            }
        } else {
            result.push(c);
        }
    }
    result
}

fn load_file_content_with_interpolation<P: AsRef<Path>>(path: P) -> Option<String> {
    let content = fs::read_to_string(path).ok()?;
    Some(interpolate_env_vars(&content))
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
        let main_config: MainFileConfig =
            if let Some(interpolated) = load_file_content_with_interpolation(path) {
                serde_yaml::from_str(&interpolated).unwrap_or_default()
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
                format!("postgres://{}:{}@{}:{}/{}", username, password, host, port, database)
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

        // 2. 初始化核心结构，其余所有业务与媒体配置将全部由数据库中的 system_configs 表覆盖
        Self {
            sip_udp_bind: net_section.sip_udp_bind.unwrap_or_else(|| "0.0.0.0:5060".to_string()),
            advertised_addr: net_section.advertised_addr.unwrap_or_else(|| "127.0.0.1:5060".to_string()),
            default_gateway: route_section.default_gateway.unwrap_or_default(),
            manage_bind: net_section.manage_bind.unwrap_or_else(|| "127.0.0.1:8082".to_string()),
            stun_server: nat_section.stun_server,
            upnp_enabled: nat_section.upnp_enabled.unwrap_or(false),
            database_url,
            database_max_connections: db_section.max_connections.unwrap_or(10),
            redis_max_connections: redis_section.max_connections.unwrap_or(10),
            nats_url: nats_section.url,
            nats_cdr_stream: Some(nats_section.cdr_stream.unwrap_or_else(|| "VOS_RS_CDR".to_string())),
            nats_cdr_subject: Some(nats_section.cdr_subject.unwrap_or_else(|| "vos_rs.cdr".to_string())),
            redis_url,
            media: media::MediaConfig::new_with_symmetric_learning("127.0.0.1", 40000, 40100, true),
            auth: AuthConfig::disabled(),
            session_expires_gateway: 600,
            session_expires_caller: 1800,
            sbc_allow_rules: Vec::new(),
            sbc_block_rules: Vec::new(),
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
            udp_workers: num_cpus::get().max(1),
            udp_workers_auto: false,
            udp_receive_buffer_bytes: DEFAULT_UDP_BUFFER_BYTES,
            udp_send_buffer_bytes: DEFAULT_UDP_BUFFER_BYTES,
        }
    }

    pub async fn override_from_db(&mut self, db: &cdr_core::PostgresCdrStore) {
        // Try reading from Redis first
        let mut redis_configs = std::collections::HashMap::new();
        let redis_url = self.redis_url.clone().unwrap_or_else(|| {
            "redis://127.0.0.1:6379".to_string()
        });

        if let Ok(client) = redis::Client::open(redis_url) {
            if let Ok(mut con) = client.get_multiplexed_tokio_connection().await {
                let res: Result<std::collections::HashMap<String, String>, redis::RedisError> = redis::cmd("HGETALL")
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
            if let Ok(v) = val.parse() { self.session_expires_gateway = v; }
        }
        if let Some(val) = get_val!("session_expires_caller").await {
            if let Ok(v) = val.parse() { self.session_expires_caller = v; }
        }
        if let Some(val) = get_val!("sbc_rate_limit_capacity").await {
            if let Ok(v) = val.parse() { self.sbc_rate_limit_capacity = v; }
        }
        if let Some(val) = get_val!("sbc_rate_limit_fill_rate").await {
            if let Ok(v) = val.parse() { self.sbc_rate_limit_fill_rate = v; }
        }
        if let Some(val) = get_val!("sbc_max_concurrency").await {
            if let Ok(v) = val.parse() { self.sbc_max_concurrency = v; }
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
            if let Ok(v) = val.parse() { self.udp_workers = v; }
        }
        if let Some(val) = get_val!("udp_workers_auto").await {
            self.udp_workers_auto = val == "true" || val == "1";
        }
        if let Some(val) = get_val!("udp_receive_buffer_bytes").await {
            if let Ok(v) = val.parse() { self.udp_receive_buffer_bytes = v; }
        }
        if let Some(val) = get_val!("udp_send_buffer_bytes").await {
            if let Ok(v) = val.parse() { self.udp_send_buffer_bytes = v; }
        }

        // 覆盖 Media Config 中的相关属性
        if let Some(val) = get_val!("rtp_advertised_addr").await {
            self.media.advertised_addr = val;
        }
        if let Some(val) = get_val!("rtp_port_min").await {
            if let Ok(v) = val.parse() { self.media.port_min = v; }
        }
        if let Some(val) = get_val!("rtp_port_max").await {
            if let Ok(v) = val.parse() { self.media.port_max = v; }
        }
        if let Some(val) = get_val!("rtp_symmetric_learning").await {
            self.media.symmetric_rtp_learning = val == "true" || val == "1";
        }
        if let Some(val) = get_val!("rtp_anti_spoofing").await {
            self.media.anti_spoofing = val == "true" || val == "1";
        }
        if let Some(val) = get_val!("rtp_source_relearn_secs").await {
            if let Ok(v) = val.parse() { self.media.source_relearn_after_secs = v; }
        }
        if let Some(val) = get_val!("recording_enabled").await {
            self.media.recording_enabled = val == "true" || val == "1";
        }
        if let Some(val) = get_val!("recording_dir").await {
            self.media.recording_dir = std::path::PathBuf::from(val);
        }
        if let Some(val) = get_val!("recording_retention_secs").await {
            if let Ok(v) = val.parse() { self.media.recording_retention_secs = v; }
        }
        if let Some(val) = get_val!("recording_min_free_bytes").await {
            if let Ok(v) = val.parse() { self.media.recording_min_free_bytes = v; }
        }
        if let Some(val) = get_val!("recording_max_file_bytes").await {
            if let Ok(v) = val.parse() { self.media.recording_max_file_bytes = v; }
        }
        if let Some(val) = get_val!("recording_max_duration_secs").await {
            if let Ok(v) = val.parse() { self.media.recording_max_duration_secs = v; }
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
}

pub fn env_non_empty(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub fn env_bool(name: &str) -> Option<bool> {
    let value = env::var(name).ok()?;
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn env_usize_or_default(name: &str, default: usize) -> usize {
    env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
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
        Self {
            sip_udp_bind: "0.0.0.0:5060".to_string(),
            advertised_addr: "127.0.0.1:5060".to_string(),
            default_gateway: String::new(),
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
            media: media::MediaConfig::new_with_symmetric_learning("127.0.0.1", 40000, 40100, true),
            auth: AuthConfig::disabled(),
            session_expires_gateway: 600,
            session_expires_caller: 1800,
            sbc_allow_rules: Vec::new(),
            sbc_block_rules: Vec::new(),
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
        }
    }
}
