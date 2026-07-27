use crate::edge_state::EdgeState;
use sip_core::SipUri;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub(crate) struct CachedRegistrationLookup {
    pub(crate) contact: Option<crate::sip::registrar::RegistrationContact>,
    pub(crate) expires_at: Instant,
}

const POSITIVE_REGISTRATION_CACHE_TTL: Duration = Duration::from_secs(5);
const NEGATIVE_REGISTRATION_CACHE_TTL: Duration = Duration::from_secs(1);
const MAX_REGISTRATION_CACHE_ENTRIES: usize = 10_000;

impl EdgeState {
    /// 查找注册绑定的 Contact 地址。先查本地注册表，未命中再查 Redis。
    ///
    /// SIP 请求热路径不访问 PostgreSQL，避免数据库池等待拖慢所有呼叫。
    pub(crate) async fn lookup_contact(
        &self,
        uri: &SipUri,
    ) -> Option<crate::sip::registrar::RegistrationContact> {
        let contact = self.lookup_contact_internal(uri).await;
        if contact.is_some() {
            return contact;
        }
        if uri.port.is_some() {
            let mut uri_no_port = uri.clone();
            uri_no_port.port = None;
            return Box::pin(self.lookup_contact_internal(&uri_no_port)).await;
        }
        None
    }

    pub(crate) async fn lookup_contact_internal(
        &self,
        uri: &SipUri,
    ) -> Option<crate::sip::registrar::RegistrationContact> {
        let aor = crate::sip::registrar::canonical_aor(uri).ok()?;
        if let Some(cached) = self.cached_registration_lookup(&aor) {
            return cached;
        }

        let lookup_lock = self
            .registration_lookup_locks
            .entry(aor.clone())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone();
        let _guard = lookup_lock.lock().await;
        if let Some(cached) = self.cached_registration_lookup(&aor) {
            return cached;
        }

        let now = std::time::SystemTime::now();
        let mut result = self
            .registrar
            .read()
            .await
            .lookup_contact(uri, now, self.db_store.as_ref())
            .await;

        if result.is_none() {
            if let Some(mut conn) = self.redis_connection() {
                let redis_key = format!("vos_rs:reg:{aor}");
                let res: Result<String, redis::RedisError> = redis::cmd("GET")
                    .arg(&redis_key)
                    .query_async(&mut conn)
                    .await;
                if let Ok(json_str) = res {
                    if let Ok(contacts) = serde_json::from_str::<
                        Vec<crate::sip::registrar::RegistrationContact>,
                    >(&json_str)
                    {
                        result = contacts.into_iter().find(|contact| contact.expires > 0);
                    }
                }
            }
        }

        let ttl = if result.is_some() {
            POSITIVE_REGISTRATION_CACHE_TTL
        } else {
            NEGATIVE_REGISTRATION_CACHE_TTL
        };
        self.registration_lookup_cache.insert(
            aor,
            CachedRegistrationLookup {
                contact: result.clone(),
                expires_at: Instant::now() + ttl,
            },
        );
        self.prune_registration_lookup_cache();
        result
    }

    /// 使用号码库存解析被叫后查找注册 Contact。
    pub(crate) async fn lookup_destination_contact(
        &self,
        uri: &SipUri,
    ) -> Option<crate::sip::registrar::RegistrationContact> {
        let resolved = self.resolve_number_destination(uri);
        self.lookup_contact(&resolved).await
    }

    pub(crate) fn cached_registration_lookup(
        &self,
        aor: &str,
    ) -> Option<Option<crate::sip::registrar::RegistrationContact>> {
        let cached = self.registration_lookup_cache.get(aor)?;
        if cached.expires_at > Instant::now() {
            return Some(cached.contact.clone());
        }
        drop(cached);
        self.registration_lookup_cache.remove(aor);
        None
    }

    pub(crate) fn prune_registration_lookup_cache(&self) {
        if self.registration_lookup_cache.len() <= MAX_REGISTRATION_CACHE_ENTRIES {
            return;
        }
        let now = Instant::now();
        self.registration_lookup_cache
            .retain(|_, cached| cached.expires_at > now);
        self.registration_lookup_locks
            .retain(|aor, _| self.registration_lookup_cache.contains_key(aor));
    }

    pub(crate) fn invalidate_registration_lookup(&self, aor: &str) {
        self.registration_lookup_cache.remove(aor);
    }
}
