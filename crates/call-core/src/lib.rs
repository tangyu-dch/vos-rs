//! # call-core：呼叫控制与路由引擎
//!
//! 本 crate 是 VoIP 软交换平台的核心呼叫控制层，负责：
//!
//! - **呼叫状态机**：SIP INVITE → Ringing → Established → Terminated 的完整生命周期管理
//! - **路由引擎**：最长前缀匹配 + LCR（最低成本路由）+ 加权负载均衡 + 时间窗口过滤
//! - **网关健康熔断器**：Circuit Breaker 模式，跟踪网关成功率，自动熔断与恢复
//! - **容量控制**：per-gateway 并发呼叫上限，防止过载
//! - **Failover**：408/5xx 响应自动切换到下一个候选网关
//! - **CDR 生成**：呼叫结束时生成通话详单，包含 MOS、RTCP 质量指标
//!
//! ## 核心设计
//!
//! - `CallManager`：并发安全的呼叫管理器，使用 `DashMap`（分片无锁）存储呼叫状态
//! - `RouteTable`：路由表，支持热更新（NATS 广播触发）
//! - `GatewayHealthTracker`：网关健康追踪器，实现 Circuit Breaker 状态机
//!
//! ## 路由选择算法
//!
//! 1. 最长前缀匹配（prefix length DESC）
//! 2. 优先级排序（priority DESC）
//! 3. 最低成本（cost ASC，LCR）
//! 4. 同等条件下加权随机（weight DESC/random）
//! 5. 健康状态过滤（Circuit Breaker）
//! 6. 容量检查（max_capacity / max_concurrent）

mod acd;
mod billing;
mod call;
mod caller_identity;

pub use acd::{AcdEngine, AgentSession, AgentState as AcdAgentState, AllocationStrategy, EnqueueResult, WaitingCall};
pub use billing::AtomicBillingBucket;
pub use call::{
    Call, CallDirection, CallId, CallLeg, CallState, FailureCause, LegDirection, LegId, LegState,
};
pub use caller_identity::{CallerIdentity, CallerIdentityMode, CallerNumberDirectory};
mod cdr;
mod error;
mod manager;
mod outbound_policy;
mod pool_selection;
mod queue;
mod routing;
mod webhooks;

pub use cdr::{CallCdr, CallQualityMetrics, CdrAuditSnapshot, CdrStatus};
pub use error::{CallError, CallResult};
pub use manager::{
    CallEventSendError, CallEventSink, CallManager, CdrSendError, CdrSink, InboundInviteOutcome,
    OutboundResponseOutcome, TerminationOutcome,
};
pub use outbound_policy::{
    CallSource, OutboundPolicyDirectory, RuntimeCallerPool, RuntimeCallerPoolMember,
    RuntimeEgressGroupMember, RuntimeEgressPolicy, RuntimeSourcePolicy,
};
pub use pool_selection::CallerPoolStrategy;
pub use queue::{Agent, AgentState, CallQueue, QueueMetrics, QueueStrategy, QueuedCall};
pub use routing::{
    CircuitState, GatewayHealth, GatewayHealthTracker, GatewayId, HealthThresholds, Route,
    RouteTable, RouteTarget, SelectedRoute,
};
pub use webhooks::{CallEvent, VciInstruction, WebhookEvent, WEBHOOK_SCHEMA_VERSION};

/// 活跃呼叫摘要（供管理 API 暴露）。
///
/// 包含呼叫的基本信息，用于 `/manage/active-calls` 端点返回当前活跃通话列表。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ActiveCall {
    /// SIP Call-ID
    pub call_id: String,
    /// 主叫号码（From header）
    pub caller: Option<String>,
    /// 被叫号码（To URI）
    pub callee: Option<String>,
    /// 当前呼叫状态（Routing/Ringing/Established/Failed/Terminated）
    pub state: String,
    /// 呼叫开始时间（毫秒时间戳）
    pub started_at_ms: i64,
    /// 呼叫接通时间（毫秒时间戳），未接通为 None
    pub answered_at_ms: Option<i64>,
    /// 当前选中的网关 ID
    pub gateway: Option<String>,
}
