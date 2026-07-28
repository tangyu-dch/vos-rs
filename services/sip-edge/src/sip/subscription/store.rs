//! 订阅存储与生命周期管理。

use std::fmt;
use std::time::SystemTime;

use dashmap::DashMap;
use tokio::sync::RwLock;

use super::types::{
    default_expires_seconds, normalize_expires, EventPackage, Subscription, SubscriptionId,
    SubscriptionState,
};

/// 订阅操作错误。
#[derive(Debug, PartialEq, Eq)]
pub enum SubscriptionStoreError {
    /// SUBSCRIBE 缺少 `Event` 头或事件包不支持。
    UnsupportedEventPackage,
}

impl fmt::Display for SubscriptionStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedEventPackage => {
                formatter.write_str("missing or unsupported Event header")
            }
        }
    }
}

impl std::error::Error for SubscriptionStoreError {}

/// 订阅存储，按 `(event_package, aor)` 索引，便于状态变更时快速定位订阅者集合。
///
/// 内部使用 [`DashMap`] 分片并发，支持热路径读取；
/// 单个订阅的更新通过短期锁保护，避免长事务。
#[derive(Debug, Default)]
pub struct SubscriptionStore {
    /// 主索引：`(event_package, aor)` -> `SubscriptionId` 集合。
    by_aor: DashMap<(EventPackage, String), Vec<SubscriptionId>>,
    /// 订阅详情：`SubscriptionId` -> `Subscription`。
    by_id: DashMap<SubscriptionId, Subscription>,
    /// 序列化并发写访问（用于 `prune_expired` 等需要全局遍历的操作）。
    iteration_lock: RwLock<()>,
}

impl SubscriptionStore {
    /// 创建空存储。
    pub fn new() -> Self {
        Self::default()
    }

    /// 当前活跃订阅总数。
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    /// 是否为空。
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    /// 创建或刷新一条订阅。
    ///
    /// - 若 `expires_secs == 0`，视为终止订阅，调用方应使用 [`Self::remove`].
    /// - 若订阅 ID 已存在，则刷新有效期并更新 Contact/Route；否则插入新订阅。
    ///
    /// 返回值：旧订阅（若是刷新），便于调用方判断是否需要立即发送 NOTIFY。
    pub fn upsert(&self, subscription: Subscription) -> Option<Subscription> {
        let key = (subscription.event_package, subscription.aor.clone());
        let id = subscription.id.clone();

        let previous = self.by_id.insert(id.clone(), subscription.clone());

        let mut ids = self.by_aor.entry(key).or_default();
        if !ids.iter().any(|existing| existing == &id) {
            ids.push(id);
        }

        previous
    }

    /// 移除一条订阅，返回被移除的订阅（若存在）。
    pub fn remove(&self, id: &SubscriptionId) -> Option<Subscription> {
        let removed = self.by_id.remove(id)?.1;

        let key = (removed.event_package, removed.aor.clone());
        if let Some(mut ids) = self.by_aor.get_mut(&key) {
            ids.retain(|existing| existing != id);
            if ids.is_empty() {
                drop(ids);
                self.by_aor.remove(&key);
            }
        }

        Some(removed)
    }

    /// 返回某个 AOR 上指定事件包的全部活跃订阅。
    #[allow(dead_code)]
    pub fn subscribers_for(&self, event_package: EventPackage, aor: &str) -> Vec<Subscription> {
        let key = (event_package, aor.to_string());
        let Some(ids) = self.by_aor.get(&key) else {
            return Vec::new();
        };
        ids.iter()
            .filter_map(|id| self.by_id.get(id).map(|entry| entry.clone()))
            .collect()
    }

    /// 移除所有已过期的订阅，返回被移除的订阅列表（用于发送 terminated NOTIFY）。
    pub async fn prune_expired(&self, now: SystemTime) -> Vec<Subscription> {
        let _guard = self.iteration_lock.write().await;
        let expired_ids: Vec<SubscriptionId> = self
            .by_id
            .iter()
            .filter(|entry| entry.is_expired(now))
            .map(|entry| entry.id.clone())
            .collect();

        expired_ids
            .into_iter()
            .filter_map(|id| self.remove(&id))
            .collect()
    }

    /// 获取某条订阅的当前状态头值（基于剩余有效期）。
    #[allow(dead_code)]
    pub fn state_for(&self, id: &SubscriptionId, now: SystemTime) -> Option<SubscriptionState> {
        let subscription = self.by_id.get(id)?;
        let remaining = subscription.remaining_seconds(now);
        if remaining == 0 {
            Some(SubscriptionState::Terminated {
                reason: Some("timeout"),
            })
        } else {
            Some(SubscriptionState::Active { expires: remaining })
        }
    }
}

/// 从 SUBSCRIBE 请求中提取订阅元数据，规范化 `Expires` 值。
///
/// 调用方在调用此函数前应已校验：
/// - 请求方法为 SUBSCRIBE
/// - 已通过 SBC 鉴权
///
/// 返回值：`(规范化后的 expires, 事件包, 订阅 ID)`，订阅 ID 由 `Call-ID + From-tag` 拼接。
pub fn parse_subscribe_request(
    call_id: &str,
    from_tag: &str,
    event_header: Option<&str>,
    expires_header: Option<&str>,
) -> Result<(u32, EventPackage, SubscriptionId), SubscriptionStoreError> {
    let event_str = event_header.ok_or(SubscriptionStoreError::UnsupportedEventPackage)?;
    let event_package = EventPackage::from_header(event_str)
        .ok_or(SubscriptionStoreError::UnsupportedEventPackage)?;

    let requested = expires_header
        .and_then(|value| value.trim().parse::<u32>().ok())
        .unwrap_or_else(default_expires_seconds);

    let normalized = normalize_expires(requested);
    let subscription_id = SubscriptionId::new(format!("{call_id}-{from_tag}"));
    Ok((normalized, event_package, subscription_id))
}

/// 测试辅助：用于单元测试的存储构造。
#[cfg(test)]
impl SubscriptionStore {
    pub fn contains(&self, id: &SubscriptionId) -> bool {
        self.by_id.contains_key(id)
    }

    pub fn subscriber_count(&self, event_package: EventPackage, aor: &str) -> usize {
        self.subscribers_for(event_package, aor).len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use std::time::Duration;

    fn dummy_subscription(
        id: &str,
        aor: &str,
        package: EventPackage,
        expires: u32,
    ) -> Subscription {
        Subscription {
            id: SubscriptionId::new(id),
            aor: aor.to_string(),
            event_package: package,
            contact_uri: "sip:watcher@10.0.0.1:5060".to_string(),
            peer: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)), 5060),
            from_uri: "sip:watcher@example.com".to_string(),
            to_uri: format!("sip:{aor}"),
            dialog_call_id: format!("call-{id}"),
            local_tag: "local".to_string(),
            remote_tag: Some("remote".to_string()),
            route_set: Vec::new(),
            last_cseq: 0,
            expires_at: SystemTime::now() + Duration::from_secs(expires as u64),
        }
    }

    #[test]
    fn upsert_inserts_new_subscription() {
        let store = SubscriptionStore::new();
        let sub = dummy_subscription("s1", "1001@example.com", EventPackage::Presence, 600);

        assert!(store.upsert(sub).is_none());
        assert_eq!(store.len(), 1);
        assert_eq!(
            store.subscriber_count(EventPackage::Presence, "1001@example.com"),
            1
        );
    }

    #[test]
    fn upsert_refreshes_existing_subscription() {
        let store = SubscriptionStore::new();
        let original = dummy_subscription("s1", "1001@example.com", EventPackage::Presence, 60);
        store.upsert(original.clone());

        let mut refreshed = original.clone();
        refreshed.expires_at = SystemTime::now() + Duration::from_secs(3600);
        let previous = store.upsert(refreshed);

        assert_eq!(previous, Some(original));
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn remove_drops_subscription_and_index() {
        let store = SubscriptionStore::new();
        let sub = dummy_subscription("s1", "1001@example.com", EventPackage::Presence, 600);
        store.upsert(sub.clone());

        let removed = store.remove(&sub.id);
        assert_eq!(removed.as_ref(), Some(&sub));
        assert!(store.is_empty());
        assert_eq!(
            store.subscriber_count(EventPackage::Presence, "1001@example.com"),
            0
        );
    }

    #[tokio::test]
    async fn prune_expired_returns_and_removes_expired_subscriptions() {
        let store = SubscriptionStore::new();
        let live = dummy_subscription("s1", "1001@example.com", EventPackage::Presence, 600);
        let expired = dummy_subscription("s2", "1002@example.com", EventPackage::Presence, 0);
        store.upsert(live.clone());
        store.upsert(expired.clone());

        let now = expired.expires_at + Duration::from_secs(1);
        let pruned = store.prune_expired(now).await;

        assert_eq!(pruned.len(), 1);
        assert_eq!(pruned[0].id, expired.id);
        assert_eq!(store.len(), 1);
        assert!(store.contains(&live.id));
    }

    #[test]
    fn parse_subscribe_request_normalizes_expires() {
        let (expires, package, id) =
            parse_subscribe_request("call-1", "from-tag", Some("presence"), Some("999999"))
                .expect("valid subscribe");

        assert_eq!(expires, super::super::types::MAX_EXPIRES_SECONDS);
        assert_eq!(package, EventPackage::Presence);
        assert_eq!(id.as_str(), "call-1-from-tag");
    }

    #[test]
    fn parse_subscribe_request_rejects_unknown_event() {
        let result = parse_subscribe_request("call-1", "from-tag", Some("unknown"), None);
        assert_eq!(
            result.err(),
            Some(SubscriptionStoreError::UnsupportedEventPackage)
        );
    }

    #[test]
    fn parse_subscribe_request_uses_default_when_expires_missing() {
        let (expires, _, _) = parse_subscribe_request("call-1", "from-tag", Some("dialog"), None)
            .expect("valid subscribe");
        assert_eq!(expires, default_expires_seconds());
    }
}
