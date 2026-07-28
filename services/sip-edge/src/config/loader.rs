use std::env;
use std::fs;
use std::path::Path;

use crate::cluster::MediaClusterConfig;
use crate::media;
use crate::sip::AuthConfig;

use super::sections::*;
use super::{EdgeConfig, DEFAULT_UDP_BUFFER_BYTES};

fn load_file_content<P: AsRef<Path>>(path: P) -> Option<String> {
    fs::read_to_string(path).ok()
}

pub(super) fn find_config_file() -> String {
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

fn parse_auth_users(raw: &str) -> std::collections::HashMap<String, String> {
    raw.split(',')
        .filter_map(|entry| entry.trim().split_once(':'))
        .map(|(username, password)| (username.trim().to_string(), password.trim().to_string()))
        .filter(|(username, _)| !username.is_empty())
        .collect()
}

impl EdgeConfig {
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
        media.rtp_dscp = performance_section.rtp_dscp.unwrap_or(0);
        media.recording_enabled = recording_section.enabled.unwrap_or(false);
        media.recording_dir = recording_section
            .directory
            .unwrap_or_else(|| "target/recordings".to_string())
            .into();
        media.recording_retention_secs = recording_section.retention_secs.unwrap_or(604_800);
        media.recording_min_free_bytes = recording_section.min_free_bytes.unwrap_or(536_870_912);
        media.recording_max_file_bytes = recording_section.max_file_bytes.unwrap_or(134_217_728);
        media.recording_max_duration_secs = recording_section.max_duration_secs.unwrap_or(3_600);
        let auth_users = if auth_section.enabled == Some(false) {
            std::collections::HashMap::new()
        } else {
            auth_section
                .users
                .as_deref()
                .map(parse_auth_users)
                .unwrap_or_default()
        };
        let auth = AuthConfig {
            enabled: auth_section.enabled,
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
            sip_dscp: performance_section.sip_dscp.unwrap_or(0),
            rtp_dscp: performance_section.rtp_dscp.unwrap_or(0),
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
            sipflow_enabled: true,
            sipflow_whitelist: "1001,1002".to_string(),
            sipflow_retention_days: 7,
            sip_transaction_timeout_secs: performance_section
                .sip_transaction_timeout_secs
                .unwrap_or(32),
            sip_t1_initial_ms: performance_section.sip_t1_initial_ms.unwrap_or(500),
        }
    }
}
