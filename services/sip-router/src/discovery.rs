use std::{net::SocketAddr, sync::Arc, time::Duration};

use futures::StreamExt;
use redis::AsyncCommands;
use serde::Deserialize;
use tokio::sync::RwLock;

use crate::config::RouterConfig;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SipNode {
    pub(crate) id: String,
    pub(crate) address: SocketAddr,
}

#[derive(Debug, Deserialize)]
struct SipNodeRecord {
    node_id: String,
    advertised_addr: String,
}

pub(crate) type SharedNodes = Arc<RwLock<Vec<SipNode>>>;

pub(crate) async fn start(
    client: redis::Client,
    config: RouterConfig,
) -> Result<SharedNodes, Box<dyn std::error::Error + Send + Sync>> {
    let nodes = Arc::new(RwLock::new(Vec::new()));
    refresh(&client, &config.node_key_prefix, &nodes).await?;
    let background_nodes = Arc::clone(&nodes);
    tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(Duration::from_secs(config.discovery_interval_secs));
        interval.tick().await;
        loop {
            interval.tick().await;
            if let Err(error) = refresh(&client, &config.node_key_prefix, &background_nodes).await {
                tracing::warn!(%error, "刷新 SIP 节点列表失败");
            }
        }
    });
    Ok(nodes)
}

async fn refresh(
    client: &redis::Client,
    prefix: &str,
    nodes: &SharedNodes,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut connection = client.get_multiplexed_tokio_connection().await?;
    let pattern = format!("{prefix}:*");
    let iterator = connection.scan_match::<_, String>(pattern).await?;
    let keys: Vec<String> = iterator.collect().await;
    if keys.is_empty() {
        nodes.write().await.clear();
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
        let record: SipNodeRecord = serde_json::from_str(&payload)?;
        discovered.push(SipNode {
            id: record.node_id,
            address: record.advertised_addr.parse()?,
        });
    }
    discovered.sort_by(|left, right| left.id.cmp(&right.id));
    *nodes.write().await = discovered;
    Ok(())
}
