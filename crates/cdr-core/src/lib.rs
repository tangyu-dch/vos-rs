use call_core::CallCdr;
use serde::{Deserialize, Serialize};
use sqlx::{postgres::PgPoolOptions, PgPool, Row};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use time::OffsetDateTime;

pub const DEFAULT_CDR_SUBJECT: &str = "vos-rs.cdrs";
pub const DEFAULT_CDR_STREAM: &str = "VOS_RS_CDRS";

const CREATE_CDR_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS call_cdrs (
    id BIGSERIAL PRIMARY KEY,
    call_id TEXT NOT NULL,
    caller TEXT,
    callee TEXT,
    started_at TIMESTAMPTZ NOT NULL,
    answered_at TIMESTAMPTZ,
    ended_at TIMESTAMPTZ NOT NULL,
    duration_ms BIGINT NOT NULL,
    billable_duration_ms BIGINT NOT NULL,
    status TEXT NOT NULL,
    failure_status_code INTEGER,
    failure_reason TEXT,
    caller_rtcp_loss_rate DOUBLE PRECISION,
    caller_rtcp_jitter_ms DOUBLE PRECISION,
    caller_rtcp_rtt_ms INTEGER,
    gateway_rtcp_loss_rate DOUBLE PRECISION,
    gateway_rtcp_jitter_ms DOUBLE PRECISION,
    gateway_rtcp_rtt_ms INTEGER,
    mos DOUBLE PRECISION,
    dtmf_digits TEXT,
    inserted_at TIMESTAMPTZ NOT NULL DEFAULT now()
)
"#;

const CREATE_CALL_ID_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_call_cdrs_call_id ON call_cdrs (call_id)";
const CREATE_STARTED_AT_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_call_cdrs_started_at ON call_cdrs (started_at)";
const CREATE_STATUS_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_call_cdrs_status ON call_cdrs (status)";

const CREATE_SIP_USERS_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS sip_users (
    username TEXT PRIMARY KEY,
    password TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
)
"#;

const CREATE_SIP_GATEWAYS_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS sip_gateways (
    id TEXT PRIMARY KEY,
    host TEXT NOT NULL,
    port INTEGER,
    transport TEXT NOT NULL DEFAULT 'udp',
    max_capacity INTEGER,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
)
"#;

const CREATE_SIP_ROUTES_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS sip_routes (
    id TEXT PRIMARY KEY,
    prefix TEXT NOT NULL,
    priority INTEGER NOT NULL DEFAULT 100,
    gateway_id TEXT NOT NULL REFERENCES sip_gateways(id) ON DELETE CASCADE,
    cost DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
)
"#;

const CREATE_DTMF_EVENTS_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS dtmf_events (
    id BIGSERIAL PRIMARY KEY,
    call_id TEXT NOT NULL,
    digit TEXT NOT NULL,
    source TEXT NOT NULL,
    timestamp_ms BIGINT NOT NULL,
    rtp_timestamp BIGINT,
    duration_ms INTEGER,
    volume INTEGER,
    inserted_at TIMESTAMPTZ NOT NULL DEFAULT now()
)
"#;

const CREATE_DTMF_CALL_ID_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_dtmf_events_call_id ON dtmf_events (call_id)";

const CREATE_SIP_REGISTRATIONS_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS sip_registrations (
    aor TEXT NOT NULL,
    contact_uri TEXT NOT NULL,
    received_from TEXT NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    path TEXT,
    PRIMARY KEY (aor, contact_uri)
)
"#;

const CREATE_BILLING_RATES_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS billing_rates (
    id TEXT PRIMARY KEY,
    prefix TEXT NOT NULL,
    rate_per_minute DOUBLE PRECISION NOT NULL,
    description TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
)
"#;

const CREATE_BILLING_ACCOUNTS_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS billing_accounts (
    username TEXT PRIMARY KEY,
    balance DOUBLE PRECISION NOT NULL DEFAULT 0,
    currency TEXT NOT NULL DEFAULT 'CNY',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
)
"#;

const CREATE_BILLING_LEDGER_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS billing_ledger (
    id BIGSERIAL PRIMARY KEY,
    call_id TEXT NOT NULL UNIQUE,
    username TEXT NOT NULL,
    duration_ms BIGINT NOT NULL,
    rate_per_minute DOUBLE PRECISION NOT NULL,
    amount DOUBLE PRECISION NOT NULL,
    balance_after DOUBLE PRECISION NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
)
"#;

const CREATE_LEDGER_USERNAME_INDEX_SQL: &str =
    "CREATE INDEX IF NOT EXISTS idx_billing_ledger_username ON billing_ledger (username)";

const CREATE_NUMBER_INVENTORY_TABLE_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS number_inventory (
    number TEXT PRIMARY KEY,
    username TEXT,
    status TEXT NOT NULL DEFAULT 'available',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
)
"#;

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
        }
    }

    pub fn to_json_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("CDR event JSON serialization should not fail")
    }

    pub fn from_json_slice(payload: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(payload)
    }
}

/// Source of a DTMF event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DtmfSource {
    /// RFC 2833/4733 RTP telephone-event.
    Rtp,
    /// SIP INFO with `application/dtmf-relay` or `application/dtmf`.
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

/// A single DTMF event record for the audit detail table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DtmfEventRecord {
    pub call_id: String,
    pub digit: String,
    pub source: DtmfSource,
    /// Wall-clock time when the event was detected (ms since UNIX epoch).
    pub timestamp_ms: i64,
    /// RTP timestamp from the telephone-event packet (RTP source only).
    pub rtp_timestamp: Option<u32>,
    /// Duration of the DTMF tone in milliseconds (RTP source only).
    pub duration_ms: Option<u16>,
    /// Volume level in dBm0 (RTP source only).
    pub volume: Option<u8>,
}

impl DtmfEventRecord {
    /// Create a DTMF event from an RTP telephone-event packet.
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

    /// Create a DTMF event from a SIP INFO message.
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

// ===== API 数据模型 =====

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

/// 按小时聚合的呼叫趋势（用于仪表板折线图）。
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
    #[serde(with = "time::serde::rfc3339::option", default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<OffsetDateTime>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SipGateway {
    pub id: String,
    pub host: String,
    pub port: Option<u16>,
    pub transport: String,
    pub max_capacity: Option<u32>,
    #[serde(with = "time::serde::rfc3339::option", default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<OffsetDateTime>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SipRoute {
    pub id: String,
    pub prefix: String,
    pub priority: i32,
    pub gateway_id: String,
    pub cost: f64,
    pub time_start: Option<String>,
    pub time_end: Option<String>,
    #[serde(with = "time::serde::rfc3339::option", default, skip_serializing_if = "Option::is_none")]
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
    #[serde(with = "time::serde::rfc3339::option", default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<OffsetDateTime>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BillingRate {
    pub id: String,
    pub prefix: String,
    pub rate_per_minute: f64,
    pub description: Option<String>,
    #[serde(with = "time::serde::rfc3339::option", default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<OffsetDateTime>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BillingAccount {
    pub username: String,
    pub balance: f64,
    pub currency: String,
    #[serde(with = "time::serde::rfc3339::option", default, skip_serializing_if = "Option::is_none")]
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
    #[serde(with = "time::serde::rfc3339::option", default, skip_serializing_if = "Option::is_none")]
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
    pub status: String,
    #[serde(with = "time::serde::rfc3339::option", default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<OffsetDateTime>,
}

// ===== 辅助函数 =====

fn system_time_millis(value: SystemTime) -> i64 {
    let millis = value
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    i64::try_from(millis).unwrap_or(i64::MAX)
}

fn duration_millis(value: Duration) -> i64 {
    i64::try_from(value.as_millis()).unwrap_or(i64::MAX)
}

fn offset_from_millis(value: i64) -> OffsetDateTime {
    let nanos = i128::from(value).saturating_mul(1_000_000);
    OffsetDateTime::from_unix_timestamp_nanos(nanos).unwrap_or(OffsetDateTime::UNIX_EPOCH)
}

/// 从 SIP From/Contact 头提取 user 部分（如 `"1001" <sip:1001@...>` → `1001`）。
fn extract_sip_user(value: &str) -> Option<&str> {
    let idx = value.find("sip:")?;
    let rest = &value[idx + 4..];
    let end = rest
        .find(|c: char| c == '@' || c == ';' || c == '>')
        .unwrap_or(rest.len());
    if end == 0 {
        None
    } else {
        Some(&rest[..end])
    }
}

/// 按被叫号码匹配费率：最长前缀优先，空前缀兜底，无匹配返回 0。
fn match_rate(callee: &str, rates: &[(String, f64)]) -> f64 {
    let mut best_match: Option<(&str, f64)> = None;
    let mut fallback: Option<f64> = None;

    for (prefix, rate) in rates {
        if prefix.is_empty() {
            fallback = Some(*rate);
        } else if callee.starts_with(prefix.as_str()) {
            match best_match {
                Some((best_prefix, _)) if best_prefix.len() >= prefix.len() => {}
                _ => best_match = Some((prefix.as_str(), *rate)),
            }
        }
    }

    best_match.map_or(fallback.unwrap_or(0.0), |(_, rate)| rate)
}

fn cdr_event_from_row(row: sqlx::postgres::PgRow) -> Result<CdrEvent, sqlx::Error> {
    let started_at: OffsetDateTime = row.get(3);
    let answered_at: Option<OffsetDateTime> = row.get(4);
    let ended_at: OffsetDateTime = row.get(5);
    let caller_rtcp_rtt_ms: Option<i32> = row.get(12);
    let gateway_rtcp_rtt_ms: Option<i32> = row.get(16);

    Ok(CdrEvent {
        call_id: row.get(0),
        caller: row.get(1),
        callee: row.get(2),
        started_at_ms: started_at.unix_timestamp_nanos() as i64 / 1_000_000,
        answered_at_ms: answered_at.map(|t| t.unix_timestamp_nanos() as i64 / 1_000_000),
        ended_at_ms: ended_at.unix_timestamp_nanos() as i64 / 1_000_000,
        duration_ms: row.get(6),
        billable_duration_ms: row.get(7),
        status: row.get(8),
        failure_status_code: row.get::<Option<i32>, _>(9).map(|v| v as u16),
        failure_reason: row.get(10),
        caller_rtcp_loss_rate: row.get(11),
        caller_rtcp_jitter_ms: row.get(12),
        caller_rtcp_rtt_ms: caller_rtcp_rtt_ms.map(|v| v as u32),
        gateway_rtcp_loss_rate: row.get(14),
        gateway_rtcp_jitter_ms: row.get(15),
        gateway_rtcp_rtt_ms: gateway_rtcp_rtt_ms.map(|v| v as u32),
        mos: row.get(17),
        dtmf_digits: row.get(18),
    })
}

#[derive(Debug, Clone)]
pub struct PostgresCdrStore {
    pool: PgPool,
}

impl PostgresCdrStore {
    pub async fn connect(database_url: &str) -> Result<Self, sqlx::Error> {
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(database_url)
            .await?;
        let store = Self { pool };
        store.migrate().await?;
        Ok(store)
    }

    async fn migrate(&self) -> Result<(), sqlx::Error> {
        sqlx::query(CREATE_CDR_TABLE_SQL)
            .execute(&self.pool)
            .await?;

        // Perform online migration to add new columns to existing database tables if they don't exist
        sqlx::query(
            "ALTER TABLE call_cdrs \
              ADD COLUMN IF NOT EXISTS caller_rtcp_loss_rate DOUBLE PRECISION, \
              ADD COLUMN IF NOT EXISTS caller_rtcp_jitter_ms DOUBLE PRECISION, \
              ADD COLUMN IF NOT EXISTS caller_rtcp_rtt_ms INTEGER, \
              ADD COLUMN IF NOT EXISTS gateway_rtcp_loss_rate DOUBLE PRECISION, \
              ADD COLUMN IF NOT EXISTS gateway_rtcp_jitter_ms DOUBLE PRECISION, \
              ADD COLUMN IF NOT EXISTS gateway_rtcp_rtt_ms INTEGER, \
              ADD COLUMN IF NOT EXISTS mos DOUBLE PRECISION, \
              ADD COLUMN IF NOT EXISTS dtmf_digits TEXT",
        )
        .execute(&self.pool)
        .await?;

        sqlx::query(CREATE_CALL_ID_INDEX_SQL)
            .execute(&self.pool)
            .await?;
        sqlx::query(CREATE_STARTED_AT_INDEX_SQL)
            .execute(&self.pool)
            .await?;
        sqlx::query(CREATE_STATUS_INDEX_SQL)
            .execute(&self.pool)
            .await?;
        sqlx::query(CREATE_SIP_USERS_TABLE_SQL)
            .execute(&self.pool)
            .await?;
        sqlx::query(CREATE_SIP_GATEWAYS_TABLE_SQL)
            .execute(&self.pool)
            .await?;
        sqlx::query(CREATE_SIP_ROUTES_TABLE_SQL)
            .execute(&self.pool)
            .await?;
        sqlx::query(CREATE_SIP_REGISTRATIONS_TABLE_SQL)
            .execute(&self.pool)
            .await?;
        sqlx::query("ALTER TABLE sip_registrations ADD COLUMN IF NOT EXISTS path TEXT")
            .execute(&self.pool)
            .await?;
        // Online migration for gateway capacity and route cost (LCR).
        sqlx::query("ALTER TABLE sip_gateways ADD COLUMN IF NOT EXISTS max_capacity INTEGER")
            .execute(&self.pool)
            .await?;
        sqlx::query(
            "ALTER TABLE sip_routes ADD COLUMN IF NOT EXISTS cost DOUBLE PRECISION NOT NULL DEFAULT 0.0",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query("ALTER TABLE sip_routes ADD COLUMN IF NOT EXISTS time_start TEXT")
            .execute(&self.pool)
            .await?;
        sqlx::query("ALTER TABLE sip_routes ADD COLUMN IF NOT EXISTS time_end TEXT")
            .execute(&self.pool)
            .await?;
        // DTMF audit detail table.
        sqlx::query(CREATE_DTMF_EVENTS_TABLE_SQL)
            .execute(&self.pool)
            .await?;
        sqlx::query(CREATE_DTMF_CALL_ID_INDEX_SQL)
            .execute(&self.pool)
            .await?;
        sqlx::query(CREATE_BILLING_RATES_TABLE_SQL)
            .execute(&self.pool)
            .await?;
        sqlx::query(CREATE_BILLING_ACCOUNTS_TABLE_SQL)
            .execute(&self.pool)
            .await?;
        sqlx::query(CREATE_BILLING_LEDGER_TABLE_SQL)
            .execute(&self.pool)
            .await?;
        sqlx::query(CREATE_LEDGER_USERNAME_INDEX_SQL)
            .execute(&self.pool)
            .await?;
        sqlx::query(CREATE_NUMBER_INVENTORY_TABLE_SQL)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn insert_call_cdr(&self, cdr: &CallCdr) -> Result<(), sqlx::Error> {
        self.insert_event(&CdrEvent::from_call_cdr(cdr)).await
    }

    pub async fn insert_event(&self, event: &CdrEvent) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"
            INSERT INTO call_cdrs (
                call_id,
                caller,
                callee,
                started_at,
                answered_at,
                ended_at,
                duration_ms,
                billable_duration_ms,
                status,
                failure_status_code,
                failure_reason,
                caller_rtcp_loss_rate,
                caller_rtcp_jitter_ms,
                caller_rtcp_rtt_ms,
                gateway_rtcp_loss_rate,
                gateway_rtcp_jitter_ms,
                gateway_rtcp_rtt_ms,
                mos,
                dtmf_digits
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19)
            "#,
        )
        .bind(event.call_id.as_str())
        .bind(event.caller.as_deref())
        .bind(event.callee.as_deref())
        .bind(offset_from_millis(event.started_at_ms))
        .bind(event.answered_at_ms.map(offset_from_millis))
        .bind(offset_from_millis(event.ended_at_ms))
        .bind(event.duration_ms)
        .bind(event.billable_duration_ms)
        .bind(event.status.as_str())
        .bind(event.failure_status_code.map(i32::from))
        .bind(event.failure_reason.as_deref())
        .bind(event.caller_rtcp_loss_rate)
        .bind(event.caller_rtcp_jitter_ms)
        .bind(event.caller_rtcp_rtt_ms.map(|v| v as i32))
        .bind(event.gateway_rtcp_loss_rate)
        .bind(event.gateway_rtcp_jitter_ms)
        .bind(event.gateway_rtcp_rtt_ms.map(|v| v as i32))
        .bind(event.mos)
        .bind(event.dtmf_digits.as_deref())
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn insert_events_batch(&self, events: &[CdrEvent]) -> Result<(), sqlx::Error> {
        if events.is_empty() {
            return Ok(());
        }
        let mut tx = self.pool.begin().await?;

        // Batch in chunks of 50 to avoid exceeding parameter limits
        for chunk in events.chunks(50) {
            use sqlx::QueryBuilder;
            let mut qb: QueryBuilder<'_, sqlx::Postgres> = QueryBuilder::new(
                "INSERT INTO call_cdrs (
                    call_id, caller, callee, started_at, answered_at, ended_at,
                    duration_ms, billable_duration_ms, status, failure_status_code,
                    failure_reason, caller_rtcp_loss_rate, caller_rtcp_jitter_ms,
                    caller_rtcp_rtt_ms, gateway_rtcp_loss_rate, gateway_rtcp_jitter_ms,
                    gateway_rtcp_rtt_ms, mos, dtmf_digits
                ) ",
            );

            for (i, event) in chunk.iter().enumerate() {
                let separator = if i == 0 { "VALUES " } else { ", " };
                qb.push(separator);
                qb.push("(")
                    .push_bind(event.call_id.as_str())
                    .push_bind(event.caller.as_deref())
                    .push_bind(event.callee.as_deref())
                    .push_bind(offset_from_millis(event.started_at_ms))
                    .push_bind(event.answered_at_ms.map(offset_from_millis))
                    .push_bind(offset_from_millis(event.ended_at_ms))
                    .push_bind(event.duration_ms)
                    .push_bind(event.billable_duration_ms)
                    .push_bind(event.status.as_str())
                    .push_bind(event.failure_status_code.map(i32::from))
                    .push_bind(event.failure_reason.as_deref())
                    .push_bind(event.caller_rtcp_loss_rate)
                    .push_bind(event.caller_rtcp_jitter_ms)
                    .push_bind(event.caller_rtcp_rtt_ms.map(|v| v as i32))
                    .push_bind(event.gateway_rtcp_loss_rate)
                    .push_bind(event.gateway_rtcp_jitter_ms)
                    .push_bind(event.gateway_rtcp_rtt_ms.map(|v| v as i32))
                    .push_bind(event.mos)
                    .push_bind(event.dtmf_digits.as_deref())
                    .push(")");
            }

            qb.build().execute(&mut *tx).await?;
        }

        tx.commit().await?;
        Ok(())
    }

    pub async fn get_user_password(&self, username: &str) -> Result<Option<String>, sqlx::Error> {
        let row = sqlx::query("SELECT password FROM sip_users WHERE username = $1")
            .bind(username)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| r.get::<String, _>(0)))
    }

    pub async fn load_gateways(
        &self,
    ) -> Result<Vec<(String, String, Option<u16>, String, Option<u32>)>, sqlx::Error> {
        let rows =
            sqlx::query("SELECT id, host, port, transport, max_capacity FROM sip_gateways")
                .fetch_all(&self.pool)
                .await?;
        let mut gateways = Vec::new();
        for row in rows {
            let id: String = row.get(0);
            let host: String = row.get(1);
            let port_i32: Option<i32> = row.get(2);
            let port = port_i32.and_then(|p| u16::try_from(p).ok());
            let transport: String = row.get(3);
            let cap_i32: Option<i32> = row.get(4);
            let max_capacity = cap_i32.and_then(|c| u32::try_from(c).ok());
            gateways.push((id, host, port, transport, max_capacity));
        }
        Ok(gateways)
    }

    pub async fn load_routes(
        &self,
    ) -> Result<Vec<(String, String, i32, String, f64, Option<String>, Option<String>)>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT id, prefix, priority, gateway_id, cost, time_start, time_end FROM sip_routes",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut routes = Vec::new();
        for row in rows {
            let id: String = row.get(0);
            let prefix: String = row.get(1);
            let priority: i32 = row.get(2);
            let gateway_id: String = row.get(3);
            let cost: f64 = row.get(4);
            let time_start: Option<String> = row.get(5);
            let time_end: Option<String> = row.get(6);
            routes.push((id, prefix, priority, gateway_id, cost, time_start, time_end));
        }
        Ok(routes)
    }

    pub async fn insert_user(&self, username: &str, password: &str) -> Result<(), sqlx::Error> {
        sqlx::query("INSERT INTO sip_users (username, password) VALUES ($1, $2) ON CONFLICT (username) DO UPDATE SET password = EXCLUDED.password")
            .bind(username)
            .bind(password)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn insert_gateway(
        &self,
        id: &str,
        host: &str,
        port: Option<u16>,
        transport: &str,
    ) -> Result<(), sqlx::Error> {
        self.insert_gateway_with_capacity(id, host, port, transport, None)
            .await
    }

    pub async fn insert_gateway_with_capacity(
        &self,
        id: &str,
        host: &str,
        port: Option<u16>,
        transport: &str,
        max_capacity: Option<u32>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query("INSERT INTO sip_gateways (id, host, port, transport, max_capacity) VALUES ($1, $2, $3, $4, $5) ON CONFLICT (id) DO UPDATE SET host = EXCLUDED.host, port = EXCLUDED.port, transport = EXCLUDED.transport, max_capacity = EXCLUDED.max_capacity")
            .bind(id)
            .bind(host)
            .bind(port.map(i32::from))
            .bind(transport)
            .bind(max_capacity.map(i32::try_from).and_then(Result::ok))
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn insert_route(
        &self,
        id: &str,
        prefix: &str,
        priority: i32,
        gateway_id: &str,
    ) -> Result<(), sqlx::Error> {
        self.insert_route_with_cost(id, prefix, priority, gateway_id, 0.0, None, None)
            .await
    }

    pub async fn insert_route_with_cost(
        &self,
        id: &str,
        prefix: &str,
        priority: i32,
        gateway_id: &str,
        cost: f64,
        time_start: Option<&str>,
        time_end: Option<&str>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query("INSERT INTO sip_routes (id, prefix, priority, gateway_id, cost, time_start, time_end) VALUES ($1, $2, $3, $4, $5, $6, $7) ON CONFLICT (id) DO UPDATE SET prefix = EXCLUDED.prefix, priority = EXCLUDED.priority, gateway_id = EXCLUDED.gateway_id, cost = EXCLUDED.cost, time_start = EXCLUDED.time_start, time_end = EXCLUDED.time_end")
            .bind(id)
            .bind(prefix)
            .bind(priority)
            .bind(gateway_id)
            .bind(cost)
            .bind(time_start)
            .bind(time_end)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn get_registrations(
        &self,
        aor: &str,
    ) -> Result<Vec<(String, String, OffsetDateTime, Vec<String>)>, sqlx::Error> {
        let rows = sqlx::query("SELECT contact_uri, received_from, expires_at, path FROM sip_registrations WHERE aor = $1 AND expires_at > now()")
            .bind(aor)
            .fetch_all(&self.pool)
            .await?;
        let mut regs = Vec::new();
        for row in rows {
            let contact_uri: String = row.get(0);
            let received_from: String = row.get(1);
            let expires_at: OffsetDateTime = row.get(2);
            let path_str: Option<String> = row.get(3);
            let path = path_str
                .unwrap_or_default()
                .split(',')
                .map(|s| s.to_string())
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>();
            regs.push((contact_uri, received_from, expires_at, path));
        }
        Ok(regs)
    }

    pub async fn get_all_active_received_from(&self) -> Result<Vec<String>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT DISTINCT received_from FROM sip_registrations WHERE expires_at > now()",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut addrs = Vec::new();
        for row in rows {
            let addr: String = row.get(0);
            addrs.push(addr);
        }
        Ok(addrs)
    }

    pub async fn upsert_registration(
        &self,
        aor: &str,
        contact_uri: &str,
        received_from: &str,
        expires_at: OffsetDateTime,
        path: &[String],
    ) -> Result<(), sqlx::Error> {
        let path_str = path.join(",");
        sqlx::query(
            r#"
            INSERT INTO sip_registrations (aor, contact_uri, received_from, expires_at, path, updated_at)
            VALUES ($1, $2, $3, $4, $5, now())
            ON CONFLICT (aor, contact_uri)
            DO UPDATE SET received_from = EXCLUDED.received_from, expires_at = EXCLUDED.expires_at, path = EXCLUDED.path, updated_at = now()
            "#
        )
        .bind(aor)
        .bind(contact_uri)
        .bind(received_from)
        .bind(expires_at)
        .bind(path_str)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn delete_registration(
        &self,
        aor: &str,
        contact_uri: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM sip_registrations WHERE aor = $1 AND contact_uri = $2")
            .bind(aor)
            .bind(contact_uri)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn delete_all_registrations(&self, aor: &str) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM sip_registrations WHERE aor = $1")
            .bind(aor)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn prune_expired_registrations(&self) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM sip_registrations WHERE expires_at <= now()")
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Insert a single DTMF event record into the audit table.
    pub async fn insert_dtmf_event(
        &self,
        record: &DtmfEventRecord,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"
            INSERT INTO dtmf_events (
                call_id, digit, source, timestamp_ms,
                rtp_timestamp, duration_ms, volume
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            "#,
        )
        .bind(record.call_id.as_str())
        .bind(record.digit.as_str())
        .bind(record.source.as_str())
        .bind(record.timestamp_ms)
        .bind(record.rtp_timestamp.map(|v| v as i64))
        .bind(record.duration_ms.map(i32::from))
        .bind(record.volume.map(i32::from))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Insert multiple DTMF event records in a single transaction.
    pub async fn insert_dtmf_events_batch(
        &self,
        records: &[DtmfEventRecord],
    ) -> Result<(), sqlx::Error> {
        if records.is_empty() {
            return Ok(());
        }
        let mut tx = self.pool.begin().await?;

        for chunk in records.chunks(100) {
            use sqlx::QueryBuilder;
            let mut qb: QueryBuilder<'_, sqlx::Postgres> = QueryBuilder::new(
                "INSERT INTO dtmf_events (call_id, digit, source, timestamp_ms, rtp_timestamp, duration_ms, volume) ",
            );

            for (i, record) in chunk.iter().enumerate() {
                let separator = if i == 0 { "VALUES " } else { ", " };
                qb.push(separator)
                    .push("(")
                    .push_bind(record.call_id.as_str())
                    .push_bind(record.digit.as_str())
                    .push_bind(record.source.as_str())
                    .push_bind(record.timestamp_ms)
                    .push_bind(record.rtp_timestamp.map(|v| v as i64))
                    .push_bind(record.duration_ms.map(i32::from))
                    .push_bind(record.volume.map(i32::from))
                    .push(")");
            }

            qb.build().execute(&mut *tx).await?;
        }

        tx.commit().await?;
        Ok(())
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    // ===== API 查询方法 - 简化版本 =====

    /// 获取 CDR 列表（带分页与筛选）。
    pub async fn list_cdrs(
        &self,
        page: i64,
        page_size: i64,
        status: Option<&str>,
        caller: Option<&str>,
        callee: Option<&str>,
        start_time: Option<OffsetDateTime>,
        end_time: Option<OffsetDateTime>,
    ) -> Result<(Vec<CdrEvent>, i64), sqlx::Error> {
        use sqlx::QueryBuilder;
        let offset = (page - 1) * page_size;

        // 计数
        let mut count_qb: QueryBuilder<'_, sqlx::Postgres> =
            QueryBuilder::new("SELECT COUNT(*) FROM call_cdrs WHERE 1=1");
        if let Some(s) = status {
            count_qb.push(" AND status = ").push_bind(s);
        }
        if let Some(c) = caller {
            count_qb.push(" AND caller ILIKE ").push_bind(format!("%{}%", c));
        }
        if let Some(c) = callee {
            count_qb.push(" AND callee ILIKE ").push_bind(format!("%{}%", c));
        }
        if let Some(st) = start_time {
            count_qb.push(" AND started_at >= ").push_bind(st);
        }
        if let Some(et) = end_time {
            count_qb.push(" AND started_at <= ").push_bind(et);
        }
        let total: i64 = count_qb
            .build_query_scalar::<i64>()
            .fetch_one(&self.pool)
            .await?;

        // 列表
        let mut list_qb: QueryBuilder<'_, sqlx::Postgres> = QueryBuilder::new(
            "SELECT call_id, caller, callee, started_at, answered_at, ended_at,
                    duration_ms, billable_duration_ms, status, failure_status_code,
                    failure_reason, caller_rtcp_loss_rate, caller_rtcp_jitter_ms,
                    caller_rtcp_rtt_ms, gateway_rtcp_loss_rate, gateway_rtcp_jitter_ms,
                    gateway_rtcp_rtt_ms, mos, dtmf_digits
             FROM call_cdrs WHERE 1=1",
        );
        if let Some(s) = status {
            list_qb.push(" AND status = ").push_bind(s);
        }
        if let Some(c) = caller {
            list_qb.push(" AND caller ILIKE ").push_bind(format!("%{}%", c));
        }
        if let Some(c) = callee {
            list_qb.push(" AND callee ILIKE ").push_bind(format!("%{}%", c));
        }
        if let Some(st) = start_time {
            list_qb.push(" AND started_at >= ").push_bind(st);
        }
        if let Some(et) = end_time {
            list_qb.push(" AND started_at <= ").push_bind(et);
        }
        list_qb
            .push(" ORDER BY started_at DESC LIMIT ")
            .push_bind(page_size)
            .push(" OFFSET ")
            .push_bind(offset);

        let rows = list_qb.build().fetch_all(&self.pool).await?;
        let mut events = Vec::with_capacity(rows.len());
        for row in rows {
            events.push(cdr_event_from_row(row)?);
        }

        Ok((events, total))
    }

    /// 获取单个 CDR
    pub async fn get_cdr(&self, call_id: &str) -> Result<Option<CdrEvent>, sqlx::Error> {
        let row = sqlx::query(
            "SELECT call_id, caller, callee, started_at, answered_at, ended_at,
                    duration_ms, billable_duration_ms, status, failure_status_code,
                    failure_reason, caller_rtcp_loss_rate, caller_rtcp_jitter_ms,
                    caller_rtcp_rtt_ms, gateway_rtcp_loss_rate, gateway_rtcp_jitter_ms,
                    gateway_rtcp_rtt_ms, mos, dtmf_digits
             FROM call_cdrs WHERE call_id = $1"
        )
        .bind(call_id)
        .fetch_optional(&self.pool)
        .await?;

        row.map(cdr_event_from_row).transpose()
    }

    /// 获取仪表板统计信息
    pub async fn get_dashboard_stats(&self) -> Result<DashboardStats, sqlx::Error> {
        let today_start = OffsetDateTime::now_utc().date().midnight().assume_utc();

        let row = sqlx::query(
            "SELECT COUNT(*) as total,
                    SUM(CASE WHEN status = 'answered' THEN 1 ELSE 0 END) as answered,
                    SUM(CASE WHEN status = 'canceled' THEN 1 ELSE 0 END) as canceled,
                    SUM(CASE WHEN status = 'failed' THEN 1 ELSE 0 END) as failed,
                    AVG(mos) as avg_mos,
                    AVG(caller_rtcp_loss_rate) as avg_loss_rate,
                    AVG(caller_rtcp_jitter_ms) as avg_jitter
             FROM call_cdrs WHERE started_at >= $1"
        )
        .bind(today_start)
        .fetch_one(&self.pool)
        .await?;

        let total: i64 = row.get(0);
        let answered: Option<i64> = row.get(1);
        let canceled: Option<i64> = row.get(2);
        let failed: Option<i64> = row.get(3);
        let avg_mos: Option<f64> = row.get(4);
        let avg_loss_rate: Option<f64> = row.get(5);
        let avg_jitter: Option<f64> = row.get(6);

        let gateways_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sip_gateways")
            .fetch_one(&self.pool)
            .await?;

        let registrations_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sip_registrations WHERE expires_at > NOW()"
        )
        .fetch_one(&self.pool)
        .await?;

        let answered_num = answered.unwrap_or(0);
        let answer_rate = if total > 0 {
            (answered_num as f64 / total as f64) * 100.0
        } else {
            0.0
        };

        Ok(DashboardStats {
            active_calls: 0,
            today_total_calls: total,
            today_answered_calls: answered_num,
            today_canceled_calls: canceled.unwrap_or(0),
            today_failed_calls: failed.unwrap_or(0),
            answer_rate,
            avg_mos,
            avg_loss_rate,
            avg_jitter_ms: avg_jitter,
            registered_users: registrations_count,
            active_gateways: gateways_count,
        })
    }

    /// 获取今日按小时聚合的呼叫趋势。
    pub async fn get_hourly_trend(&self) -> Result<Vec<HourlyTrend>, sqlx::Error> {
        let today_start = OffsetDateTime::now_utc().date().midnight().assume_utc();
        let rows = sqlx::query(
            "SELECT EXTRACT(HOUR FROM started_at)::int AS h,
                    COUNT(*) AS total,
                    SUM(CASE WHEN status = 'answered' THEN 1 ELSE 0 END) AS answered
             FROM call_cdrs
             WHERE started_at >= $1
             GROUP BY h
             ORDER BY h"
        )
        .bind(today_start)
        .fetch_all(&self.pool)
        .await?;

        // 补齐 0~当前小时 的空档，保证折线图横轴连续。
        let now_hour = OffsetDateTime::now_utc().time().hour() as i32;
        let mut map: std::collections::BTreeMap<i32, (i64, i64)> = std::collections::BTreeMap::new();
        for r in rows {
            let h: i32 = r.get(0);
            let total: i64 = r.get(1);
            let answered: i64 = r.get(2);
            map.insert(h, (total, answered));
        }
        let mut out = Vec::with_capacity((now_hour + 1) as usize);
        for h in 0..=now_hour {
            let (total, answered) = map.remove(&h).unwrap_or((0, 0));
            out.push(HourlyTrend { hour: h, total, answered });
        }
        Ok(out)
    }

    // ===== SIP 用户 CRUD =====

    pub async fn list_users(&self) -> Result<Vec<SipUser>, sqlx::Error> {
        let rows = sqlx::query("SELECT username, created_at FROM sip_users ORDER BY username")
            .fetch_all(&self.pool)
            .await?;

        let mut users = Vec::with_capacity(rows.len());
        for row in rows {
            users.push(SipUser {
                username: row.get(0),
                password: None,
                created_at: row.get(1),
            });
        }
        Ok(users)
    }

    pub async fn delete_user(&self, username: &str) -> Result<bool, sqlx::Error> {
        let result = sqlx::query("DELETE FROM sip_users WHERE username = $1")
            .bind(username)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    // ===== SIP 网关 CRUD =====

    pub async fn list_gateways_full(&self) -> Result<Vec<SipGateway>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT id, host, port, transport, max_capacity, created_at FROM sip_gateways ORDER BY id"
        )
        .fetch_all(&self.pool)
        .await?;

        let mut gateways = Vec::with_capacity(rows.len());
        for row in rows {
            let port_i32: Option<i32> = row.get(2);
            let cap_i32: Option<i32> = row.get(4);
            gateways.push(SipGateway {
                id: row.get(0),
                host: row.get(1),
                port: port_i32.and_then(|p| u16::try_from(p).ok()),
                transport: row.get(3),
                max_capacity: cap_i32.and_then(|c| u32::try_from(c).ok()),
                created_at: row.get(5),
            });
        }
        Ok(gateways)
    }

    pub async fn delete_gateway(&self, id: &str) -> Result<bool, sqlx::Error> {
        let result = sqlx::query("DELETE FROM sip_gateways WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    // ===== SIP 路由 CRUD =====

    pub async fn list_routes_full(&self) -> Result<Vec<SipRoute>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT id, prefix, priority, gateway_id, cost, time_start, time_end, created_at FROM sip_routes ORDER BY priority, id"
        )
        .fetch_all(&self.pool)
        .await?;

        let mut routes = Vec::with_capacity(rows.len());
        for row in rows {
            routes.push(SipRoute {
                id: row.get(0),
                prefix: row.get(1),
                priority: row.get(2),
                gateway_id: row.get(3),
                cost: row.get(4),
                time_start: row.get(5),
                time_end: row.get(6),
                created_at: row.get(7),
            });
        }
        Ok(routes)
    }

    pub async fn delete_route(&self, id: &str) -> Result<bool, sqlx::Error> {
        let result = sqlx::query("DELETE FROM sip_routes WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    // ===== 注册信息查询 =====

    pub async fn list_registrations(&self) -> Result<Vec<SipRegistration>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT aor, contact_uri, received_from, expires_at, path, updated_at
             FROM sip_registrations ORDER BY aor"
        )
        .fetch_all(&self.pool)
        .await?;

        let mut registrations = Vec::with_capacity(rows.len());
        for row in rows {
            let path_str: Option<String> = row.get(4);
            let path = path_str
                .unwrap_or_default()
                .split(',')
                .map(|s| s.to_string())
                .filter(|s| !s.is_empty())
                .collect();
            registrations.push(SipRegistration {
                aor: row.get(0),
                contact_uri: row.get(1),
                received_from: row.get(2),
                expires_at: row.get(3),
                path,
                updated_at: row.get(5),
            });
        }
        Ok(registrations)
    }

    // ===== DTMF 事件查询 =====

    pub async fn get_dtmf_events(&self, call_id: &str) -> Result<Vec<DtmfEventRecord>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT call_id, digit, source, timestamp_ms, rtp_timestamp, duration_ms, volume
             FROM dtmf_events WHERE call_id = $1 ORDER BY timestamp_ms"
        )
        .bind(call_id)
        .fetch_all(&self.pool)
        .await?;

        let mut events = Vec::with_capacity(rows.len());
        for row in rows {
            let source_str: &str = row.get(2);
            let source = if source_str == "rtp" {
                DtmfSource::Rtp
            } else {
                DtmfSource::SipInfo
            };
            let rtp_ts: Option<i64> = row.get(4);
            let dur_ms: Option<i32> = row.get(5);
            let vol: Option<i32> = row.get(6);
            events.push(DtmfEventRecord {
                call_id: row.get(0),
                digit: row.get(1),
                source,
                timestamp_ms: row.get(3),
                rtp_timestamp: rtp_ts.map(|v| v as u32),
                duration_ms: dur_ms.map(|v| v as u16),
                volume: vol.map(|v| v as u8),
            });
        }
        Ok(events)
    }

    // ===== 计费：费率 =====
    pub async fn list_rates(&self) -> Result<Vec<BillingRate>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT id, prefix, rate_per_minute, description, created_at FROM billing_rates \
             ORDER BY length(prefix) DESC, id",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            out.push(BillingRate {
                id: row.get(0),
                prefix: row.get(1),
                rate_per_minute: row.get(2),
                description: row.get(3),
                created_at: row.get(4),
            });
        }
        Ok(out)
    }

    pub async fn upsert_rate(
        &self,
        id: &str,
        prefix: &str,
        rate_per_minute: f64,
        description: Option<&str>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO billing_rates (id, prefix, rate_per_minute, description) VALUES ($1,$2,$3,$4) \
             ON CONFLICT (id) DO UPDATE SET prefix=EXCLUDED.prefix, \
             rate_per_minute=EXCLUDED.rate_per_minute, description=EXCLUDED.description",
        )
        .bind(id)
        .bind(prefix)
        .bind(rate_per_minute)
        .bind(description)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn delete_rate(&self, id: &str) -> Result<bool, sqlx::Error> {
        let r = sqlx::query("DELETE FROM billing_rates WHERE id=$1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() > 0)
    }

    // ===== 计费：账户 =====
    pub async fn list_accounts(&self) -> Result<Vec<BillingAccount>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT username, balance, currency, created_at FROM billing_accounts ORDER BY username",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            out.push(BillingAccount {
                username: row.get(0),
                balance: row.get(1),
                currency: row.get(2),
                created_at: row.get(3),
            });
        }
        Ok(out)
    }

    /// 充值：账户不存在则创建。返回新余额。
    pub async fn credit_account(&self, username: &str, amount: f64) -> Result<f64, sqlx::Error> {
        sqlx::query(
            "INSERT INTO billing_accounts (username, balance) VALUES ($1, $2) \
             ON CONFLICT (username) DO UPDATE SET balance = billing_accounts.balance + $2",
        )
        .bind(username)
        .bind(amount)
        .execute(&self.pool)
        .await?;
        let balance: f64 =
            sqlx::query_scalar("SELECT balance FROM billing_accounts WHERE username=$1")
                .bind(username)
                .fetch_one(&self.pool)
                .await?;
        Ok(balance)
    }

    // ===== 计费：扣费明细 =====
    pub async fn list_ledger(&self, username: Option<&str>) -> Result<Vec<LedgerEntry>, sqlx::Error> {
        let rows = if let Some(u) = username {
            sqlx::query(
                "SELECT id, call_id, username, duration_ms, rate_per_minute, amount, balance_after, created_at \
                 FROM billing_ledger WHERE username=$1 ORDER BY created_at DESC LIMIT 500",
            )
            .bind(u)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query(
                "SELECT id, call_id, username, duration_ms, rate_per_minute, amount, balance_after, created_at \
                 FROM billing_ledger ORDER BY created_at DESC LIMIT 500",
            )
            .fetch_all(&self.pool)
            .await?
        };
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            out.push(LedgerEntry {
                id: row.get(0),
                call_id: row.get(1),
                username: row.get(2),
                duration_ms: row.get(3),
                rate_per_minute: row.get(4),
                amount: row.get(5),
                balance_after: row.get(6),
                created_at: row.get(7),
            });
        }
        Ok(out)
    }

    // ===== 计费：离线对账 =====
    /// 扫描区间内已接通 CDR，按被叫前缀匹配费率，算费扣账并写明细（按 call_id 幂等）。
    pub async fn reconcile_billing(
        &self,
        start: OffsetDateTime,
        end: OffsetDateTime,
    ) -> Result<ReconcileResult, sqlx::Error> {
        let rate_rows = sqlx::query(
            "SELECT prefix, rate_per_minute FROM billing_rates ORDER BY length(prefix) DESC",
        )
        .fetch_all(&self.pool)
        .await?;
        let rates: Vec<(String, f64)> = rate_rows
            .into_iter()
            .map(|r| (r.get::<String, _>(0), r.get::<f64, _>(1)))
            .collect();

        let cdr_rows = sqlx::query(
            "SELECT call_id, caller, callee, billable_duration_ms FROM call_cdrs \
             WHERE status='answered' AND started_at >= $1 AND started_at <= $2",
        )
        .bind(start)
        .bind(end)
        .fetch_all(&self.pool)
        .await?;

        let mut tx = self.pool.begin().await?;
        let mut processed: i64 = 0;
        let mut skipped: i64 = 0;
        let mut total_amount: f64 = 0.0;

        for row in cdr_rows {
            let call_id: String = row.get(0);
            let caller: Option<String> = row.get(1);
            let callee: Option<String> = row.get(2);
            let billable_ms: i64 = row.get(3);

            // 幂等：跳过已对账
            let exists: Option<i64> =
                sqlx::query_scalar("SELECT 1 FROM billing_ledger WHERE call_id=$1")
                    .bind(&call_id)
                    .fetch_optional(&mut *tx)
                    .await?;
            if exists.is_some() {
                skipped += 1;
                continue;
            }

            let user = caller.as_deref().and_then(extract_sip_user).unwrap_or("");
            if user.is_empty() {
                skipped += 1;
                continue;
            }
            let callee_str = callee.unwrap_or_default();
            let rate = match_rate(&callee_str, &rates);
            let amount = rate * billable_ms as f64 / 60000.0;

            let bal: Option<f64> =
                sqlx::query_scalar("SELECT balance FROM billing_accounts WHERE username=$1")
                    .bind(user)
                    .fetch_optional(&mut *tx)
                    .await?;
            if bal.is_none() {
                skipped += 1;
                continue;
            }
            let new_bal = bal.unwrap() - amount;
            sqlx::query("UPDATE billing_accounts SET balance=$1 WHERE username=$2")
                .bind(new_bal)
                .bind(user)
                .execute(&mut *tx)
                .await?;
            sqlx::query(
                "INSERT INTO billing_ledger (call_id, username, duration_ms, rate_per_minute, amount, balance_after) \
                 VALUES ($1,$2,$3,$4,$5,$6)",
            )
            .bind(&call_id)
            .bind(user)
            .bind(billable_ms)
            .bind(rate)
            .bind(amount)
            .bind(new_bal)
            .execute(&mut *tx)
            .await?;
            processed += 1;
            total_amount += amount;
        }
        tx.commit().await?;
        Ok(ReconcileResult {
            processed,
            skipped,
            total_amount,
        })
    }

    // ===== 号码库存 =====
    pub async fn list_numbers(&self) -> Result<Vec<NumberInventory>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT number, username, status, created_at FROM number_inventory ORDER BY number",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            out.push(NumberInventory {
                number: row.get(0),
                username: row.get(1),
                status: row.get(2),
                created_at: row.get(3),
            });
        }
        Ok(out)
    }

    pub async fn upsert_number(
        &self,
        number: &str,
        username: Option<&str>,
        status: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO number_inventory (number, username, status) VALUES ($1, $2, $3) \
             ON CONFLICT (number) DO UPDATE SET username = EXCLUDED.username, status = EXCLUDED.status",
        )
        .bind(number)
        .bind(username)
        .bind(status)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn delete_number(&self, number: &str) -> Result<bool, sqlx::Error> {
        let r = sqlx::query("DELETE FROM number_inventory WHERE number = $1")
            .bind(number)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::{CdrEvent, CREATE_CDR_TABLE_SQL, DEFAULT_CDR_STREAM, DEFAULT_CDR_SUBJECT};
    use call_core::{CallCdr, CallId, CdrStatus, FailureCause};
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn default_subject_name_is_stable() {
        assert_eq!(DEFAULT_CDR_SUBJECT, "vos-rs.cdrs");
        assert_eq!(DEFAULT_CDR_STREAM, "VOS_RS_CDRS");
    }

    #[test]
    fn schema_creates_call_cdrs_table() {
        assert!(CREATE_CDR_TABLE_SQL.contains("CREATE TABLE IF NOT EXISTS call_cdrs"));
        assert!(CREATE_CDR_TABLE_SQL.contains("call_id TEXT NOT NULL"));
        assert!(CREATE_CDR_TABLE_SQL.contains("billable_duration_ms BIGINT NOT NULL"));
        assert!(CREATE_CDR_TABLE_SQL.contains("failure_status_code INTEGER"));
    }

    #[test]
    fn converts_answered_call_cdr_to_event() {
        let cdr = CallCdr {
            call_id: CallId::new("call-1@example.com"),
            caller: Some("sip:1001@example.com".to_string()),
            callee: Some("13800138000".to_string()),
            started_at: UNIX_EPOCH + Duration::from_millis(1_000),
            answered_at: Some(UNIX_EPOCH + Duration::from_millis(1_500)),
            ended_at: UNIX_EPOCH + Duration::from_millis(4_000),
            duration: Duration::from_millis(3_000),
            billable_duration: Duration::from_millis(2_500),
            status: CdrStatus::Answered,
            failure_cause: None,
            caller_rtcp_loss_rate: None,
            caller_rtcp_jitter_ms: None,
            caller_rtcp_rtt_ms: None,
            gateway_rtcp_loss_rate: None,
            gateway_rtcp_jitter_ms: None,
            gateway_rtcp_rtt_ms: None,
            mos: None,
            dtmf_digits: Some("123#".to_string()),
        };

        let event = CdrEvent::from_call_cdr(&cdr);

        assert_eq!(event.call_id, "call-1@example.com");
        assert_eq!(event.caller.as_deref(), Some("sip:1001@example.com"));
        assert_eq!(event.callee.as_deref(), Some("13800138000"));
        assert_eq!(event.started_at_ms, 1000);
        assert_eq!(event.answered_at_ms, Some(1500));
        assert_eq!(event.ended_at_ms, 4000);
        assert_eq!(event.duration_ms, 3000);
        assert_eq!(event.billable_duration_ms, 2500);
        assert_eq!(event.status, "answered");
        assert_eq!(event.failure_status_code, None);
        assert_eq!(event.failure_reason, None);
        assert_eq!(event.dtmf_digits.as_deref(), Some("123#"));
    }

    #[test]
    fn converts_failed_call_cdr_to_event() {
        let cdr = CallCdr {
            call_id: CallId::new("call-failed@example.com"),
            caller: None,
            callee: None,
            started_at: UNIX_EPOCH,
            answered_at: None,
            ended_at: UNIX_EPOCH + Duration::from_millis(250),
            duration: Duration::from_millis(250),
            billable_duration: Duration::ZERO,
            status: CdrStatus::Failed,
            failure_cause: Some(FailureCause {
                status_code: Some(503),
                reason: "gateway unavailable".to_string(),
            }),
            caller_rtcp_loss_rate: None,
            caller_rtcp_jitter_ms: None,
            caller_rtcp_rtt_ms: None,
            gateway_rtcp_loss_rate: None,
            gateway_rtcp_jitter_ms: None,
            gateway_rtcp_rtt_ms: None,
            mos: None,
            dtmf_digits: None,
        };

        let event = CdrEvent::from_call_cdr(&cdr);

        assert_eq!(event.caller, None);
        assert_eq!(event.callee, None);
        assert_eq!(event.answered_at_ms, None);
        assert_eq!(event.failure_status_code, Some(503));
        assert_eq!(event.failure_reason.as_deref(), Some("gateway unavailable"));
        assert_eq!(event.dtmf_digits, None);
    }

    #[test]
    fn round_trips_cdr_event_json() {
        let event = CdrEvent {
            call_id: "call-json@example.com".to_string(),
            caller: Some("caller".to_string()),
            callee: Some("callee".to_string()),
            started_at_ms: 1,
            answered_at_ms: Some(2),
            ended_at_ms: 3,
            duration_ms: 2,
            billable_duration_ms: 1,
            status: "answered".to_string(),
            failure_status_code: None,
            failure_reason: None,
            caller_rtcp_loss_rate: None,
            caller_rtcp_jitter_ms: None,
            caller_rtcp_rtt_ms: None,
            gateway_rtcp_loss_rate: None,
            gateway_rtcp_jitter_ms: None,
            gateway_rtcp_rtt_ms: None,
            mos: None,
            dtmf_digits: Some("987*".to_string()),
        };

        let parsed = CdrEvent::from_json_slice(&event.to_json_bytes()).unwrap();

        assert_eq!(parsed, event);
    }
}
