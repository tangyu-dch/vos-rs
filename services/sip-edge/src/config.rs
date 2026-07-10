use std::env;

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

#[derive(Debug, Clone, PartialEq)]
pub struct EdgeConfig {
    pub advertised_addr: String,
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
    pub tls_allow_test_certificate: bool,
    pub tls_ca_path: Option<String>,
    pub tls_insecure_skip_verify: bool,
    pub tls_server_name: Option<String>,
    pub udp_workers: usize,
    pub udp_workers_auto: bool,
    pub udp_receive_buffer_bytes: usize,
    pub udp_send_buffer_bytes: usize,
}

impl EdgeConfig {
    pub fn from_env() -> Self {
        let sbc_allow_rules = env::var("VOS_RS_SBC_ALLOW")
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        let sbc_block_rules = env::var("VOS_RS_SBC_BLOCK")
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        let sbc_rate_limit_capacity = env::var("VOS_RS_SBC_LIMIT_CAPACITY")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(100.0);
        let sbc_rate_limit_fill_rate = env::var("VOS_RS_SBC_LIMIT_FILL_RATE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(10.0);
        let sbc_max_concurrency = env::var("VOS_RS_SBC_MAX_CONCURRENCY")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(10);

        Self {
            advertised_addr: env::var(ADVERTISED_ADDR_ENV)
                .unwrap_or_else(|_| DEFAULT_ADVERTISED_ADDR.to_string()),
            media: media::MediaConfig::from_env(),
            auth: AuthConfig::from_env(),
            session_expires_gateway: env::var("VOS_RS_SESSION_EXPIRES_GATEWAY")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(600),
            session_expires_caller: env::var("VOS_RS_SESSION_EXPIRES_CALLER")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(1800),
            sbc_allow_rules,
            sbc_block_rules,
            sbc_rate_limit_capacity,
            sbc_rate_limit_fill_rate,
            sbc_max_concurrency,
            tls_cert_path: env_non_empty(TLS_CERT_PATH_ENV),
            tls_key_path: env_non_empty(TLS_KEY_PATH_ENV),
            tls_allow_test_certificate: env_bool(TLS_ALLOW_TEST_CERT_ENV).unwrap_or(false),
            tls_ca_path: env_non_empty(TLS_CA_PATH_ENV),
            tls_insecure_skip_verify: env_bool(TLS_INSECURE_SKIP_VERIFY_ENV).unwrap_or(false),
            tls_server_name: env_non_empty(TLS_SERVER_NAME_ENV),
            udp_workers: env::var("VOS_RS_UDP_WORKERS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or_else(|| num_cpus::get().max(1)),
            udp_workers_auto: env_bool("VOS_RS_UDP_WORKERS_AUTO").unwrap_or(false),
            udp_receive_buffer_bytes: env_usize_or_default(
                UDP_RECEIVE_BUFFER_ENV,
                DEFAULT_UDP_BUFFER_BYTES,
            ),
            udp_send_buffer_bytes: env_usize_or_default(
                UDP_SEND_BUFFER_ENV,
                DEFAULT_UDP_BUFFER_BYTES,
            ),
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
