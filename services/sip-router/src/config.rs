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
        })
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
}
