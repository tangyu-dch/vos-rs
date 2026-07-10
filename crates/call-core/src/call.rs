use crate::{CallError, CallResult, GatewayId, RouteTarget, SelectedRoute};
use sip_core::{SipRequest, SipUri};
use std::time::SystemTime;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegDirection {
    Inbound,
    Outbound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegState {
    New,
    Inviting,
    Ringing,
    Answered,
    Terminated,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallState {
    Routing,
    Ringing,
    Established,
    Terminated,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailureCause {
    pub status_code: Option<u16>,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallLeg {
    pub id: LegId,
    pub direction: LegDirection,
    pub remote_uri: SipUri,
    pub state: LegState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Call {
    pub id: CallId,
    pub caller: Option<String>,
    pub inbound: CallLeg,
    pub outbound: Option<CallLeg>,
    pub outbound_history: Vec<CallLeg>,
    pub candidates: Vec<SelectedRoute>,
    pub current_candidate_index: usize,
    pub state: CallState,
    pub failure_cause: Option<FailureCause>,
    pub started_at: SystemTime,
    pub answered_at: Option<SystemTime>,
    pub ended_at: Option<SystemTime>,
    pub recording_path: Option<String>,
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
                host: redirect_uri.host.clone(),
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
