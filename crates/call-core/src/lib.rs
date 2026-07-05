mod call;
mod cdr;
mod error;
mod manager;
mod routing;

pub use call::{Call, CallId, CallLeg, CallState, FailureCause, LegDirection, LegId, LegState};
pub use cdr::{CallCdr, CallQualityMetrics, CdrStatus};
pub use error::{CallError, CallResult};
pub use manager::{CallManager, InboundInviteOutcome, OutboundResponseOutcome, TerminationOutcome};
pub use routing::{
    GatewayHealth, GatewayHealthTracker, GatewayId, HealthThresholds, Route, RouteTable,
    RouteTarget, SelectedRoute,
};

/// 活跃呼叫摘要（供管理 API 暴露）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ActiveCall {
    pub call_id: String,
    pub caller: Option<String>,
    pub callee: Option<String>,
    pub state: String,
    pub started_at_ms: i64,
    pub answered_at_ms: Option<i64>,
    pub gateway: Option<String>,
}
