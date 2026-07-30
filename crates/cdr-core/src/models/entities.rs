use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, PartialEq)]
pub struct AntiFraudRule {
    pub id: String,
    pub rule_type: String,
    pub target_value: String,
    pub limit_number: Option<i32>,
    pub enabled: bool,
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

/// 每日汇报聚合数据：用于 Copilot 生成"每日汇报"风格回答。
///
/// 包含当日总结、分小时通话趋势、失败原因分布、Top 失败主被叫对，
/// 供 LLM 生成结构化的"当日总结 / 呼叫情况 / 问题原因分析"汇报。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailyReport {
    /// 日报日期（Asia/Shanghai 时区，YYYY-MM-DD）
    pub date: String,
    /// 当日总结
    pub summary: DailyReportSummary,
    /// 分小时通话趋势（0-23 点）
    pub hourly_trend: Vec<HourlyTrend>,
    /// 失败原因分布（按 failure_reason 聚合，取 Top N）
    pub failure_reasons: Vec<FailureReasonStat>,
    /// Top 失败主被叫对（按失败次数倒序，取 Top N）
    pub top_failed_pairs: Vec<FailedPairStat>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailyReportSummary {
    /// 当日总通话数
    pub total_calls: i64,
    /// 当日接通数
    pub answered_calls: i64,
    /// 当日失败数
    pub failed_calls: i64,
    /// 当日取消数
    pub canceled_calls: i64,
    /// 接通率（0.0-1.0）
    pub answer_rate: f64,
    /// 平均通话时长（毫秒，仅接通通话）
    pub avg_duration_ms: Option<f64>,
    /// 总通话分钟数（按 billable_duration_ms 累加，分钟）
    pub total_billable_minutes: Option<f64>,
    /// 平均 MOS 音质评分
    pub avg_mos: Option<f64>,
    /// 平均丢包率
    pub avg_loss_rate: Option<f64>,
    /// 平均抖动（毫秒）
    pub avg_jitter_ms: Option<f64>,
    /// 当前在线注册分机数
    pub registered_users: i64,
    /// 当前活跃网关数
    pub active_gateways: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailureReasonStat {
    /// 失败原因（为空时归为"未记录"）
    pub reason: String,
    /// 出现次数
    pub count: i64,
    /// 占失败通话的比例（0.0-1.0）
    pub ratio: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailedPairStat {
    /// 主叫号码
    pub caller: String,
    /// 被叫号码
    pub callee: String,
    /// 失败次数
    pub failed_count: i64,
    /// 最近一次失败原因
    pub last_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SipUser {
    pub username: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    /// 关联的租户 ID（可空，NULL 表示未关联租户，向后兼容旧数据）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<String>,
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
    pub role: Option<String>,
    pub access_auth_mode: Option<String>,
    pub access_username: Option<String>,
    pub access_realm: Option<String>,
    #[serde(skip)]
    pub access_password_hash: Option<String>,
    pub has_access_password: bool,
    pub prefix_rules: Option<String>,
    pub supports_registration: Option<bool>,
    pub reg_auth_type: Option<String>,
    pub reg_username: Option<String>,
    #[serde(skip_serializing)]
    pub reg_password: Option<String>,
    pub parent_gateway_id: Option<String>,
    pub caller_id_mode: Option<String>,
    pub virtual_caller: Option<String>,
    pub current_concurrent: Option<i32>,
    pub circuit_state: Option<String>,
    pub account_id: Option<i64>,
    pub tenant_id: Option<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topology: Option<serde_json::Value>,
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
    /// SIP 终端的 User-Agent 头（客户端名称）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_agent: Option<String>,
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
pub struct LedgerEntry {
    pub id: i64,
    pub call_id: String,
    pub entry_type: String,
    pub username: String,
    pub duration_ms: i64,
    pub rate_per_minute: Decimal,
    pub billing_interval_secs: i32,
    pub price_per_interval: Decimal,
    pub amount: Decimal,
    pub balance_after: Decimal,
    #[serde(
        with = "time::serde::rfc3339::option",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub created_at: Option<OffsetDateTime>,
}

/// Immutable input for settling one side of a completed call.
#[derive(Debug, Clone, Copy)]
pub struct CallSettlementInput<'a> {
    pub call_id: &'a str,
    pub entry_type: &'a str,
    pub username: &'a str,
    pub callee: &'a str,
    pub duration_ms: i64,
    pub tenant_id: Option<&'a str>,
}

/// Persisted pulse-rating result for one side of a call.
#[derive(Debug, Clone, PartialEq)]
pub struct CallSettlementResult {
    pub balance_after: Decimal,
    pub billed_duration_ms: i64,
    pub amount: Decimal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NumberInventory {
    pub number: String,
    pub username: Option<String>,
    pub allocation_source_type: Option<String>,
    pub allocation_source_id: Option<String>,
    pub gateway_id: Option<String>,
    pub owner_egress_trunk_id: Option<String>,
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

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, PartialEq)]
pub struct AuditLog {
    pub id: i64,
    pub request_id: String,
    pub username: String,
    pub role: String,
    pub method: String,
    pub path: String,
    pub query_params: Option<String>,
    pub request_body: Option<String>,
    pub status_code: i32,
    pub source_ip: Option<String>,
    #[serde(
        with = "time::serde::rfc3339::option",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub created_at: Option<OffsetDateTime>,
}

#[derive(Debug, Clone)]
pub struct AuditLogInput<'a> {
    pub request_id: &'a str,
    pub username: &'a str,
    pub role: &'a str,
    pub method: &'a str,
    pub path: &'a str,
    pub query_params: Option<&'a str>,
    pub request_body: Option<&'a str>,
    pub status_code: u16,
    pub source_ip: Option<&'a str>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, PartialEq)]
pub struct SipFlowRecord {
    pub id: i64,
    pub call_id: String,
    pub method: String,
    pub direction: String,
    pub from_addr: String,
    pub to_addr: String,
    pub raw_message: String,
    #[serde(with = "time::serde::rfc3339")]
    pub timestamp: OffsetDateTime,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IvrMenuRecord {
    pub id: String,
    pub name: String,
    pub welcome_prompt: String,
    pub timeout_secs: i32,
    pub nodes: Option<String>,
    pub edges: Option<String>,
}
