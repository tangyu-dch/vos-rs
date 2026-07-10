use call_core::CallCdr;
use serde::{Deserialize, Serialize};
use std::time::SystemTime;
use time::OffsetDateTime;

use crate::utils::{duration_millis, system_time_millis};

pub const DEFAULT_CDR_SUBJECT: &str = "vos-rs.cdrs";
pub const DEFAULT_CDR_STREAM: &str = "VOS_RS_CDRS";

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, PartialEq)]
pub struct AntiFraudRule {
    pub id: String,
    pub rule_type: String,
    pub target_value: String,
    pub limit_number: Option<i32>,
    pub enabled: bool,
}

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
        }
    }

    pub fn to_json_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("CDR event JSON serialization should not fail")
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardStats {
    pub active_calls: i64,
    pub today_total_calls: i64,
    pub today_answered_calls: i64,
    pub today_canceled_calls: i64,
    pub today_failed_calls: i64,
    pub answer_rate: f64,
    pub avg_mos: Option<f64>,
    pub avg_loss_rate: Option<f64>,
    pub avg_jitter_ms: Option<f64>,
    pub registered_users: i64,
    pub active_gateways: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HourlyTrend {
    pub hour: i32,
    pub total: i64,
    pub answered: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SipUser {
    pub username: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    #[serde(
        with = "time::serde::rfc3339::option",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub created_at: Option<OffsetDateTime>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SipGateway {
    pub id: String,
    pub host: String,
    pub port: Option<u16>,
    pub transport: String,
    pub max_capacity: Option<u32>,
    pub gateway_type: Option<String>,
    pub prefix_rules: Option<String>,
    pub supports_registration: Option<bool>,
    pub reg_auth_type: Option<String>,
    pub reg_username: Option<String>,
    pub reg_password: Option<String>,
    pub parent_gateway_id: Option<String>,
    pub caller_id_mode: Option<String>,
    pub virtual_caller: Option<String>,
    pub current_concurrent: Option<i32>,
    pub account_id: Option<i64>,
    pub max_concurrent: Option<i32>,
    pub enabled: Option<bool>,
    #[serde(
        with = "time::serde::rfc3339::option",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub created_at: Option<OffsetDateTime>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SipRoute {
    pub id: String,
    pub prefix: String,
    pub priority: i32,
    pub gateway_id: String,
    pub cost: f64,
    pub weight: i32,
    pub time_start: Option<String>,
    pub time_end: Option<String>,
    #[serde(
        with = "time::serde::rfc3339::option",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub created_at: Option<OffsetDateTime>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SipRegistration {
    pub aor: String,
    pub contact_uri: String,
    pub received_from: String,
    #[serde(with = "time::serde::rfc3339")]
    pub expires_at: OffsetDateTime,
    pub path: Vec<String>,
    #[serde(
        with = "time::serde::rfc3339::option",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub updated_at: Option<OffsetDateTime>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BillingRate {
    pub id: String,
    pub prefix: String,
    pub rate_per_minute: f64,
    pub description: Option<String>,
    #[serde(
        with = "time::serde::rfc3339::option",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub created_at: Option<OffsetDateTime>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BillingAccount {
    pub username: String,
    pub balance: f64,
    pub currency: String,
    #[serde(
        with = "time::serde::rfc3339::option",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub created_at: Option<OffsetDateTime>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LedgerEntry {
    pub id: i64,
    pub call_id: String,
    pub username: String,
    pub duration_ms: i64,
    pub rate_per_minute: f64,
    pub amount: f64,
    pub balance_after: f64,
    #[serde(
        with = "time::serde::rfc3339::option",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub created_at: Option<OffsetDateTime>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReconcileResult {
    pub processed: i64,
    pub skipped: i64,
    pub total_amount: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NumberInventory {
    pub number: String,
    pub username: Option<String>,
    pub gateway_id: Option<String>,
    pub direction: Option<String>,
    pub max_concurrent: Option<i32>,
    pub current_concurrent: Option<i32>,
    pub status: String,
    #[serde(
        with = "time::serde::rfc3339::option",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub created_at: Option<OffsetDateTime>,
    #[serde(
        with = "time::serde::rfc3339::option",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub updated_at: Option<OffsetDateTime>,
}
