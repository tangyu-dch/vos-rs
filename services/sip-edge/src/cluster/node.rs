use std::{
    sync::{atomic::Ordering, Arc},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use redis::AsyncCommands;
use serde::Serialize;

use super::{ClusterConfig, RouterMode};
use crate::edge_state::EdgeState;

#[derive(Debug, Serialize)]
struct SipNodeRecord<'a> {
    node_id: &'a str,
    advertised_addr: &'a str,
    management_url: &'a str,
    router_mode: RouterMode,
    status: &'static str,
    active_calls: usize,
    version: &'static str,
    started_at: u64,
    updated_at: u64,
}

/// 节点心跳控制句柄，用于摘流时立即刷新状态并在退出前注销节点。
pub(crate) struct NodeHeartbeat {
    client: redis::Client,
    config: ClusterConfig,
    state: Arc<EdgeState>,
    started_at: u64,
    shutdown: tokio::sync::watch::Sender<bool>,
}

/// 写入首个节点心跳，并启动后台续约任务。
pub(crate) async fn spawn_node_heartbeat(
    redis_client: &redis::Client,
    config: &ClusterConfig,
    state: Arc<EdgeState>,
) -> Result<Option<NodeHeartbeat>, Box<dyn std::error::Error + Send + Sync>> {
    if !config.enabled {
        return Ok(None);
    }

    let started_at = unix_timestamp();
    let mut connection = redis_client.get_multiplexed_tokio_connection().await?;
    write_heartbeat(&mut connection, config, &state, started_at).await?;

    tracing::info!(
        node_id = %config.node_id,
        advertised_addr = %config.advertised_addr,
        router_mode = ?config.router_mode,
        "SIP 集群节点已注册"
    );

    let client = redis_client.clone();
    let background_config = config.clone();
    let background_state = Arc::clone(&state);
    let (shutdown, mut shutdown_receiver) = tokio::sync::watch::channel(false);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(
            background_config.heartbeat_interval_secs.max(1),
        ));
        interval.tick().await;
        loop {
            tokio::select! {
                _ = interval.tick() => {}
                changed = shutdown_receiver.changed() => {
                    if changed.is_err() || *shutdown_receiver.borrow() {
                        break;
                    }
                    continue;
                }
            }
            match client.get_multiplexed_tokio_connection().await {
                Ok(mut connection) => {
                    if let Err(error) = write_heartbeat(
                        &mut connection,
                        &background_config,
                        &background_state,
                        started_at,
                    )
                    .await
                    {
                        tracing::warn!(
                            node_id = %background_config.node_id,
                            %error,
                            "SIP 集群节点心跳写入失败"
                        );
                    }
                }
                Err(error) => tracing::warn!(
                    node_id = %background_config.node_id,
                    %error,
                    "SIP 集群节点心跳 Redis 连接失败"
                ),
            }
        }
    });

    Ok(Some(NodeHeartbeat {
        client: redis_client.clone(),
        config: config.clone(),
        state,
        started_at,
        shutdown,
    }))
}

impl NodeHeartbeat {
    /// 立即写入当前摘流状态，不等待下一次心跳周期。
    pub(crate) async fn refresh(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut connection = self.client.get_multiplexed_tokio_connection().await?;
        write_heartbeat(&mut connection, &self.config, &self.state, self.started_at).await
    }

    /// 停止续约并主动删除节点心跳。
    pub(crate) async fn unregister(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let _ = self.shutdown.send(true);
        let mut connection = self.client.get_multiplexed_tokio_connection().await?;
        let _: usize = connection
            .del(node_key(&self.config.node_key_prefix, &self.config.node_id))
            .await?;
        Ok(())
    }
}

async fn write_heartbeat(
    connection: &mut redis::aio::MultiplexedConnection,
    config: &ClusterConfig,
    state: &EdgeState,
    started_at: u64,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let payload = heartbeat_payload(config, state, started_at)?;
    let key = node_key(&config.node_key_prefix, &config.node_id);
    let ttl = config.node_timeout_secs.max(1);
    let _: () = connection.set_ex(key, payload, ttl).await?;
    Ok(())
}

fn heartbeat_payload(
    config: &ClusterConfig,
    state: &EdgeState,
    started_at: u64,
) -> Result<String, serde_json::Error> {
    serde_json::to_string(&SipNodeRecord {
        node_id: &config.node_id,
        advertised_addr: &config.advertised_addr,
        management_url: &config.management_url,
        router_mode: config.router_mode,
        status: if state.draining.load(Ordering::Acquire) {
            "draining"
        } else {
            "active"
        },
        active_calls: state.call_manager.active_calls_count(),
        version: env!("CARGO_PKG_VERSION"),
        started_at,
        updated_at: unix_timestamp(),
    })
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn node_key(prefix: &str, node_id: &str) -> String {
    format!("{}:{node_id}", prefix.trim_end_matches(':'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_heartbeat_payload_contains_runtime_state() {
        let config = ClusterConfig {
            enabled: true,
            node_id: "sip-a".to_string(),
            advertised_addr: "10.0.0.11:5060".to_string(),
            router_mode: RouterMode::Native,
            ..ClusterConfig::default()
        };
        let (sender, _receiver) = tokio::sync::mpsc::channel(1);
        let state = EdgeState::new(call_core::CallManager::new(
            call_core::RouteTable::default(),
            sender,
        ));

        let payload = heartbeat_payload(&config, &state, 1_720_000_000)
            .expect("heartbeat payload should serialize");

        assert!(payload.contains("\"node_id\":\"sip-a\""));
        assert!(payload.contains("\"router_mode\":\"native\""));
        assert!(payload.contains("\"status\":\"active\""));
        assert!(payload.contains("\"active_calls\":0"));
        assert!(payload.contains("\"started_at\":1720000000"));
        assert_eq!(
            node_key(&config.node_key_prefix, "sip-a"),
            "vos_rs:cluster:sip_nodes:sip-a"
        );
    }

    #[test]
    fn test_heartbeat_payload_reports_draining_state() {
        let config = ClusterConfig::default();
        let (sender, _receiver) = tokio::sync::mpsc::channel(1);
        let state = EdgeState::new(call_core::CallManager::new(
            call_core::RouteTable::default(),
            sender,
        ));
        state.draining.store(true, Ordering::Release);

        let payload = heartbeat_payload(&config, &state, 1).expect("heartbeat payload");

        assert!(payload.contains("\"status\":\"draining\""));
    }
}
