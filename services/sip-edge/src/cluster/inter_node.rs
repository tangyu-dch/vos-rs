use std::{net::SocketAddr, sync::Arc};

use futures::StreamExt;
use serde::{Deserialize, Serialize};

use crate::{config::EdgeConfig, edge_state::EdgeState};

const FLOW_KEY_PREFIX: &str = "vos_rs:cluster:sip_flows";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct FlowRecord {
    pub(crate) owner_node_id: String,
    pub(crate) transport: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct InterNodeEgress {
    target: String,
    bytes: Vec<u8>,
}

#[derive(Debug, Clone)]
pub(crate) struct ClusterEgress {
    pub(crate) node_id: String,
    subject_prefix: String,
    client: async_nats::Client,
}

impl ClusterEgress {
    pub(crate) async fn publish(
        &self,
        owner_node_id: &str,
        target: SocketAddr,
        bytes: Vec<u8>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let payload = serde_json::to_vec(&InterNodeEgress {
            target: target.to_string(),
            bytes,
        })?;
        self.client
            .publish(
                format!("{}.{}.egress", self.subject_prefix, owner_node_id),
                payload.into(),
            )
            .await?;
        Ok(())
    }
}

pub(crate) async fn start_inter_node_egress(
    state: Arc<EdgeState>,
    config: &EdgeConfig,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if !config.cluster.enabled {
        return Ok(());
    }
    let nats_url = config
        .nats_url
        .as_deref()
        .ok_or("SIP 集群跨节点投递缺少 NATS URL")?;
    let client = async_nats::connect(nats_url).await?;
    let egress = ClusterEgress {
        node_id: config.cluster.node_id.clone(),
        subject_prefix: config.cluster.nats_subject_prefix.clone(),
        client: client.clone(),
    };
    state.set_cluster_egress(egress);
    let subject = format!(
        "{}.{}.egress",
        config.cluster.nats_subject_prefix, config.cluster.node_id
    );
    let mut subscriber = client.subscribe(subject.clone()).await?;
    tracing::info!(%subject, "SIP 集群跨节点投递订阅已启动");
    tokio::spawn(async move {
        while let Some(message) = subscriber.next().await {
            let Ok(egress) = serde_json::from_slice::<InterNodeEgress>(&message.payload) else {
                tracing::warn!(%subject, "忽略无法解析的跨节点 SIP 投递消息");
                continue;
            };
            let Ok(target) = egress.target.parse::<SocketAddr>() else {
                continue;
            };
            let Some(connection) = state.get_tcp_connection(&target) else {
                tracing::warn!(%target, "跨节点 SIP 投递的本地连接已失效");
                continue;
            };
            if connection.send(egress.bytes).await.is_err() {
                tracing::warn!(%target, "跨节点 SIP 投递写入本地连接失败");
            }
        }
    });
    Ok(())
}

pub(crate) fn flow_key(address: SocketAddr) -> String {
    format!("{FLOW_KEY_PREFIX}:{address}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_flow_key_contains_exact_connection_address() {
        let address: SocketAddr = "127.0.0.1:5090".parse().expect("address");
        assert_eq!(flow_key(address), "vos_rs:cluster:sip_flows:127.0.0.1:5090");
    }

    #[tokio::test]
    #[ignore = "需要本机 NATS 4222"]
    async fn test_inter_node_egress_round_trip_over_nats() {
        let client = async_nats::connect("nats://127.0.0.1:4222")
            .await
            .expect("NATS");
        let subject_prefix = format!("vos_rs.test.{}", uuid::Uuid::new_v4());
        let subject = format!("{subject_prefix}.sip-b.egress");
        let mut subscriber = client.subscribe(subject).await.expect("subscribe");
        let egress = ClusterEgress {
            node_id: "sip-a".to_string(),
            subject_prefix,
            client,
        };
        let target: SocketAddr = "127.0.0.1:5090".parse().expect("target");

        egress
            .publish("sip-b", target, b"OPTIONS".to_vec())
            .await
            .expect("publish");
        let message = tokio::time::timeout(std::time::Duration::from_secs(2), subscriber.next())
            .await
            .expect("message timeout")
            .expect("message");
        let payload: InterNodeEgress = serde_json::from_slice(&message.payload).expect("payload");
        assert_eq!(payload.target, target.to_string());
        assert_eq!(payload.bytes, b"OPTIONS");
    }
}
