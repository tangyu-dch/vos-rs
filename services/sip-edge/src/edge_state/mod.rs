//! # EdgeState：sip-edge 全局共享状态
//!
//! [`EdgeState`] 是 sip-edge 进程的全局状态容器，被所有 SIP/媒体/API 子系统通过
//! `Arc<EdgeState>` 共享。
//!
//! ## 子模块组织
//!
//! | 模块 | 职责 |
//! |------|------|
//! | [`models`] | 公共数据模型（`InboundTransaction` / `CallSessionStore` / `PendingDatagram` 等） |
//! | [`types`] | 辅助类型（`IvrMenu` / `OutboundRegState` / `ReferSubscription` / `ParkedCall` 等） |
//! | [`inbound_dialog`] | `InboundTransaction` 对话校验 impl |
//! | [`uri_utils`] | Contact/Route 头与 peer 字符串的 URI 工具函数 |
//! | [`constructor`] | `EdgeState` 构造函数与依赖注入器（Redis/NATS/VoiceEngine） |
//! | [`transport`] | 出站 SIP 数据报发送（UDP/TCP/TLS/WS）与 cluster egress 转发 |
//! | [`session`] | B2BUA 会话生命周期（remember_inbound_invite/bind_gateway_dialog/teardown） |
//! | [`media`] | 媒体资源绑定与释放（RTP/录音/会议） |
//! | [`auth`] | SIP Digest 鉴权（Redis + PostgreSQL 双源） |
//! | [`billing`] | 实时计费账户校验 |
//! | [`concurrency`] | 按用户的并发通话计数 |
//! | [`access_trunk`] | 接入网关 IP 规则 |
//! | [`gateway_identity`] | Gateway 身份缓存（SocketAddr/IP → gateway_id） |
//! | [`registration`] | REGISTER 注册查询缓存 |
//! | [`server_transaction`] | 服务端事务表索引 |
//! | (subscription_store) | SUBSCRIBE/NOTIFY 订阅状态（RFC 6665） |
//!
//! ## B2BUA 模型
//!
//! ```text
//! A-leg Call-ID ─┐
//!                ├─> session_id ─> media_session
//! B-leg Call-ID ─┘
//! ```
//!
//! A-leg 与 B-leg 的 Call-ID 都通过
//! [`CallSessionStore::index_dialog`][models::CallSessionStore::index_dialog]
//! 索引到统一的 `session_id`，媒体层只识别 `session_id`，与 wire Call-ID 解耦。

pub(crate) mod access_trunk;
pub(crate) mod auth;
pub(crate) mod billing;
pub(crate) mod concurrency;
pub(crate) mod constructor;
pub(crate) mod gateway_identity;
pub(crate) mod inbound_dialog;
pub(crate) mod media;
pub(crate) mod models;
pub(crate) mod registration;
pub(crate) mod server_transaction;
pub(crate) mod session;
pub(crate) mod transport;
pub(crate) mod types;
pub(crate) mod uri_utils;

pub(crate) use access_trunk::AccessIpRule;
#[cfg(test)]
pub(crate) use billing::build_balance_check;
pub(crate) use models::*;
pub(crate) use types::{
    CdrSinks, IvrAction, IvrMenu, OutboundRegState, ParkedCall, ReferSubscription,
};
pub(crate) use uri_utils::{extract_uri_from_contact, parse_target_addr_from_route};

use crate::media::MediaRelayState;
use crate::sbc;
use crate::sip::client_transaction::ClientTransactionManager;
use crate::sip::registrar::RegistrationStore;
use crate::sip::subscription::SubscriptionStore;
use crate::sip::transaction::{self, InviteAckKey, RequestTransactionKey};

use call_core::{CallManager, GatewayHealthTracker};
use dashmap::DashMap;
use gateway_identity::GatewayIdentityCache;
use registration::CachedRegistrationLookup;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::UdpSocket;

/// sip-edge 进程的全局共享状态。
///
/// 所有字段均为 `pub(crate)`，仅在本 crate 内访问。外部子模块通过
/// `Arc<EdgeState>` 引用并按需调用对应 impl 块（分布在 [`constructor`]、
/// [`transport`]、[`session`]、[`media`] 等子模块中）。
pub(crate) struct EdgeState {
    pub(crate) call_manager: Arc<CallManager>,
    pub(crate) cdr_pipeline_metrics: std::sync::OnceLock<Arc<crate::cdr_spool::CdrPipelineMetrics>>,
    pub(crate) gateway_health: GatewayHealthTracker,
    pub(crate) inbound_transactions: CallSessionStore,
    pub(crate) media_relay: MediaRelayState,
    pub(crate) registrar: tokio::sync::RwLock<RegistrationStore>,
    /// SUBSCRIBE/NOTIFY 订阅状态（RFC 6665）。
    pub(crate) subscription_store: SubscriptionStore,
    pub(crate) db_store: Option<cdr_core::PostgresCdrStore>,
    pub(crate) client_transactions: ClientTransactionManager,
    pub recent_inbound_invites: dashmap::DashMap<String, std::time::Instant>,
    pub(crate) draining: std::sync::atomic::AtomicBool,
    pub(crate) server_transactions: DashMap<
        RequestTransactionKey,
        tokio::sync::mpsc::Sender<transaction::ServerTransactionEvent>,
    >,
    pub(crate) invite_ack_transactions: DashMap<
        InviteAckKey,
        (
            RequestTransactionKey,
            tokio::sync::mpsc::Sender<transaction::ServerTransactionEvent>,
        ),
    >,
    pub(crate) socket: std::sync::OnceLock<Arc<UdpSocket>>,
    pub(crate) test_request_cache: dashmap::DashMap<RequestTransactionKey, Vec<PendingDatagram>>,
    pub(crate) nonce_replay_cache: DashMap<String, u64>,
    pub(crate) tcp_connections: dashmap::DashMap<SocketAddr, tokio::sync::mpsc::Sender<Vec<u8>>>,
    pub(crate) sbc_engine: sbc::SbcEngine,
    pub(crate) sbc_rate_limit_enabled: bool,
    pub(crate) gateway_cache: std::sync::RwLock<GatewayIdentityCache>,
    pub(crate) access_trunk_auth_modes: std::sync::RwLock<HashMap<String, String>>,
    pub(crate) access_username_to_trunk_id: std::sync::RwLock<HashMap<String, String>>,
    pub(crate) trunk_billing_accounts: std::sync::RwLock<HashMap<String, String>>,
    pub(crate) did_destinations: std::sync::RwLock<HashMap<String, cdr_core::DidDestination>>,
    pub(crate) extension_groups: std::sync::RwLock<HashMap<String, Vec<String>>>,
    pub(crate) ivr_menus: std::sync::RwLock<HashMap<String, IvrMenu>>,
    pub(crate) outbound_registrations: dashmap::DashMap<String, OutboundRegState>,
    pub(crate) access_ip_rules: std::sync::RwLock<Vec<AccessIpRule>>,
    pub(crate) registered_access_users: std::sync::RwLock<Vec<String>>,
    /// 按用户名跟踪活跃并发通话数，O(1) 替代 O(n) iter 扫描
    pub(crate) user_concurrency: dashmap::DashMap<String, u32>,
    pub(crate) anti_fraud_rules: std::sync::RwLock<Vec<cdr_core::AntiFraudRule>>,
    pub(crate) media_metrics_log: bool,
    pub(crate) billing_settlement_enabled: bool,
    pub(crate) parked_calls: Arc<dashmap::DashMap<String, ParkedCall>>,
    pub(crate) nats_client: std::sync::OnceLock<async_nats::Client>,
    pub(crate) gateway_health_persistence_enabled: bool,
    /// Active gateway OPTIONS probes keyed by their SIP Call-ID.
    pub(crate) gateway_probes: dashmap::DashMap<String, String>,
    /// Redis 自动重连连接，用于集群状态与呼叫热路径缓存。
    pub(crate) redis_conn: std::sync::OnceLock<redis::aio::ConnectionManager>,
    registration_sync: std::sync::OnceLock<crate::cluster::RegistrationSyncSender>,
    cluster_egress: std::sync::OnceLock<crate::cluster::ClusterEgress>,
    pub(crate) registration_lookup_cache: dashmap::DashMap<String, CachedRegistrationLookup>,
    pub(crate) registration_lookup_locks: dashmap::DashMap<String, Arc<tokio::sync::Mutex<()>>>,
    #[cfg(test)]
    pub(crate) test_gateways: std::sync::Mutex<Vec<String>>,
    pub(crate) sipflow_enabled: std::sync::atomic::AtomicBool,
    pub(crate) sipflow_whitelist: std::sync::RwLock<String>,
    pub(crate) sipflow_retention_days: std::sync::atomic::AtomicI32,
    pub(crate) sip_flow_tx: std::sync::OnceLock<tokio::sync::mpsc::Sender<cdr_core::SipFlowRecord>>,
    pub(crate) call_caller_addrs: dashmap::DashMap<String, SocketAddr>,
    pub(crate) matched_call_ids: dashmap::DashMap<String, std::time::Instant>,
    pub(crate) self_weak: std::sync::OnceLock<std::sync::Weak<EdgeState>>,
    /// IVR TTS/ASR 引擎管理器 (惰性初始化, 由 main.rs 启动阶段注入)
    pub(crate) voice_engine:
        std::sync::OnceLock<Arc<crate::sip::handlers::ivr_topology::VoiceEngineManager>>,
}
