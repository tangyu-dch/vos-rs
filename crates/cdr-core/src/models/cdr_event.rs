use call_core::{CallCdr, CdrAuditSnapshot};
use serde::{Deserialize, Serialize};
use std::time::SystemTime;

use crate::utils::{duration_millis, system_time_millis};

pub const DEFAULT_CDR_SUBJECT: &str = "vos-rs.cdrs";
pub const DEFAULT_CDR_STREAM: &str = "VOS_RS_CDRS";

/// CDR 事件：通话详单的核心数据结构。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CdrEvent {
    pub call_id: String,
    pub caller: Option<String>,
    pub callee: Option<String>,
    pub started_at_ms: i64,
    pub answered_at_ms: Option<i64>,
    pub ended_at_ms: i64,
    pub duration_ms: i64,
    pub billable_duration_ms: i64,
    pub status: String,
    pub failure_status_code: Option<u16>,
    pub failure_reason: Option<String>,
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
    #[serde(default)]
    pub audit: CdrAuditSnapshot,
}

impl CdrEvent {
    pub fn from_call_cdr(cdr: &CallCdr) -> Self {
        Self {
            call_id: cdr.call_id.as_str().to_string(),
            caller: cdr.caller.clone(),
            callee: cdr.callee.clone(),
            started_at_ms: system_time_millis(cdr.started_at),
            answered_at_ms: cdr.answered_at.map(system_time_millis),
            ended_at_ms: system_time_millis(cdr.ended_at),
            duration_ms: duration_millis(cdr.duration),
            billable_duration_ms: duration_millis(cdr.billable_duration),
            status: cdr.status.as_str().to_string(),
            failure_status_code: cdr
                .failure_cause
                .as_ref()
                .and_then(|cause| cause.status_code),
            failure_reason: cdr.failure_cause.as_ref().map(|cause| cause.reason.clone()),
            caller_rtcp_loss_rate: cdr.caller_rtcp_loss_rate,
            caller_rtcp_jitter_ms: cdr.caller_rtcp_jitter_ms,
            caller_rtcp_rtt_ms: cdr.caller_rtcp_rtt_ms,
            gateway_rtcp_loss_rate: cdr.gateway_rtcp_loss_rate,
            gateway_rtcp_jitter_ms: cdr.gateway_rtcp_jitter_ms,
            gateway_rtcp_rtt_ms: cdr.gateway_rtcp_rtt_ms,
            mos: cdr.mos,
            dtmf_digits: cdr.dtmf_digits.clone(),
            recording_path: cdr.recording_path.clone(),
            direction: cdr.direction.clone(),
            audit: cdr.audit.clone(),
        }
    }

    pub fn to_json_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }

    pub fn from_json_slice(payload: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(payload)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DtmfSource {
    Rtp,
    SipInfo,
}

impl DtmfSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Rtp => "rtp",
            Self::SipInfo => "sip-info",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DtmfEventRecord {
    pub call_id: String,
    pub digit: String,
    pub source: DtmfSource,
    pub timestamp_ms: i64,
    pub rtp_timestamp: Option<u32>,
    pub duration_ms: Option<u16>,
    pub volume: Option<u8>,
}

impl DtmfEventRecord {
    pub fn from_rtp(
        call_id: &str,
        digit: char,
        rtp_timestamp: u32,
        duration: u16,
        volume: u8,
    ) -> Self {
        Self {
            call_id: call_id.to_string(),
            digit: digit.to_string(),
            source: DtmfSource::Rtp,
            timestamp_ms: system_time_millis(SystemTime::now()),
            rtp_timestamp: Some(rtp_timestamp),
            duration_ms: Some(duration),
            volume: Some(volume),
        }
    }

    pub fn from_sip_info(call_id: &str, digit: char) -> Self {
        Self {
            call_id: call_id.to_string(),
            digit: digit.to_string(),
            source: DtmfSource::SipInfo,
            timestamp_ms: system_time_millis(SystemTime::now()),
            rtp_timestamp: None,
            duration_ms: None,
            volume: None,
        }
    }
}
