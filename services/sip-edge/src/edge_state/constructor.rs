//! # EdgeState 构造与依赖注入
//!
//! 本模块扩展 [`EdgeState`][super::EdgeState]，提供：
//!
//! - 构造函数（`new` / `with_media_relay_and_db` / `with_config`）
//! - 一次性注入器（Redis / NATS / VoiceEngine / RegistrationSync / ClusterEgress）
//! - 对应的访问器

use std::sync::Arc;

use call_core::CallManager;

use crate::cluster::{ClusterEgress, RegistrationSyncSender};
use crate::config::EdgeConfig;
use crate::media::MediaRelayState;
use crate::sbc;
use crate::sip::handlers::ivr_topology::VoiceEngineManager;
use crate::sip::registrar::RegistrationStore;
use crate::sip::subscription::SubscriptionStore;

use super::gateway_identity::GatewayIdentityCache;
use super::models::CallSessionStore;
use super::EdgeState;

impl EdgeState {
    #[cfg(test)]
    pub(crate) fn new(call_manager: CallManager) -> Self {
        Self::with_media_relay_and_db(
            call_manager,
            MediaRelayState::new(),
            None,
            &EdgeConfig::default(),
        )
    }

    pub(crate) fn with_media_relay_and_db(
        call_manager: CallManager,
        media_relay: MediaRelayState,
        db_store: Option<cdr_core::PostgresCdrStore>,
        config: &EdgeConfig,
    ) -> Self {
        let sbc_engine = sbc::SbcEngine::new(
            &config
                .sbc_allow_rules
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>(),
            &config
                .sbc_block_rules
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>(),
            config.sbc_rate_limit_capacity,
            config.sbc_rate_limit_fill_rate,
        );

        Self {
            call_manager: std::sync::Arc::new(call_manager),
            cdr_pipeline_metrics: std::sync::OnceLock::new(),
            gateway_health: call_core::GatewayHealthTracker::new(
                call_core::HealthThresholds::default(),
            ),
            inbound_transactions: CallSessionStore::default(),
            media_relay,
            registrar: tokio::sync::RwLock::new(RegistrationStore::new()),
            subscription_store: SubscriptionStore::new(),
            db_store,
            client_transactions: crate::sip::client_transaction::ClientTransactionManager::new(),
            recent_inbound_invites: dashmap::DashMap::new(),
            draining: std::sync::atomic::AtomicBool::new(false),
            server_transactions: dashmap::DashMap::new(),
            invite_ack_transactions: dashmap::DashMap::new(),
            socket: std::sync::OnceLock::new(),
            test_request_cache: dashmap::DashMap::new(),
            nonce_replay_cache: dashmap::DashMap::new(),
            tcp_connections: dashmap::DashMap::new(),
            sbc_engine,
            sbc_rate_limit_enabled: config.sbc_rate_limit_enabled,
            gateway_cache: std::sync::RwLock::new(GatewayIdentityCache::default()),
            access_trunk_auth_modes: std::sync::RwLock::new(std::collections::HashMap::new()),
            access_username_to_trunk_id: std::sync::RwLock::new(std::collections::HashMap::new()),
            trunk_billing_accounts: std::sync::RwLock::new(std::collections::HashMap::new()),
            did_destinations: std::sync::RwLock::new(std::collections::HashMap::new()),
            extension_groups: std::sync::RwLock::new(std::collections::HashMap::new()),
            ivr_menus: std::sync::RwLock::new(std::collections::HashMap::new()),
            outbound_registrations: dashmap::DashMap::new(),
            access_ip_rules: std::sync::RwLock::new(Vec::new()),
            registered_access_users: std::sync::RwLock::new(Vec::new()),
            user_concurrency: dashmap::DashMap::new(),
            anti_fraud_rules: std::sync::RwLock::new(Vec::new()),
            media_metrics_log: config.media_metrics_log,
            billing_settlement_enabled: config.billing_settlement_enabled,
            gateway_health_persistence_enabled: config.gateway_health_checks_enabled,
            gateway_probes: dashmap::DashMap::new(),
            redis_conn: std::sync::OnceLock::new(),
            registration_sync: std::sync::OnceLock::new(),
            cluster_egress: std::sync::OnceLock::new(),
            registration_lookup_cache: dashmap::DashMap::new(),
            registration_lookup_locks: dashmap::DashMap::new(),
            parked_calls: std::sync::Arc::new(dashmap::DashMap::new()),
            nats_client: std::sync::OnceLock::new(),
            #[cfg(test)]
            test_gateways: std::sync::Mutex::new(Vec::new()),
            self_weak: std::sync::OnceLock::new(),
            sipflow_enabled: std::sync::atomic::AtomicBool::new(config.sipflow_enabled),
            sipflow_whitelist: std::sync::RwLock::new(config.sipflow_whitelist.clone()),
            sipflow_retention_days: std::sync::atomic::AtomicI32::new(
                config.sipflow_retention_days,
            ),
            sip_flow_tx: std::sync::OnceLock::new(),
            call_caller_addrs: dashmap::DashMap::new(),
            matched_call_ids: dashmap::DashMap::new(),
            voice_engine: std::sync::OnceLock::new(),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_config(call_manager: CallManager, config: &EdgeConfig) -> Self {
        Self::with_media_relay_and_db(call_manager, MediaRelayState::new(), None, config)
    }

    /// 设置 Redis 连接（仅在启动阶段调用一次）。
    pub(crate) fn set_redis(&self, conn: redis::aio::ConnectionManager) {
        let _ = self.redis_conn.set(conn);
    }

    /// 获取 Redis 连接管理器的克隆，各请求可并发发送命令并共享重连状态。
    pub(crate) fn redis_connection(&self) -> Option<redis::aio::ConnectionManager> {
        self.redis_conn.get().cloned()
    }

    pub(crate) fn set_nats(&self, conn: async_nats::Client) {
        let _ = self.nats_client.set(conn);
    }

    pub(crate) fn nats_connection(&self) -> Option<async_nats::Client> {
        self.nats_client.get().cloned()
    }

    /// 注入 IVR TTS/ASR 引擎管理器 (仅在启动阶段调用一次)。
    pub(crate) fn set_voice_engine(&self, manager: Arc<VoiceEngineManager>) {
        let _ = self.voice_engine.set(manager);
    }

    /// 获取共享的 TTS/ASR 引擎管理器 (未注入时返回 None)。
    pub(crate) fn voice_engine(&self) -> Option<Arc<VoiceEngineManager>> {
        self.voice_engine.get().cloned()
    }

    pub(crate) fn set_registration_sync(&self, sender: RegistrationSyncSender) {
        let _ = self.registration_sync.set(sender);
    }

    pub(crate) fn registration_sync(&self) -> Option<RegistrationSyncSender> {
        self.registration_sync.get().cloned()
    }

    pub(crate) fn set_cluster_egress(&self, egress: ClusterEgress) {
        let _ = self.cluster_egress.set(egress);
    }
}
