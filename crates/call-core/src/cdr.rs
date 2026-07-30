use crate::{Call, CallId, CallState, FailureCause};
use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime};

/// Immutable routing and billing decisions captured when a call is established.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CdrAuditSnapshot {
    pub source_type: Option<String>,
    pub source_id: Option<String>,
    pub billing_account: Option<String>,
    pub original_caller: Option<String>,
    pub presented_caller: Option<String>,
    pub caller_mode: Option<String>,
    pub caller_pool_id: Option<String>,
    pub caller_selection: Option<String>,
    pub ingress_trunk_id: Option<String>,
    pub egress_trunk_id: Option<String>,
    pub selected_route_id: Option<String>,
    pub fallback_used: bool,
    pub billing_interval_secs: Option<u32>,
    pub price_per_interval: Option<f64>,
    pub egress_billing_account: Option<String>,
    pub egress_billing_interval_secs: Option<u32>,
    pub egress_price_per_interval: Option<f64>,
    pub tenant_id: Option<String>,
    pub tenant_name: Option<String>,
    pub auth_realm: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CallCdr {
    pub call_id: CallId,
    pub caller: Option<String>,
    pub callee: Option<String>,
    pub started_at: SystemTime,
    #[serde(default)]
    pub ringing_at: Option<SystemTime>,
    pub answered_at: Option<SystemTime>,
    pub ended_at: SystemTime,
    pub duration: Duration,
    pub billable_duration: Duration,
    #[serde(default)]
    pub ringing_duration: Option<Duration>,
    #[serde(default)]
    pub access_billable_duration: Option<Duration>,
    #[serde(default)]
    pub access_charge_amount: Option<f64>,
    #[serde(default)]
    pub egress_billable_duration: Option<Duration>,
    #[serde(default)]
    pub egress_cost_amount: Option<f64>,
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
    pub audit: CdrAuditSnapshot,
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

        let selected_route = call.candidates.get(call.current_candidate_index);
        let identity = call.caller_identity.as_ref();
        let mut audit = call.audit.clone();
        audit.billing_account.clone_from(&call.billing_account);
        if let Some(identity) = identity {
            audit.original_caller = Some(identity.original_number.clone());
            audit.presented_caller = Some(identity.presented_number.clone());
        }
        audit.egress_trunk_id =
            selected_route.map(|route| route.target.gateway_id.as_str().to_string());
        audit.selected_route_id = selected_route.map(|route| route.route_id.clone());
        audit.fallback_used = audit.fallback_used
            || !call.outbound_history.is_empty()
            || call.current_candidate_index > 0;

        let answered_duration = call
            .answered_at
            .map(|answered_at| elapsed(answered_at, ended_at));
        let ringing_duration = call
            .ringing_at
            .map(|ringing_at| elapsed(ringing_at, call.answered_at.unwrap_or(ended_at)));
        let (access_billable_duration, access_charge_amount) = billing_snapshot(
            answered_duration,
            audit.billing_interval_secs,
            audit.price_per_interval,
        );
        let (egress_billable_duration, egress_cost_amount) = billing_snapshot(
            answered_duration,
            audit.egress_billing_interval_secs,
            audit.egress_price_per_interval,
        );

        Some(Self {
            call_id: call.id.clone(),
            caller: call.caller.as_deref().and_then(actual_number),
            callee: call.inbound.remote_uri.user.as_ref().map(|u| u.to_string()),
            started_at: call.started_at,
            ringing_at: call.ringing_at,
            answered_at: call.answered_at,
            ended_at,
            duration: elapsed(call.started_at, ended_at),
            billable_duration: answered_duration.unwrap_or_default(),
            ringing_duration,
            access_billable_duration,
            access_charge_amount,
            egress_billable_duration,
            egress_cost_amount,
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
            audit,
        })
    }
}

fn elapsed(start: SystemTime, end: SystemTime) -> Duration {
    end.duration_since(start).unwrap_or_default()
}

fn billing_snapshot(
    duration: Option<Duration>,
    interval_secs: Option<u32>,
    price_per_interval: Option<f64>,
) -> (Option<Duration>, Option<f64>) {
    let Some(duration) = duration.filter(|value| !value.is_zero()) else {
        return (None, None);
    };
    let Some(interval_secs) = interval_secs.filter(|value| *value > 0) else {
        return (None, None);
    };
    let Some(price_per_interval) = price_per_interval.filter(|value| *value >= 0.0) else {
        return (None, None);
    };
    let interval_millis = u128::from(interval_secs) * 1_000;
    let pulses = duration.as_millis().div_ceil(interval_millis);
    let billed_millis = pulses.saturating_mul(interval_millis).min(u64::MAX as u128) as u64;
    (
        Some(Duration::from_millis(billed_millis)),
        Some(pulses as f64 * price_per_interval),
    )
}

fn actual_number(value: &str) -> Option<String> {
    let trimmed = value.trim();
    let candidate = trimmed
        .find("sip:")
        .map(|index| &trimmed[index + 4..])
        .unwrap_or(trimmed)
        .split(['@', ';', '>'])
        .next()
        .unwrap_or_default()
        .trim_matches(['\"', '<', ' ']);
    (!candidate.is_empty()).then(|| candidate.to_string())
}

#[cfg(test)]
mod tests {
    use super::{actual_number, billing_snapshot};
    use std::time::Duration;

    #[test]
    fn pulse_billing_rounds_partial_interval_up() {
        let (duration, amount) =
            billing_snapshot(Some(Duration::from_secs(61)), Some(60), Some(0.5));
        assert_eq!(duration, Some(Duration::from_secs(120)));
        assert_eq!(amount, Some(1.0));
    }

    #[test]
    fn actual_number_removes_sip_display_and_parameters() {
        assert_eq!(
            actual_number("\"1002\" <sip:1002@vos-rs>;tag=abc"),
            Some("1002".to_string())
        );
        assert_eq!(actual_number("1002"), Some("1002".to_string()));
    }
}
