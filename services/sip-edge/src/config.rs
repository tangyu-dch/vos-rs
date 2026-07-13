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
    pub advertised_addr: String,
    pub database_url: Option<String>,
    pub nats_url: Option<String>,
    pub nats_cdr_stream: Option<String>,
    pub nats_cdr_subject: Option<String>,
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
    advertised_addr: Option<String>,
    database_url: Option<String>,
    nats_url: Option<String>,
    nats_cdr_stream: Option<String>,
    nats_cdr_subject: Option<String>,
    session_expires_gateway: Option<u32>,
    session_expires_caller: Option<u32>,
    tls_cert_path: Option<String>,
    tls_key_path: Option<String>,
    tls_bind_addr: Option<String>,
    tls_allow_test_certificate: Option<bool>,
    tls_ca_path: Option<String>,
    tls_insecure_skip_verify: Option<bool>,
    tls_server_name: Option<String>,
    udp_workers: Option<usize>,
    udp_workers_auto: Option<bool>,
    udp_receive_buffer_bytes: Option<usize>,
    udp_send_buffer_bytes: Option<usize>,

    // Sub-config paths
    media_config_path: Option<String>,
    auth_config_path: Option<String>,
    sbc_config_path: Option<String>,
}

#[derive(serde::Deserialize, Debug, Default)]
struct SbcFileConfig {
    sbc_allow: Option<Vec<String>>,
    sbc_block: Option<Vec<String>>,
    sbc_rate_limit_capacity: Option<f64>,
    sbc_rate_limit_fill_rate: Option<f64>,
    sbc_max_concurrency: Option<u32>,
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

impl EdgeConfig {
    pub fn from_env() -> Self {
        Self::load()
    }

    pub fn load() -> Self {
        let config_file_path =
            env::var("VOS_RS_CONFIG_FILE").unwrap_or_else(|_| "config.yaml".to_string());
        Self::load_from_file(config_file_path)
    }

    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Self {
        // 1. Locate and read main config
        let main_config: MainFileConfig =
            if let Some(interpolated) = load_file_content_with_interpolation(path) {
                serde_yaml::from_str(&interpolated).unwrap_or_default()
            } else {
                MainFileConfig::default()
            };

        // 2. Load sub-configs or fallback to defaults
        let media_path = main_config
            .media_config_path
            .clone()
            .unwrap_or_else(|| "media.yaml".to_string());
        let media: media::MediaConfig = if let Some(interpolated) =
            load_file_content_with_interpolation(&media_path)
        {
            serde_yaml::from_str(&interpolated).unwrap_or_else(|_| media::MediaConfig::from_env())
        } else {
            media::MediaConfig::from_env()
        };

        let auth_path = main_config
            .auth_config_path
            .clone()
            .unwrap_or_else(|| "auth.yaml".to_string());
        let auth: AuthConfig =
            if let Some(interpolated) = load_file_content_with_interpolation(&auth_path) {
                serde_yaml::from_str(&interpolated).unwrap_or_else(|_| AuthConfig::from_env())
            } else {
                AuthConfig::from_env()
            };

        let sbc_path = main_config
            .sbc_config_path
            .clone()
            .unwrap_or_else(|| "sbc.yaml".to_string());
        let sbc: SbcFileConfig =
            if let Some(interpolated) = load_file_content_with_interpolation(&sbc_path) {
                serde_yaml::from_str(&interpolated).unwrap_or_default()
            } else {
                SbcFileConfig::default()
            };

        let sbc_allow_rules = sbc.sbc_allow.unwrap_or_else(|| {
            env::var("VOS_RS_SBC_ALLOW")
                .unwrap_or_default()
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        });

        let sbc_block_rules = sbc.sbc_block.unwrap_or_else(|| {
            env::var("VOS_RS_SBC_BLOCK")
                .unwrap_or_default()
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        });

        let sbc_rate_limit_capacity = sbc.sbc_rate_limit_capacity.unwrap_or_else(|| {
            env::var("VOS_RS_SBC_LIMIT_CAPACITY")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(2000.0)
        });

        let sbc_rate_limit_fill_rate = sbc.sbc_rate_limit_fill_rate.unwrap_or_else(|| {
            env::var("VOS_RS_SBC_LIMIT_FILL_RATE")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(500.0)
        });

        let sbc_max_concurrency = sbc.sbc_max_concurrency.unwrap_or_else(|| {
            env::var("VOS_RS_SBC_MAX_CONCURRENCY")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(2000)
        });

        Self {
            advertised_addr: main_config.advertised_addr.unwrap_or_else(|| {
                env::var(ADVERTISED_ADDR_ENV)
                    .unwrap_or_else(|_| DEFAULT_ADVERTISED_ADDR.to_string())
            }),
            database_url: main_config
                .database_url
                .or_else(|| env_non_empty(DATABASE_URL_ENV)),
            nats_url: main_config.nats_url.or_else(|| env_non_empty(NATS_URL_ENV)),
            nats_cdr_stream: main_config
                .nats_cdr_stream
                .or_else(|| env_non_empty(NATS_CDR_STREAM_ENV)),
            nats_cdr_subject: main_config
                .nats_cdr_subject
                .or_else(|| env_non_empty(NATS_CDR_SUBJECT_ENV)),
            media,
            auth,
            session_expires_gateway: main_config.session_expires_gateway.unwrap_or_else(|| {
                env::var("VOS_RS_SESSION_EXPIRES_GATEWAY")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(600)
            }),
            session_expires_caller: main_config.session_expires_caller.unwrap_or_else(|| {
                env::var("VOS_RS_SESSION_EXPIRES_CALLER")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(1800)
            }),
            sbc_allow_rules,
            sbc_block_rules,
            sbc_rate_limit_capacity,
            sbc_rate_limit_fill_rate,
            sbc_max_concurrency,
            tls_cert_path: main_config
                .tls_cert_path
                .or_else(|| env_non_empty(TLS_CERT_PATH_ENV)),
            tls_key_path: main_config
                .tls_key_path
                .or_else(|| env_non_empty(TLS_KEY_PATH_ENV)),
            tls_bind_addr: main_config
                .tls_bind_addr
                .or_else(|| env_non_empty(TLS_BIND_ENV)),
            tls_allow_test_certificate: main_config
                .tls_allow_test_certificate
                .unwrap_or_else(|| env_bool(TLS_ALLOW_TEST_CERT_ENV).unwrap_or(false)),
            tls_ca_path: main_config
                .tls_ca_path
                .or_else(|| env_non_empty(TLS_CA_PATH_ENV)),
            tls_insecure_skip_verify: main_config
                .tls_insecure_skip_verify
                .unwrap_or_else(|| env_bool(TLS_INSECURE_SKIP_VERIFY_ENV).unwrap_or(false)),
            tls_server_name: main_config
                .tls_server_name
                .or_else(|| env_non_empty(TLS_SERVER_NAME_ENV)),
            udp_workers: main_config.udp_workers.unwrap_or_else(|| {
                env::var("VOS_RS_UDP_WORKERS")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or_else(|| num_cpus::get().max(1))
            }),
            udp_workers_auto: main_config
                .udp_workers_auto
                .unwrap_or_else(|| env_bool("VOS_RS_UDP_WORKERS_AUTO").unwrap_or(false)),
            udp_receive_buffer_bytes: main_config.udp_receive_buffer_bytes.unwrap_or_else(|| {
                env_usize_or_default(UDP_RECEIVE_BUFFER_ENV, DEFAULT_UDP_BUFFER_BYTES)
            }),
            udp_send_buffer_bytes: main_config.udp_send_buffer_bytes.unwrap_or_else(|| {
                env_usize_or_default(UDP_SEND_BUFFER_ENV, DEFAULT_UDP_BUFFER_BYTES)
            }),
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
