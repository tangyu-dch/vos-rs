use std::{
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant},
};

use dashmap::DashMap;
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
    message_id: String,
    target: String,
    bytes: Vec<u8>,
}

#[derive(Debug, Serialize, Deserialize)]
struct InterNodeAck {
    message_id: String,
    accepted: bool,
    error: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ClusterEgress {
    pub(crate) node_id: String,
    subject_prefix: String,
    client: async_nats::Client,
    ack_timeout: Duration,
    max_retries: u32,
    retry_delay: Duration,
}

impl ClusterEgress {
    pub(crate) async fn publish(
        &self,
        owner_node_id: &str,
        target: SocketAddr,
        bytes: Vec<u8>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let message_id = uuid::Uuid::new_v4().to_string();
        let payload = serde_json::to_vec(&InterNodeEgress {
            message_id: message_id.clone(),
            target: target.to_string(),
            bytes,
        })?;
        let subject = format!("{}.{}.egress", self.subject_prefix, owner_node_id);
        let mut last_error = "未收到接收确认".to_string();
        for attempt in 0..=self.max_retries {
            let response = tokio::time::timeout(
                self.ack_timeout,
                self.client.request(subject.clone(), payload.clone().into()),
            )
            .await;
            match response {
                Ok(Ok(message)) => match serde_json::from_slice::<InterNodeAck>(&message.payload) {
                    Ok(ack) if ack.message_id == message_id && ack.accepted => return Ok(()),
                    Ok(ack) => {
                        last_error = ack.error.unwrap_or_else(|| "接收节点拒绝消息".to_string())
                    }
                    Err(error) => last_error = format!("确认消息格式无效: {error}"),
                },
                Ok(Err(error)) => last_error = format!("NATS 请求失败: {error}"),
                Err(_) => last_error = "等待接收节点确认超时".to_string(),
            }
            if attempt < self.max_retries {
                tokio::time::sleep(retry_delay(self.retry_delay, attempt)).await;
            }
        }
        Err(format!("跨节点 SIP 投递失败: {last_error}").into())
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
        ack_timeout: Duration::from_millis(config.cluster.inter_node_ack_timeout_ms),
        max_retries: config.cluster.inter_node_max_retries,
        retry_delay: Duration::from_millis(config.cluster.inter_node_retry_delay_ms),
    };
    state.set_cluster_egress(egress);
    let subject = format!(
        "{}.{}.egress",
        config.cluster.nats_subject_prefix, config.cluster.node_id
    );
    let mut subscriber = client.subscribe(subject.clone()).await?;
    let dedupe = Arc::new(DashMap::<String, Instant>::new());
    spawn_dedupe_cleanup(
        Arc::clone(&dedupe),
        Duration::from_secs(config.cluster.inter_node_dedupe_ttl_secs),
    );
    tracing::info!(%subject, "SIP 集群跨节点可靠投递订阅已启动");
    tokio::spawn(async move {
        while let Some(message) = subscriber.next().await {
            let reply = message.reply.clone();
            let ack = accept_message(&state, &dedupe, &message.payload).await;
            if let Some(reply) = reply {
                if let Ok(payload) = serde_json::to_vec(&ack) {
                    if let Err(error) = client.publish(reply, payload.into()).await {
                        tracing::warn!(%error, "发送跨节点 SIP 投递确认失败");
                    }
                }
            }
        }
    });
    Ok(())
}

async fn accept_message(
    state: &EdgeState,
    dedupe: &DashMap<String, Instant>,
    payload: &[u8],
) -> InterNodeAck {
    let egress = match serde_json::from_slice::<InterNodeEgress>(payload) {
        Ok(egress) => egress,
        Err(error) => {
            return InterNodeAck {
                message_id: String::new(),
                accepted: false,
                error: Some(format!("消息格式无效: {error}")),
            };
        }
    };
    if dedupe.contains_key(&egress.message_id) {
        return accepted_ack(egress.message_id);
    }
    let target = match egress.target.parse::<SocketAddr>() {
        Ok(target) => target,
        Err(error) => return rejected_ack(egress.message_id, format!("目标地址无效: {error}")),
    };
    let Some(connection) = state.get_tcp_connection(&target) else {
        return rejected_ack(
            egress.message_id,
            "本地 TCP/WebSocket 连接已失效".to_string(),
        );
    };
    match connection.try_send(egress.bytes) {
        Ok(()) => {
            dedupe.insert(egress.message_id.clone(), Instant::now());
            accepted_ack(egress.message_id)
        }
        Err(error) => rejected_ack(egress.message_id, format!("本地连接写队列不可用: {error}")),
    }
}

fn accepted_ack(message_id: String) -> InterNodeAck {
    InterNodeAck {
        message_id,
        accepted: true,
        error: None,
    }
}

fn rejected_ack(message_id: String, error: String) -> InterNodeAck {
    InterNodeAck {
        message_id,
        accepted: false,
        error: Some(error),
    }
}

fn spawn_dedupe_cleanup(dedupe: Arc<DashMap<String, Instant>>, ttl: Duration) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(ttl.min(Duration::from_secs(30)));
        interval.tick().await;
        loop {
            interval.tick().await;
            let cutoff = Instant::now() - ttl;
            dedupe.retain(|_, accepted_at| *accepted_at > cutoff);
        }
    });
}

fn retry_delay(base: Duration, attempt: u32) -> Duration {
    base.saturating_mul(2_u32.saturating_pow(attempt.min(10)))
        .min(Duration::from_secs(2))
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

    #[test]
    fn test_retry_delay_is_capped() {
        assert_eq!(
            retry_delay(Duration::from_millis(50), 0),
            Duration::from_millis(50)
        );
        assert_eq!(
            retry_delay(Duration::from_millis(50), 20),
            Duration::from_secs(2)
        );
    }

    #[tokio::test]
    async fn test_duplicate_message_is_acked_without_second_local_write() {
        let (sender, _receiver) = tokio::sync::mpsc::channel(1);
        let state = EdgeState::new(call_core::CallManager::new(
            call_core::RouteTable::default(),
            sender,
        ));
        let dedupe = DashMap::new();
        dedupe.insert("duplicate-id".to_string(), Instant::now());
        let payload = serde_json::to_vec(&InterNodeEgress {
            message_id: "duplicate-id".to_string(),
            target: "127.0.0.1:5090".to_string(),
            bytes: b"OPTIONS".to_vec(),
        })
        .expect("payload");

        let ack = accept_message(&state, &dedupe, &payload).await;

        assert!(ack.accepted);
        assert_eq!(ack.message_id, "duplicate-id");
    }

    #[tokio::test]
    #[ignore = "需要本机 NATS 4222"]
    async fn test_inter_node_egress_waits_for_matching_ack() {
        let client = async_nats::connect("nats://127.0.0.1:4222")
            .await
            .expect("NATS");
        let subject_prefix = format!("vos_rs.test.{}", uuid::Uuid::new_v4());
        let subject = format!("{subject_prefix}.sip-b.egress");
        let mut subscriber = client.subscribe(subject).await.expect("subscribe");
        let responder = client.clone();
        tokio::spawn(async move {
            let message = subscriber.next().await.expect("message");
            let egress: InterNodeEgress =
                serde_json::from_slice(&message.payload).expect("payload");
            let ack = accepted_ack(egress.message_id);
            responder
                .publish(
                    message.reply.expect("reply"),
                    serde_json::to_vec(&ack).expect("ack").into(),
                )
                .await
                .expect("respond");
        });
        let egress = ClusterEgress {
            node_id: "sip-a".to_string(),
            subject_prefix,
            client,
            ack_timeout: Duration::from_secs(1),
            max_retries: 1,
            retry_delay: Duration::from_millis(10),
        };

        egress
            .publish(
                "sip-b",
                "127.0.0.1:5090".parse().expect("target"),
                b"OPTIONS".to_vec(),
            )
            .await
            .expect("publish with ack");
    }
}
