use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    sync::Arc,
    time::{Duration, Instant},
};

use dashmap::DashMap;
use redis::AsyncCommands;

use crate::{discovery::SipNode, metrics, proxy::select_node};

const DIALOG_ROUTE_KEY_PREFIX: &str = "vos_rs:cluster:sip_dialog_routes";

#[derive(Debug, Clone)]
struct CachedRoute {
    node_id: String,
    expires_at: Instant,
    refresh_after: Instant,
}

pub(crate) struct DialogRouteStore {
    redis: Option<redis::aio::ConnectionManager>,
    ttl_secs: u64,
    local: DashMap<String, CachedRoute>,
}

impl DialogRouteStore {
    pub(crate) fn new(redis: redis::aio::ConnectionManager, ttl_secs: u64) -> Arc<Self> {
        Arc::new(Self {
            redis: Some(redis),
            ttl_secs: ttl_secs.max(60),
            local: DashMap::new(),
        })
    }

    pub(crate) async fn resolve(
        &self,
        call_id: &str,
        nodes: &[SipNode],
    ) -> Result<SipNode, Box<dyn std::error::Error + Send + Sync>> {
        let now = Instant::now();
        if let Some(route) = self.local.get(call_id) {
            if route.expires_at > now {
                if let Some(node) = nodes.iter().find(|node| node.id == route.node_id) {
                    let node = node.clone();
                    if route.refresh_after > now {
                        return Ok(node);
                    }
                    drop(route);
                    self.cache(call_id, &node);

                    if let Some(mut redis) = self.redis.clone() {
                        let call_id_str = call_id.to_string();
                        let node_id_str = node.id.clone();
                        let ttl_secs = self.ttl_secs;
                        tokio::spawn(async move {
                            let script_result: Result<(), redis::RedisError> = async move {
                                let _: i64 = redis::Script::new(
                                    "if redis.call('GET', KEYS[1]) == ARGV[1] then \
                                       return redis.call('EXPIRE', KEYS[1], ARGV[2]); \
                                     end; return 0",
                                )
                                .key(route_key(&call_id_str))
                                .arg(&node_id_str)
                                .arg(ttl_secs)
                                .invoke_async(&mut redis)
                                .await?;
                                Ok(())
                            }
                            .await;
                            if let Err(error) = script_result {
                                metrics::redis_error();
                                tracing::warn!(%error, "Redis 对话归属续期失败 (后台任务)");
                            }
                        });
                    }
                    return Ok(node);
                }
            }
        }

        let candidate = select_node(call_id, nodes)
            .cloned()
            .ok_or("没有可用的 sip-edge 节点")?;
        let Some(mut redis) = self.redis.clone() else {
            self.cache(call_id, &candidate);
            return Ok(candidate);
        };
        let key = route_key(call_id);
        let stored: Option<String> = match redis.get(&key).await {
            Ok(stored) => stored,
            Err(error) => {
                metrics::redis_error();
                tracing::warn!(%error, "Redis 对话归属读取失败，使用确定性本地选路");
                self.cache(call_id, &candidate);
                return Ok(candidate);
            }
        };
        let node = match stored {
            Some(owner) => match nodes.iter().find(|node| node.id == owner) {
                Some(node) => node.clone(),
                None => self
                    .replace_owner(&mut redis, &key, &owner, &candidate, nodes)
                    .await
                    .unwrap_or_else(|error| {
                        metrics::redis_error();
                        tracing::warn!(%error, "替换失效对话归属失败，使用确定性本地选路");
                        candidate.clone()
                    }),
            },
            None => self
                .claim_owner(&mut redis, &key, &candidate, nodes)
                .await
                .unwrap_or_else(|error| {
                    metrics::redis_error();
                    tracing::warn!(%error, "创建对话归属失败，使用确定性本地选路");
                    candidate.clone()
                }),
        };
        self.cache(call_id, &node);
        Ok(node)
    }

    async fn claim_owner(
        &self,
        redis: &mut redis::aio::ConnectionManager,
        key: &str,
        candidate: &SipNode,
        nodes: &[SipNode],
    ) -> Result<SipNode, redis::RedisError> {
        let inserted: bool = redis::cmd("SET")
            .arg(key)
            .arg(&candidate.id)
            .arg("NX")
            .arg("EX")
            .arg(self.ttl_secs)
            .query_async(redis)
            .await?;
        if inserted {
            return Ok(candidate.clone());
        }
        let winner: Option<String> = redis.get(key).await?;
        Ok(winner
            .and_then(|id| nodes.iter().find(|node| node.id == id).cloned())
            .unwrap_or_else(|| candidate.clone()))
    }

    async fn replace_owner(
        &self,
        redis: &mut redis::aio::ConnectionManager,
        key: &str,
        stale_owner: &str,
        candidate: &SipNode,
        nodes: &[SipNode],
    ) -> Result<SipNode, redis::RedisError> {
        let owner: Option<String> = redis::Script::new(
            "local current = redis.call('GET', KEYS[1]); \
             if current == ARGV[1] then \
               redis.call('SET', KEYS[1], ARGV[2], 'EX', ARGV[3]); return ARGV[2]; \
             end; return current",
        )
        .key(key)
        .arg(stale_owner)
        .arg(&candidate.id)
        .arg(self.ttl_secs)
        .invoke_async(redis)
        .await?;
        Ok(owner
            .and_then(|id| nodes.iter().find(|node| node.id == id).cloned())
            .unwrap_or_else(|| candidate.clone()))
    }

    /// 在对话完成后删除本地与 Redis 归属，避免无效键保留到 TTL 到期。
    pub(crate) fn release(&self, call_id: &str) {
        self.local.remove(call_id);
        let Some(mut redis) = self.redis.clone() else {
            return;
        };
        let key = route_key(call_id);
        tokio::spawn(async move {
            if let Err(error) = redis.del::<_, usize>(key).await {
                metrics::redis_error();
                tracing::warn!(%error, "删除已完成对话归属失败 (后台任务)");
            }
        });
    }

    fn cache(&self, call_id: &str, node: &SipNode) {
        self.local.insert(
            call_id.to_string(),
            CachedRoute {
                node_id: node.id.clone(),
                expires_at: Instant::now() + Duration::from_secs(self.ttl_secs),
                refresh_after: Instant::now() + Duration::from_secs((self.ttl_secs / 2).max(1)),
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
    async fn test_local_route_moves_when_owner_leaves_active_nodes() {
        let store = DialogRouteStore::without_redis_for_test(60);
        let first_nodes = vec![
            SipNode {
                id: "sip-a".to_string(),
                address: "127.0.0.1:5061".parse().expect("address"),
            },
            SipNode {
                id: "sip-b".to_string(),
                address: "127.0.0.1:5062".parse().expect("address"),
            },
        ];
        let owner = store
            .resolve("call-move", &first_nodes)
            .await
            .expect("owner");
        let remaining = first_nodes
            .into_iter()
            .filter(|node| node.id != owner.id)
            .collect::<Vec<_>>();

        let replacement = store
            .resolve("call-move", &remaining)
            .await
            .expect("replacement");

        assert_ne!(replacement.id, owner.id);
    }

    #[tokio::test]
    async fn test_release_removes_local_route() {
        let store = DialogRouteStore::without_redis_for_test(60);
        let nodes = vec![SipNode {
            id: "sip-a".to_string(),
            address: "127.0.0.1:5061".parse().expect("address"),
        }];
        store.resolve("call-release", &nodes).await.expect("owner");
        assert!(store.local.contains_key("call-release"));

        store.release("call-release");

        assert!(!store.local.contains_key("call-release"));
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
        let _: () = cleanup
            .set_ex(&key, "dead-node", 60)
            .await
            .expect("seed stale owner");
        let first_redis = redis::aio::ConnectionManager::new(client.clone())
            .await
            .expect("first redis manager");
        let second_redis = redis::aio::ConnectionManager::new(client)
            .await
            .expect("second redis manager");
        let first = DialogRouteStore::new(first_redis, 60);
        let second = DialogRouteStore::new(second_redis, 60);
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
        let stored_owner: String = cleanup.get(&key).await.expect("stored owner");
        assert_eq!(stored_owner, first_owner.id);
        first.release(call_id);

        let mut exists = true;
        for _ in 0..10 {
            exists = cleanup.exists(&key).await.expect("route exists");
            if !exists {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        assert!(!exists);
    }
}
