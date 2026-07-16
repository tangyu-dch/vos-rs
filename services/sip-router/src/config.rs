use std::{env, fs, path::Path};

use serde::Deserialize;

/// 原生 SIP 路由器配置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RouterConfig {
    pub(crate) udp_bind: String,
    pub(crate) tcp_bind: String,
    pub(crate) advertised_addr: String,
    pub(crate) redis_url: String,
    pub(crate) node_key_prefix: String,
    pub(crate) discovery_interval_secs: u64,
    pub(crate) transaction_ttl_secs: u64,
    pub(crate) dialog_route_ttl_secs: u64,
    pub(crate) udp_workers: usize,
    pub(crate) udp_queue_capacity: usize,
    pub(crate) max_transactions: usize,
    pub(crate) tcp_max_connections: usize,
    pub(crate) tcp_write_queue_capacity: usize,
    pub(crate) tcp_idle_timeout_secs: u64,
    pub(crate) tcp_connect_timeout_secs: u64,
    pub(crate) manage_bind: String,
    pub(crate) acl_allow: Vec<String>,
    pub(crate) acl_block: Vec<String>,
    pub(crate) rate_limit_capacity: u32,
    pub(crate) rate_limit_fill_rate: u32,
    pub(crate) rate_limit_max_entries: usize,
}

#[derive(Debug, Default, Deserialize)]
struct RootConfig {
    connections: Option<ConnectionsSection>,
    sip_router: Option<SipRouterSection>,
}

#[derive(Debug, Default, Deserialize)]
struct ConnectionsSection {
    redis: Option<RedisSection>,
}

#[derive(Debug, Default, Deserialize)]
struct RedisSection {
    host: Option<String>,
    port: Option<u16>,
    password: Option<String>,
    database: Option<u16>,
}

#[derive(Debug, Default, Deserialize)]
struct SipRouterSection {
    udp_bind: Option<String>,
    tcp_bind: Option<String>,
    advertised_addr: Option<String>,
    node_key_prefix: Option<String>,
    discovery_interval_secs: Option<u64>,
    transaction_ttl_secs: Option<u64>,
    dialog_route_ttl_secs: Option<u64>,
    udp_workers: Option<usize>,
    udp_queue_capacity: Option<usize>,
    max_transactions: Option<usize>,
    tcp_max_connections: Option<usize>,
    tcp_write_queue_capacity: Option<usize>,
    tcp_idle_timeout_secs: Option<u64>,
    tcp_connect_timeout_secs: Option<u64>,
    manage_bind: Option<String>,
    acl_allow: Option<Vec<String>>,
    acl_block: Option<Vec<String>>,
    rate_limit_capacity: Option<u32>,
    rate_limit_fill_rate: Option<u32>,
    rate_limit_max_entries: Option<usize>,
}

impl RouterConfig {
    pub(crate) fn load() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let path = env::var("VOS_RS_CONFIG_FILE").unwrap_or_else(|_| "config.yaml".to_string());
        Self::load_from_file(path)
    }

    fn load_from_file<P: AsRef<Path>>(
        path: P,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let content = fs::read_to_string(path)?;
        let root: RootConfig = serde_yaml::from_str(&content)?;
        let redis = root
            .connections
            .unwrap_or_default()
            .redis
            .unwrap_or_default();
        let host = redis.host.ok_or("sip-router 缺少 connections.redis.host")?;
        let port = redis.port.unwrap_or(6379);
        let database = redis.database.unwrap_or(0);
        let password = redis.password.unwrap_or_default();
        let redis_url = if password.is_empty() {
            format!("redis://{host}:{port}/{database}")
        } else {
            format!("redis://:{password}@{host}:{port}/{database}")
        };
        let router = root.sip_router.unwrap_or_default();
        let udp_bind = router
            .udp_bind
            .unwrap_or_else(|| "0.0.0.0:5060".to_string());
        Ok(Self {
            tcp_bind: router.tcp_bind.unwrap_or_else(|| udp_bind.clone()),
            udp_bind,
            advertised_addr: router
                .advertised_addr
                .unwrap_or_else(|| "127.0.0.1:5060".to_string()),
            redis_url,
            node_key_prefix: router
                .node_key_prefix
                .unwrap_or_else(|| "vos_rs:cluster:sip_nodes".to_string()),
            discovery_interval_secs: router.discovery_interval_secs.unwrap_or(2).max(1),
            transaction_ttl_secs: router.transaction_ttl_secs.unwrap_or(64).max(1),
            dialog_route_ttl_secs: router.dialog_route_ttl_secs.unwrap_or(86_400).max(60),
            udp_workers: resolve_udp_workers(router.udp_workers),
            udp_queue_capacity: router.udp_queue_capacity.unwrap_or(4096).clamp(64, 65_536),
            max_transactions: router
                .max_transactions
                .unwrap_or(1_000_000)
                .clamp(1024, 10_000_000),
            tcp_max_connections: router
                .tcp_max_connections
                .unwrap_or(10_000)
                .clamp(1, 1_000_000),
            tcp_write_queue_capacity: router
                .tcp_write_queue_capacity
                .unwrap_or(1024)
                .clamp(16, 65_536),
            tcp_idle_timeout_secs: router.tcp_idle_timeout_secs.unwrap_or(300).max(10),
            tcp_connect_timeout_secs: router.tcp_connect_timeout_secs.unwrap_or(3).max(1),
            manage_bind: router
                .manage_bind
                .unwrap_or_else(|| "127.0.0.1:8083".to_string()),
            acl_allow: router.acl_allow.unwrap_or_default(),
            acl_block: router.acl_block.unwrap_or_default(),
            rate_limit_capacity: router.rate_limit_capacity.unwrap_or(200).max(1),
            rate_limit_fill_rate: router.rate_limit_fill_rate.unwrap_or(100).max(1),
            rate_limit_max_entries: router
                .rate_limit_max_entries
                .unwrap_or(100_000)
                .clamp(1024, 10_000_000),
        })
    }
}

fn resolve_udp_workers(configured: Option<usize>) -> usize {
    match configured.unwrap_or(0) {
        0 => std::thread::available_parallelism()
            .map(std::num::NonZeroUsize::get)
            .unwrap_or(1)
            .clamp(1, 64),
        workers => workers.clamp(1, 64),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_router_config_uses_shared_redis() {
        let root: RootConfig = serde_yaml::from_str(
            "connections:\n  redis:\n    host: redis\n    port: 6380\nsip_router:\n  udp_bind: 0.0.0.0:5070\n",
        )
        .expect("config should parse");
        assert_eq!(
            root.connections
                .expect("connections")
                .redis
                .expect("redis")
                .host
                .as_deref(),
            Some("redis")
        );
        assert_eq!(
            root.sip_router.expect("router").udp_bind.as_deref(),
            Some("0.0.0.0:5070")
        );
    }

    #[test]
    fn test_udp_worker_count_is_bounded_and_zero_means_auto() {
        assert!((1..=64).contains(&resolve_udp_workers(Some(0))));
        assert_eq!(resolve_udp_workers(Some(128)), 64);
        assert_eq!(resolve_udp_workers(Some(4)), 4);
    }
}
