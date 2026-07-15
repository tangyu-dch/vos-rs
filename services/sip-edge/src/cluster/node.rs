use std::time::{Duration, SystemTime, UNIX_EPOCH};

use redis::AsyncCommands;
use serde::Serialize;

use super::{ClusterConfig, RouterMode};

const SIP_NODE_KEY_PREFIX: &str = "vos_rs:cluster:sip_nodes";

#[derive(Debug, Serialize)]
struct SipNodeRecord<'a> {
    node_id: &'a str,
    advertised_addr: &'a str,
    router_mode: RouterMode,
    updated_at: u64,
}

/// 写入首个节点心跳，并启动后台续约任务。
pub(crate) async fn spawn_node_heartbeat(
    redis_client: &redis::Client,
    config: &ClusterConfig,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if !config.enabled {
        return Ok(());
    }

    let mut connection = redis_client.get_multiplexed_tokio_connection().await?;
    write_heartbeat(&mut connection, config).await?;

    tracing::info!(
        node_id = %config.node_id,
        advertised_addr = %config.advertised_addr,
        router_mode = ?config.router_mode,
        "SIP 集群节点已注册"
    );

    let client = redis_client.clone();
    let config = config.clone();
    tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(Duration::from_secs(config.heartbeat_interval_secs.max(1)));
        interval.tick().await;
        loop {
            interval.tick().await;
            match client.get_multiplexed_tokio_connection().await {
                Ok(mut connection) => {
                    if let Err(error) = write_heartbeat(&mut connection, &config).await {
                        tracing::warn!(
                            node_id = %config.node_id,
                            %error,
                            "SIP 集群节点心跳写入失败"
                        );
                    }
                }
                Err(error) => tracing::warn!(
                    node_id = %config.node_id,
                    %error,
                    "SIP 集群节点心跳 Redis 连接失败"
                ),
            }
        }
    });

    Ok(())
}

async fn write_heartbeat(
    connection: &mut redis::aio::MultiplexedConnection,
    config: &ClusterConfig,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let payload = heartbeat_payload(config)?;
    let key = node_key(&config.node_id);
    let ttl = config.node_timeout_secs.max(1);
    let _: () = connection.set_ex(key, payload, ttl).await?;
    Ok(())
}

fn heartbeat_payload(config: &ClusterConfig) -> Result<String, serde_json::Error> {
    let updated_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    serde_json::to_string(&SipNodeRecord {
        node_id: &config.node_id,
        advertised_addr: &config.advertised_addr,
        router_mode: config.router_mode,
        updated_at,
    })
}

fn node_key(node_id: &str) -> String {
    format!("{SIP_NODE_KEY_PREFIX}:{node_id}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_heartbeat_payload_contains_routable_node_data() {
        let config = ClusterConfig {
            enabled: true,
            node_id: "sip-a".to_string(),
            advertised_addr: "10.0.0.11:5060".to_string(),
            router_mode: RouterMode::Native,
            ..ClusterConfig::default()
        };

        let payload = heartbeat_payload(&config).expect("heartbeat payload should serialize");

        assert!(payload.contains("\"node_id\":\"sip-a\""));
        assert!(payload.contains("\"router_mode\":\"native\""));
        assert_eq!(node_key("sip-a"), "vos_rs:cluster:sip_nodes:sip-a");
    }
}
