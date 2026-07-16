//! # 呼叫状态机
//!
//! 本模块定义了 SIP 呼叫的核心数据结构和状态转换逻辑。
//!
//! ## 呼叫状态
//!
//! ```text
//! Routing → Ringing → Established → Terminated
//!            ↓           ↓
//!          Failed      Failed
//! ```
//!
//! - **Routing**：INVITE 已接收，正在选择路由
//! - **Ringing**：收到 180 Ringing
//! - **Established**：收到 200 OK，通话建立
//! - **Terminated**：收到 BYE，通话结束
//! - **Failed**：呼叫失败（4xx/5xx/timeout）
//!
//! ## 呼叫分支（Call Leg）
//!
//! 每个呼叫有两个分支：
//! - **Inbound**：主叫方到软交换
//! - **Outbound**：软交换到被叫方（网关）

use crate::{CallError, CallResult, GatewayId, RouteTarget, SelectedRoute};
use sip_core::{SipRequest, SipUri};
use std::time::SystemTime;

/// SIP Call-ID：呼叫的全局唯一标识符。
///
/// 用于在呼叫管理器中标识不同的呼叫。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CallId(String);

impl CallId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// 呼叫分支 ID：标识呼叫的 inbound/outbound 分支。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LegId(String);

impl LegId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// 呼叫分支方向。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegDirection {
    /// 入站（主叫方到软交换）
    Inbound,
    /// 出站（软交换到网关）
    Outbound,
}

/// 呼叫分支状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegState {
    /// 新建
    New,
    /// 正在发送 INVITE
    Inviting,
    /// 收到 180 Ringing
    Ringing,
    /// 收到 200 OK
    Answered,
    /// 已终止
    Terminated,
    /// 失败
    Failed,
}

/// 呼叫状态。
///
/// 表示整个呼叫的生命周期状态，用于路由选择和 CDR 生成。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallState {
    /// 正在选择路由（INVITE 已接收）
    Routing,
    /// 收到 180 Ringing
    Ringing,
    /// 通话建立（收到 200 OK）
    Established,
    /// 通话终止（收到 BYE）
    Terminated,
    /// 呼叫失败（4xx/5xx/timeout）
    Failed,
}

impl CallState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Routing => "routing",
            Self::Ringing => "ringing",
            Self::Established => "established",
            Self::Terminated => "terminated",
            Self::Failed => "failed",
        }
    }
}

use serde::{Deserialize, Serialize};

/// 呼叫失败原因。
///
/// 包含 SIP 状态码和失败原因描述。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailureCause {
    /// SIP 状态码（如 408、503）
    pub status_code: Option<u16>,
    /// 失败原因描述
    pub reason: String,
}

/// 呼叫分支：表示呼叫的一端（inbound 或 outbound）。
///
/// 每个呼叫有两个分支：
/// - Inbound：主叫方到软交换
/// - Outbound：软交换到网关
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallLeg {
    /// 分支 ID
    pub id: LegId,
    /// 分支方向
    pub direction: LegDirection,
    /// 远端 SIP URI
    pub remote_uri: SipUri,
    pub state: LegState,
}

/// 呼叫：VoIP 通话的完整状态表示。
///
/// 每个 SIP INVITE 创建一个 `Call` 对象，包含：
/// - 呼叫 ID、主叫号码、呼叫方向
/// - 入站/出站分支（CallLeg）
/// - 路由候选列表和当前选中索引
/// - 呼叫状态、失败原因、时间戳
/// - 录音路径、质量指标
///
/// 呼叫状态机：
/// ```text
/// Routing → Ringing → Established → Terminated
///            ↓           ↓
///          Failed      Failed
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Call {
    /// 呼叫 ID（SIP Call-ID）
    pub id: CallId,
    /// 主叫号码（From header）
    pub caller: Option<String>,
    /// 入站分支（主叫方到软交换）
    pub inbound: CallLeg,
    /// 出站分支（软交换到网关）
    pub outbound: Option<CallLeg>,
    /// 出站分支历史（Failover 时记录）
    pub outbound_history: Vec<CallLeg>,
    /// 路由候选列表（按优先级排序）
    pub candidates: Vec<SelectedRoute>,
    /// 当前选中的候选索引
    pub current_candidate_index: usize,
    /// 当前呼叫状态
    pub state: CallState,
    /// 失败原因（仅在 Failed 状态时有值）
    pub failure_cause: Option<FailureCause>,
    /// 呼叫开始时间
    pub started_at: SystemTime,
    /// 呼叫接通时间（收到 200 OK）
    pub answered_at: Option<SystemTime>,
    /// 呼叫结束时间（收到 BYE）
    pub ended_at: Option<SystemTime>,
    /// 录音文件路径
    pub recording_path: Option<String>,
    /// 呼叫方向（inbound/outbound）
    pub direction: String,
}

impl Call {
    pub fn from_inbound_invite(request: &SipRequest) -> CallResult<Self> {
        Self::from_inbound_invite_at(request, SystemTime::now())
    }

    pub fn from_inbound_invite_at(
        request: &SipRequest,
        started_at: SystemTime,
    ) -> CallResult<Self> {
        let call_id = request
            .headers
            .get("call-id")
            .ok_or(CallError::MissingRequiredHeader("Call-ID"))?
            .as_str()
            .to_string();

        Ok(Self {
            id: CallId::new(call_id),
            caller: request
                .headers
                .get("from")
                .map(|value| value.as_str().to_string()),
            inbound: CallLeg {
                id: LegId::new("inbound"),
                direction: LegDirection::Inbound,
                remote_uri: request.uri.clone(),
                state: LegState::Inviting,
            },
            outbound: None,
            outbound_history: Vec::new(),
            candidates: Vec::new(),
            current_candidate_index: 0,
            state: CallState::Routing,
            failure_cause: None,
            started_at,
            answered_at: None,
            ended_at: None,
            recording_path: None,
            direction: "outbound".to_string(),
        })
    }

    pub fn failover_to_next(&mut self) -> CallResult<Option<SipUri>> {
        self.failover_to_next_at(SystemTime::now())
    }

    pub fn failover_to_next_at(&mut self, _now: SystemTime) -> CallResult<Option<SipUri>> {
        if self.current_candidate_index + 1 >= self.candidates.len() {
            return Ok(None);
        }

        if let Some(mut old_outbound) = self.outbound.take() {
            old_outbound.state = LegState::Failed;
            self.outbound_history.push(old_outbound);
        }

        self.current_candidate_index += 1;
        let next_route = &self.candidates[self.current_candidate_index];

        self.outbound = Some(CallLeg {
            id: LegId::new(format!("outbound-{}", self.current_candidate_index)),
            direction: LegDirection::Outbound,
            remote_uri: next_route.outbound_uri.clone(),
            state: LegState::Inviting,
        });

        self.state = CallState::Routing;
        self.failure_cause = None;

        Ok(Some(next_route.outbound_uri.clone()))
    }

    pub fn redirect_to(&mut self, redirect_uri: SipUri) -> CallResult<Option<SipUri>> {
        let selected_route = SelectedRoute {
            route_id: format!("redirect-{}", self.candidates.len()),
            target: RouteTarget {
                gateway_id: GatewayId::new("redirect"),
                host: redirect_uri.host.to_string(),
                port: redirect_uri.port,
                transport: Some("udp".to_string()),
                max_capacity: None,
                caller_id_mode: None,
                virtual_caller: None,
                prefix_rules: None,
                direction: None,
                max_concurrent: None,
                current_concurrent: 0,
            },
            outbound_uri: redirect_uri.clone(),
        };
        self.candidates.push(selected_route);
        let new_index = self.candidates.len() - 1;

        if let Some(mut old_outbound) = self.outbound.take() {
            old_outbound.state = LegState::Failed;
            self.outbound_history.push(old_outbound);
        }

        self.current_candidate_index = new_index;

        self.outbound = Some(CallLeg {
            id: LegId::new(format!("outbound-{}", self.current_candidate_index)),
            direction: LegDirection::Outbound,
            remote_uri: redirect_uri.clone(),
            state: LegState::Inviting,
        });

        self.state = CallState::Routing;
        self.failure_cause = None;

        Ok(Some(redirect_uri))
    }

    pub fn select_route(&mut self, outbound_uri: SipUri) -> CallResult<()> {
        self.ensure_state(CallState::Routing, "select_route")?;

        if self.outbound.is_some() {
            return Err(CallError::OutboundLegAlreadyExists);
        }

        self.outbound = Some(CallLeg {
            id: LegId::new("outbound"),
            direction: LegDirection::Outbound,
            remote_uri: outbound_uri,
            state: LegState::Inviting,
        });
        Ok(())
    }

    pub fn mark_ringing(&mut self) -> CallResult<()> {
        // Allow re-entry from Ringing (for 183 early media after 180)
        if self.state == CallState::Ringing {
            return Ok(());
        }
        self.ensure_state(CallState::Routing, "mark_ringing")?;

        let outbound = self
            .outbound
            .as_mut()
            .ok_or(CallError::MissingOutboundLeg)?;
        outbound.state = LegState::Ringing;
        self.inbound.state = LegState::Ringing;
        self.state = CallState::Ringing;
        Ok(())
    }

    pub fn mark_answered(&mut self) -> CallResult<()> {
        self.mark_answered_at(SystemTime::now())
    }

    pub fn mark_answered_at(&mut self, answered_at: SystemTime) -> CallResult<()> {
        if !matches!(self.state, CallState::Routing | CallState::Ringing) {
            return Err(CallError::InvalidTransition {
                from: self.state.as_str(),
                event: "mark_answered",
            });
        }

        let outbound = self
            .outbound
            .as_mut()
            .ok_or(CallError::MissingOutboundLeg)?;
        outbound.state = LegState::Answered;
        self.inbound.state = LegState::Answered;
        self.state = CallState::Established;
        self.answered_at = Some(answered_at);
        Ok(())
    }

    pub fn terminate(&mut self) -> CallResult<()> {
        self.terminate_at(SystemTime::now())
    }

    pub fn terminate_at(&mut self, ended_at: SystemTime) -> CallResult<()> {
        if matches!(self.state, CallState::Terminated | CallState::Failed) {
            return Err(CallError::InvalidTransition {
                from: self.state.as_str(),
                event: "terminate",
            });
        }

        self.inbound.state = LegState::Terminated;
        if let Some(outbound) = self.outbound.as_mut() {
            outbound.state = LegState::Terminated;
        }
        self.state = CallState::Terminated;
        self.ended_at = Some(ended_at);
        Ok(())
    }

    pub fn fail(&mut self, status_code: Option<u16>, reason: impl Into<String>) -> CallResult<()> {
        self.fail_at(SystemTime::now(), status_code, reason)
    }

    pub fn fail_at(
        &mut self,
        ended_at: SystemTime,
        status_code: Option<u16>,
        reason: impl Into<String>,
    ) -> CallResult<()> {
        if matches!(self.state, CallState::Terminated | CallState::Failed) {
            return Err(CallError::InvalidTransition {
                from: self.state.as_str(),
                event: "fail",
            });
        }

        self.inbound.state = LegState::Failed;
        if let Some(outbound) = self.outbound.as_mut() {
            outbound.state = LegState::Failed;
        }
        self.state = CallState::Failed;
        self.failure_cause = Some(FailureCause {
            status_code,
            reason: reason.into(),
        });
        self.ended_at = Some(ended_at);
        Ok(())
    }

    fn ensure_state(&self, expected: CallState, event: &'static str) -> CallResult<()> {
        if self.state == expected {
            Ok(())
        } else {
            Err(CallError::InvalidTransition {
                from: self.state.as_str(),
                event,
            })
        }
    }
}
