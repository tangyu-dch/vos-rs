use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    sync::Arc,
    time::{Duration, Instant},
};

use dashmap::DashMap;
use redis::AsyncCommands;

use crate::{discovery::SipNode, proxy::select_node};

const DIALOG_ROUTE_KEY_PREFIX: &str = "vos_rs:cluster:sip_dialog_routes";

#[derive(Debug, Clone)]
struct CachedRoute {
    node_id: String,
    expires_at: Instant,
}

pub(crate) struct DialogRouteStore {
    redis: Option<redis::aio::MultiplexedConnection>,
    ttl_secs: u64,
    local: DashMap<String, CachedRoute>,
}

impl DialogRouteStore {
    pub(crate) async fn new(
        client: redis::Client,
        ttl_secs: u64,
    ) -> Result<Arc<Self>, redis::RedisError> {
        let redis = client.get_multiplexed_tokio_connection().await?;
        Ok(Arc::new(Self {
            redis: Some(redis),
            ttl_secs: ttl_secs.max(60),
            local: DashMap::new(),
        }))
    }

    pub(crate) async fn resolve(
        &self,
        call_id: &str,
        nodes: &[SipNode],
    ) -> Result<SipNode, Box<dyn std::error::Error + Send + Sync>> {
        if let Some(route) = self.local.get(call_id) {
            if route.expires_at > Instant::now() {
                if let Some(node) = nodes.iter().find(|node| node.id == route.node_id) {
                    return Ok(node.clone());
                }
            }
        }

        let Some(mut redis) = self.redis.clone() else {
            let node = select_node(call_id, nodes)
                .cloned()
                .ok_or("没有可用的 sip-edge 节点")?;
            self.cache(call_id, &node);
            return Ok(node);
        };
        let key = route_key(call_id);
        let stored: Option<String> = redis.get(&key).await?;
        let node = if let Some(node) = stored.and_then(|id| nodes.iter().find(|node| node.id == id))
        {
            node.clone()
        } else {
            let candidate = select_node(call_id, nodes)
                .cloned()
                .ok_or("没有可用的 sip-edge 节点")?;
            let inserted: bool = redis::cmd("SET")
                .arg(&key)
                .arg(&candidate.id)
                .arg("NX")
                .arg("EX")
                .arg(self.ttl_secs)
                .query_async(&mut redis)
                .await?;
            if inserted {
                candidate
            } else {
                let winner: Option<String> = redis.get(&key).await?;
                winner
                    .and_then(|id| nodes.iter().find(|node| node.id == id).cloned())
                    .unwrap_or(candidate)
            }
        };
        self.cache(call_id, &node);
        Ok(node)
    }

    fn cache(&self, call_id: &str, node: &SipNode) {
        self.local.insert(
            call_id.to_string(),
            CachedRoute {
                node_id: node.id.clone(),
                expires_at: Instant::now() + Duration::from_secs(self.ttl_secs),
            },
        );
    }

    #[cfg(test)]
    pub(crate) fn without_redis_for_test(ttl_secs: u64) -> Arc<Self> {
        Arc::new(Self {
            redis: None,
            ttl_secs,
            local: DashMap::new(),
        })
    }
}

fn route_key(call_id: &str) -> String {
    let mut left = DefaultHasher::new();
    "left".hash(&mut left);
    call_id.hash(&mut left);
    let mut right = DefaultHasher::new();
    "right".hash(&mut right);
    call_id.hash(&mut right);
    format!(
        "{DIALOG_ROUTE_KEY_PREFIX}:{:016x}{:016x}",
        left.finish(),
        right.finish()
    )
}

#[cfg(test)]
mod tests {
    use super::{route_key, DialogRouteStore};
    use crate::discovery::SipNode;
    use redis::AsyncCommands;

    #[test]
    fn test_route_key_is_stable_and_does_not_embed_call_id() {
        let key = route_key("sensitive-call-id@example.com");
        assert_eq!(key, route_key("sensitive-call-id@example.com"));
        assert!(!key.contains("sensitive-call-id"));
    }

    #[tokio::test]
    #[ignore = "需要本机 Redis 6379"]
    async fn test_two_router_instances_share_dialog_owner_in_redis() {
        let client = redis::Client::open("redis://127.0.0.1:6379/0").expect("redis client");
        let call_id = "sip-router-shared-route-test";
        let key = route_key(call_id);
        let mut cleanup = client
            .get_multiplexed_tokio_connection()
            .await
            .expect("redis connection");
        let _: usize = cleanup.del(&key).await.expect("clear route");
        let first = DialogRouteStore::new(client.clone(), 60)
            .await
            .expect("first store");
        let second = DialogRouteStore::new(client, 60)
            .await
            .expect("second store");
        let nodes = vec![
            SipNode {
                id: "sip-a".to_string(),
                address: "127.0.0.1:5061".parse().expect("address"),
            },
            SipNode {
                id: "sip-b".to_string(),
                address: "127.0.0.1:5062".parse().expect("address"),
            },
        ];

        let first_owner = first.resolve(call_id, &nodes).await.expect("first owner");
        let second_owner = second.resolve(call_id, &nodes).await.expect("second owner");

        assert_eq!(first_owner, second_owner);
        let _: usize = cleanup.del(key).await.expect("cleanup route");
    }
}
