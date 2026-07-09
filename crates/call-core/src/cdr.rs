use crate::{Call, CallId, CallState, FailureCause};
use std::time::{Duration, SystemTime};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CdrStatus {
    Answered,
    Canceled,
    Failed,
}

impl CdrStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Answered => "answered",
            Self::Canceled => "canceled",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct CallQualityMetrics {
    pub caller_loss_rate: Option<f64>,
    pub caller_jitter_ms: Option<f64>,
    pub caller_rtt_ms: Option<u32>,
    pub gateway_loss_rate: Option<f64>,
    pub gateway_jitter_ms: Option<f64>,
    pub gateway_rtt_ms: Option<u32>,
    pub mos: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CallCdr {
    pub call_id: CallId,
    pub caller: Option<String>,
    pub callee: Option<String>,
    pub started_at: SystemTime,
    pub answered_at: Option<SystemTime>,
    pub ended_at: SystemTime,
    pub duration: Duration,
    pub billable_duration: Duration,
    pub status: CdrStatus,
    pub failure_cause: Option<FailureCause>,
    pub caller_rtcp_loss_rate: Option<f64>,
    pub caller_rtcp_jitter_ms: Option<f64>,
    pub caller_rtcp_rtt_ms: Option<u32>,
    pub gateway_rtcp_loss_rate: Option<f64>,
    pub gateway_rtcp_jitter_ms: Option<f64>,
    pub gateway_rtcp_rtt_ms: Option<u32>,
    pub mos: Option<f64>,
    pub dtmf_digits: Option<String>,
    pub recording_path: Option<String>,
    pub direction: String,
}

impl CallCdr {
    pub fn from_completed_call(call: &Call) -> Option<Self> {
        Self::from_completed_call_with_metrics(call, None, None, None)
    }

    pub fn from_completed_call_with_metrics(
        call: &Call,
        metrics: Option<CallQualityMetrics>,
        dtmf_digits: Option<String>,
        _recording_path: Option<String>,
    ) -> Option<Self> {
        let ended_at = call.ended_at?;
        let status = match call.state {
            CallState::Terminated if call.answered_at.is_some() => CdrStatus::Answered,
            CallState::Terminated => CdrStatus::Canceled,
            CallState::Failed => CdrStatus::Failed,
            _ => return None,
        };

        let m = metrics.unwrap_or_default();

        Some(Self {
            call_id: call.id.clone(),
            caller: call.caller.clone(),
            callee: call.inbound.remote_uri.user.clone(),
            started_at: call.started_at,
            answered_at: call.answered_at,
            ended_at,
            duration: elapsed(call.started_at, ended_at),
            billable_duration: call
                .answered_at
                .map(|answered_at| elapsed(answered_at, ended_at))
                .unwrap_or_default(),
            status,
            failure_cause: call.failure_cause.clone(),
            caller_rtcp_loss_rate: m.caller_loss_rate,
            caller_rtcp_jitter_ms: m.caller_jitter_ms,
            caller_rtcp_rtt_ms: m.caller_rtt_ms,
            gateway_rtcp_loss_rate: m.gateway_loss_rate,
            gateway_rtcp_jitter_ms: m.gateway_jitter_ms,
            gateway_rtcp_rtt_ms: m.gateway_rtt_ms,
            mos: m.mos,
            dtmf_digits,
            recording_path: call.recording_path.clone(),
            direction: call.direction.clone(),
        })
    }
}

fn elapsed(start: SystemTime, end: SystemTime) -> Duration {
    end.duration_since(start).unwrap_or_default()
}
