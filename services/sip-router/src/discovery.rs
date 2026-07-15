use std::{net::SocketAddr, sync::Arc, time::Duration};

use futures::StreamExt;
use redis::AsyncCommands;
use serde::Deserialize;
use tokio::sync::RwLock;

use crate::{config::RouterConfig, metrics};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SipNode {
    pub(crate) id: String,
    pub(crate) address: SocketAddr,
}

#[derive(Debug, Deserialize)]
struct SipNodeRecord {
    node_id: String,
    advertised_addr: String,
    router_mode: String,
    #[serde(default = "default_node_status")]
    status: String,
}

fn default_node_status() -> String {
    "active".to_string()
}

pub(crate) type SharedNodes = Arc<RwLock<Vec<SipNode>>>;

pub(crate) async fn start(
    redis: redis::aio::ConnectionManager,
    config: RouterConfig,
) -> Result<SharedNodes, Box<dyn std::error::Error + Send + Sync>> {
    let nodes = Arc::new(RwLock::new(Vec::new()));
    refresh(redis.clone(), &config.node_key_prefix, &nodes).await?;
    let background_nodes = Arc::clone(&nodes);
    tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(Duration::from_secs(config.discovery_interval_secs));
        interval.tick().await;
        loop {
            interval.tick().await;
            if let Err(error) =
                refresh(redis.clone(), &config.node_key_prefix, &background_nodes).await
            {
                metrics::redis_error();
                tracing::warn!(%error, "刷新 SIP 节点列表失败");
            }
        }
    });
    Ok(nodes)
}

async fn refresh(
    mut connection: redis::aio::ConnectionManager,
    prefix: &str,
    nodes: &SharedNodes,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let pattern = format!("{prefix}:*");
    let iterator = connection.scan_match::<_, String>(pattern).await?;
    let keys: Vec<String> = iterator.collect().await;
    if keys.is_empty() {
        nodes.write().await.clear();
        metrics::discovered_nodes(0);
        return Ok(());
    }
    let payloads: Vec<Option<String>> = redis::cmd("MGET")
        .arg(&keys)
        .query_async(&mut connection)
        .await?;
    let mut discovered = Vec::new();
    for payload in payloads {
        let Some(payload) = payload else {
            continue;
        };
        let Ok(record) = serde_json::from_str::<SipNodeRecord>(&payload) else {
            tracing::warn!("忽略格式无效的 SIP 节点心跳");
            continue;
        };
        if !record_is_routable(&record) {
            continue;
        }
        let Some(address) = resolve_address(&record.advertised_addr).await else {
            tracing::warn!(node_id = %record.node_id, advertised_addr = %record.advertised_addr, "忽略无法解析通告地址的 SIP 节点");
            continue;
        };
        discovered.push(SipNode {
            id: record.node_id,
            address,
        });
    }
    discovered.sort_by(|left, right| left.id.cmp(&right.id));
    metrics::discovered_nodes(discovered.len());
    *nodes.write().await = discovered;
    Ok(())
}

async fn resolve_address(value: &str) -> Option<SocketAddr> {
    if let Ok(address) = value.parse() {
        return Some(address);
    }
    tokio::net::lookup_host(value).await.ok()?.next()
}

fn record_is_routable(record: &SipNodeRecord) -> bool {
    record.router_mode == "native" && record.status == "active"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_draining_node_is_not_routable() {
        let active: SipNodeRecord = serde_json::from_str(
            r#"{"node_id":"a","advertised_addr":"127.0.0.1:5061","router_mode":"native"}"#,
        )
        .expect("active record");
        let draining: SipNodeRecord = serde_json::from_str(
            r#"{"node_id":"b","advertised_addr":"127.0.0.1:5062","router_mode":"native","status":"draining"}"#,
        )
        .expect("draining record");

        assert!(record_is_routable(&active));
        assert!(!record_is_routable(&draining));
    }

    #[tokio::test]
    async fn test_resolve_address_supports_service_hostname() {
        let address = resolve_address("localhost:5060")
            .await
            .expect("localhost should resolve");
        assert_eq!(address.port(), 5060);
    }
}
