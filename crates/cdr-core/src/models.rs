//! # 数据模型
//!
//! 本模块定义了所有数据表对应的 Rust 结构体，包括：
//!
//! - **CdrEvent**：通话详单
//! - **SipGateway**：网关配置
//! - **SipRoute**：路由规则
//! - **SipUser**：SIP 用户
//! - **SipRegistration**：注册绑定
//! - **BillingRate**：费率
//! - **BillingAccount**：计费账户
//! - **LedgerEntry**：扣费流水
//! - **AntiFraudRule**：反欺诈规则
//! - **NumberInventory**：号码库存
//! - **DtmfEventRecord**：DTMF 事件
//!
//! ## 命名约定
//!
//! - 结构体使用 PascalCase
//! - 字段使用 snake_case
//! - 数据库表名使用 snake_case + 复数

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

/// CDR 事件：通话详单的核心数据结构。
///
/// 包含呼叫的完整信息：主叫/被叫、时间、时长、状态、质量指标等。
/// 用于 API 查询、报表生成和计费对账。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CdrEvent {
    /// SIP Call-ID
    pub call_id: String,
    /// 主叫号码（From header）
    pub caller: Option<String>,
    /// 被叫号码（To URI）
    pub callee: Option<String>,
    /// 呼叫开始时间（毫秒时间戳）
    pub started_at_ms: i64,
    /// 呼叫接通时间（毫秒时间戳）
    pub answered_at_ms: Option<i64>,
    /// 呼叫结束时间（毫秒时间戳）
    pub ended_at_ms: i64,
    /// 总时长（毫秒）
    pub duration_ms: i64,
    /// 计费时长（毫秒）
    pub billable_duration_ms: i64,
    /// 呼叫状态（answered/canceled/failed）
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

/// SIP 网关配置：定义出站 SIP 网关的连接参数和策略。
///
/// 网关是 VoIP 软交换与外部 PSTN/VoIP 网络的接口，
/// 每个网关包含地址、端口、传输协议、容量限制等配置。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SipGateway {
    /// 网关唯一标识
    pub id: String,
    /// 网关主机地址
    pub host: String,
    /// 网关 SIP 端口（默认 5060）
    pub port: Option<u16>,
    /// 传输协议（udp/tcp/tls）
    pub transport: String,
    /// 最大并发呼叫数（0 表示无限制）
    pub max_capacity: Option<u32>,
    /// 网关类型（如 pstn/sip/trunk）
    pub gateway_type: Option<String>,
    /// 前缀转换规则（如 "86:0086"）
    pub prefix_rules: Option<String>,
    /// 是否支持注册
    pub supports_registration: Option<bool>,
    /// 注册认证类型（digest/basic）
    pub reg_auth_type: Option<String>,
    /// 注册用户名
    pub reg_username: Option<String>,
    /// 注册密码
    pub reg_password: Option<String>,
    /// 父网关 ID（用于级联）
    pub parent_gateway_id: Option<String>,
    /// Caller ID 重写模式（passthrough/virtual/random）
    pub caller_id_mode: Option<String>,
    /// 固定虚拟主叫号码
    pub virtual_caller: Option<String>,
    /// 当前并发呼叫数
    pub current_concurrent: Option<i32>,
    /// 熔断器状态（closed/open/half_open）
    pub circuit_state: Option<String>,
    /// 关联的计费账户 ID
    pub account_id: Option<i64>,
    /// 每用户最大并发数
    pub max_concurrent: Option<i32>,
    /// 是否启用
    pub enabled: Option<bool>,
    /// 创建时间
    #[serde(
        with = "time::serde::rfc3339::option",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub created_at: Option<OffsetDateTime>,
}

/// SIP 路由规则：定义被叫号码到网关的映射。
///
/// 路由引擎根据这些规则选择出站网关：
/// 1. 最长前缀匹配（prefix length DESC）
/// 2. 优先级排序（priority DESC）
/// 3. 最低成本（cost ASC，LCR）
/// 4. 同等条件下加权随机（weight DESC/random）
/// 5. 时间窗口过滤
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SipRoute {
    /// 路由唯一标识
    pub id: String,
    /// 被叫号码前缀（如 "86" 表示中国大陆）
    pub prefix: String,
    /// 优先级（数字越大越优先）
    pub priority: i32,
    /// 目标网关 ID
    pub gateway_id: String,
    /// 每呼叫成本（用于最低成本路由）
    pub cost: f64,
    /// 权重（用于同等条件下的加权随机）
    pub weight: i32,
    /// 生效开始时间（HH:MM 格式）
    pub time_start: Option<String>,
    /// 生效结束时间（HH:MM 格式）
    pub time_end: Option<String>,
    /// 创建时间
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

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, PartialEq)]
pub struct AntiFraudConfigItem {
    pub config_key: String,
    pub config_value: String,
    pub description: Option<String>,
    #[serde(
        with = "time::serde::rfc3339::option",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub updated_at: Option<OffsetDateTime>,
}

/// 管理 API 审计日志记录。
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, PartialEq)]
pub struct AuditLog {
    pub id: i64,
    pub request_id: String,
    pub username: String,
    pub role: String,
    pub method: String,
    pub path: String,
    pub status_code: i32,
    pub source_ip: Option<String>,
    #[serde(
        with = "time::serde::rfc3339::option",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub created_at: Option<OffsetDateTime>,
}

/// 写入审计日志所需的请求信息。
#[derive(Debug, Clone)]
pub struct AuditLogInput<'a> {
    pub request_id: &'a str,
    pub username: &'a str,
    pub role: &'a str,
    pub method: &'a str,
    pub path: &'a str,
    pub status_code: u16,
    pub source_ip: Option<&'a str>,
}
