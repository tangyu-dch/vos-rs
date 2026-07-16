use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};

use dashmap::DashMap;

use crate::cluster::{MediaAllocationStrategy, MediaClusterConfig, MediaNodeConfig, MediaNodeType};

/// 媒体节点的运行时状态。
pub(crate) struct MediaNodeRuntime {
    pub(crate) config: MediaNodeConfig,
    pub(crate) client: reqwest::Client,
    healthy: AtomicBool,
    consecutive_failures: AtomicU32,
    active_endpoints: AtomicU64,
}

impl MediaNodeRuntime {
    fn new(config: MediaNodeConfig) -> Self {
        Self {
            config,
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(2))
                .build()
                .unwrap_or_default(),
            healthy: AtomicBool::new(true),
            consecutive_failures: AtomicU32::new(0),
            active_endpoints: AtomicU64::new(0),
        }
    }

    pub(crate) fn is_uds(&self) -> bool {
        self.config.control_url.as_deref().is_some_and(|url| {
            url.starts_with("uds://") || url.starts_with('/') || url.ends_with(".sock")
        })
    }

    pub(crate) fn uds_path(&self) -> Option<&str> {
        self.config
            .control_url
            .as_deref()
            .map(|url| url.trim_start_matches("uds://"))
    }

    pub(crate) fn is_local(&self) -> bool {
        self.config.node_type == MediaNodeType::Local
    }

    pub(crate) fn control_url(&self) -> Option<&str> {
        self.config.control_url.as_deref()
    }

    pub(crate) fn is_healthy(&self) -> bool {
        self.healthy.load(Ordering::Relaxed)
    }
}

/// 带健康摘除和 Call-ID 亲和的媒体节点池。
pub(crate) struct MediaNodePool {
    nodes: Vec<Arc<MediaNodeRuntime>>,
    strategy: MediaAllocationStrategy,
    unhealthy_threshold: u32,
    cursor: AtomicU64,
    call_nodes: DashMap<String, usize>,
    call_endpoint_counts: DashMap<String, u32>,
    port_nodes: DashMap<u16, usize>,
    port_calls: DashMap<u16, String>,
}

impl MediaNodePool {
    pub(crate) fn new(config: &MediaClusterConfig) -> Arc<Self> {
        let pool = Arc::new(Self {
            nodes: config
                .nodes
                .iter()
                .cloned()
                .map(MediaNodeRuntime::new)
                .map(Arc::new)
                .collect(),
            strategy: config.allocation_strategy,
            unhealthy_threshold: config.unhealthy_threshold.max(1),
            cursor: AtomicU64::new(0),
            call_nodes: DashMap::new(),
            call_endpoint_counts: DashMap::new(),
            port_nodes: DashMap::new(),
            port_calls: DashMap::new(),
        });
        Self::spawn_health_checks(&pool, config.health_check_interval_secs.max(1));
        pool
    }

    pub(crate) fn node_for_call(&self, call_id: &str) -> Option<(usize, Arc<MediaNodeRuntime>)> {
        if let Some(index) = self.call_nodes.get(call_id).map(|entry| *entry) {
            let node = Arc::clone(self.nodes.get(index)?);
            // 已分配呼叫不能静默迁移，否则两条媒体腿会落到不同节点。
            return Some((index, node));
        }

        let index = match self.strategy {
            MediaAllocationStrategy::WeightedRoundRobin => self.select_weighted_round_robin()?,
            MediaAllocationStrategy::LeastSessions => self.select_least_sessions()?,
            MediaAllocationStrategy::CallIdHash => self.select_call_id_hash(call_id)?,
        };
        self.call_nodes.insert(call_id.to_string(), index);
        Some((index, Arc::clone(&self.nodes[index])))
    }

    pub(crate) fn record_allocation(&self, call_id: &str, node_index: usize, port: u16) {
        self.port_nodes.insert(port, node_index);
        self.port_calls.insert(port, call_id.to_string());
        self.call_endpoint_counts
            .entry(call_id.to_string())
            .and_modify(|count| *count = count.saturating_add(1))
            .or_insert(1);
        self.nodes[node_index]
            .active_endpoints
            .fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn node_for_port(&self, port: u16) -> Option<Arc<MediaNodeRuntime>> {
        let rtp_port = if port % 2 == 0 { port } else { port - 1 };
        let index = self.port_nodes.get(&rtp_port).map(|entry| *entry)?;
        self.nodes.get(index).map(Arc::clone)
    }

    pub(crate) fn release_port(&self, port: u16) {
        let rtp_port = if port % 2 == 0 { port } else { port - 1 };
        if let Some((_, index)) = self.port_nodes.remove(&rtp_port) {
            self.nodes[index]
                .active_endpoints
                .fetch_sub(1, Ordering::Relaxed);
        }
        let Some((_, call_id)) = self.port_calls.remove(&rtp_port) else {
            return;
        };
        if let Some(mut count) = self.call_endpoint_counts.get_mut(&call_id) {
            *count = count.saturating_sub(1);
            if *count > 0 {
                return;
            }
            drop(count);
            self.call_endpoint_counts.remove(&call_id);
            self.call_nodes.remove(&call_id);
        }
    }

    pub(crate) fn cancel_unallocated_call(&self, call_id: &str) {
        if !self.call_endpoint_counts.contains_key(call_id) {
            self.call_nodes.remove(call_id);
        }
    }

    pub(crate) fn nodes(&self) -> &[Arc<MediaNodeRuntime>] {
        &self.nodes
    }

    fn healthy_indices(&self) -> impl Iterator<Item = usize> + '_ {
        self.nodes
            .iter()
            .enumerate()
            .filter_map(|(index, node)| node.is_healthy().then_some(index))
    }

    fn select_weighted_round_robin(&self) -> Option<usize> {
        let total_weight: u64 = self
            .healthy_indices()
            .map(|index| u64::from(self.nodes[index].config.weight))
            .sum();
        if total_weight == 0 {
            return None;
        }
        let mut position = self.cursor.fetch_add(1, Ordering::Relaxed) % total_weight;
        for index in self.healthy_indices() {
            let weight = u64::from(self.nodes[index].config.weight);
            if position < weight {
                return Some(index);
            }
            position -= weight;
        }
        None
    }

    fn select_least_sessions(&self) -> Option<usize> {
        self.healthy_indices()
            .min_by_key(|index| self.nodes[*index].active_endpoints.load(Ordering::Relaxed))
    }

    fn select_call_id_hash(&self, call_id: &str) -> Option<usize> {
        self.healthy_indices().max_by_key(|index| {
            let mut hasher = DefaultHasher::new();
            call_id.hash(&mut hasher);
            self.nodes[*index].config.id.hash(&mut hasher);
            hasher.finish()
        })
    }

    fn spawn_health_checks(pool: &Arc<Self>, interval_secs: u64) {
        let weak_pool = Arc::downgrade(pool);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(interval_secs));
            loop {
                interval.tick().await;
                let Some(pool) = weak_pool.upgrade() else {
                    break;
                };
                for node in &pool.nodes {
                    if node.is_local() || node.is_uds() {
                        continue;
                    }
                    let healthy = match node.control_url() {
                        Some(control_url) => node
                            .client
                            .get(format!("{control_url}/health"))
                            .send()
                            .await
                            .is_ok_and(|response| response.status().is_success()),
                        None => false,
                    };
                    if healthy {
                        node.consecutive_failures.store(0, Ordering::Relaxed);
                        node.healthy.store(true, Ordering::Relaxed);
                    } else {
                        let failures = node
                            .consecutive_failures
                            .fetch_add(1, Ordering::Relaxed)
                            .saturating_add(1);
                        if failures >= pool.unhealthy_threshold {
                            node.healthy.store(false, Ordering::Relaxed);
                        }
                    }
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str, weight: u16, port_min: u16) -> MediaNodeConfig {
        MediaNodeConfig {
            id: id.to_string(),
            node_type: MediaNodeType::Remote,
            control_url: Some(format!("uds:///tmp/{id}.sock")),
            advertised_addr: "127.0.0.1".to_string(),
            port_min,
            port_max: port_min + 98,
            weight,
            control_token: String::new(),
        }
    }

    #[tokio::test]
    async fn test_call_allocations_keep_node_affinity_until_all_ports_release() {
        let config = MediaClusterConfig {
            nodes: vec![node("a", 1, 40_000), node("b", 1, 41_000)],
            ..MediaClusterConfig::default()
        };
        let pool = MediaNodePool::new(&config);
        let (index, first) = pool.node_for_call("call-1").expect("node");
        pool.record_allocation("call-1", index, first.config.port_min);
        let (_, second) = pool.node_for_call("call-1").expect("same node");

        assert_eq!(first.config.id, second.config.id);
        assert_eq!(
            pool.node_for_port(first.config.port_min)
                .expect("port owner")
                .config
                .id,
            first.config.id
        );
        pool.release_port(first.config.port_min);
        assert!(pool.node_for_port(first.config.port_min).is_none());
    }

    #[tokio::test]
    async fn test_local_and_remote_nodes_share_the_same_scheduler() {
        let mut local = node("local", 1, 40_000);
        local.node_type = MediaNodeType::Local;
        local.control_url = None;
        let config = MediaClusterConfig {
            nodes: vec![local, node("remote", 1, 41_000)],
            ..MediaClusterConfig::default()
        };
        let pool = MediaNodePool::new(&config);

        let (_, first) = pool.node_for_call("call-a").expect("first node");
        let (_, second) = pool.node_for_call("call-b").expect("second node");

        assert_ne!(first.config.node_type, second.config.node_type);
    }
}
