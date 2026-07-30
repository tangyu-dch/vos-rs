use std::time::{Duration, SystemTime, UNIX_EPOCH};
use time::OffsetDateTime;

pub(crate) fn system_time_millis(value: SystemTime) -> i64 {
    value
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

pub(crate) fn duration_millis(d: Duration) -> i64 {
    d.as_millis() as i64
}

pub(crate) fn offset_from_millis(millis: i64) -> OffsetDateTime {
    let secs = millis / 1000;
    let nanos = ((millis % 1000) * 1_000_000) as u32;
    OffsetDateTime::from_unix_timestamp(secs).unwrap_or(OffsetDateTime::UNIX_EPOCH)
        + time::Duration::nanoseconds(nanos as i64)
}

#[cfg(test)]
pub(crate) fn extract_sip_user(value: &str) -> Option<&str> {
    let idx = value.find("sip:")?;
    let rest = &value[idx + 4..];
    let end = rest.find(['@', ';', '>']).unwrap_or(rest.len());
    if end == 0 {
        None
    } else {
        Some(&rest[..end])
    }
}

pub(crate) fn cdr_event_from_row(row: &sqlx::postgres::PgRow) -> crate::models::CdrEvent {
    use sqlx::Row;
    crate::models::CdrEvent {
        call_id: row.get(0),
        caller: row.get(1),
        callee: row.get(2),
        started_at_ms: {
            let ts: time::OffsetDateTime = row.get(3);
            ts.unix_timestamp_nanos() as i64 / 1_000_000
        },
        ringing_at_ms: row
            .get::<Option<time::OffsetDateTime>, _>(4)
            .map(|ts| ts.unix_timestamp_nanos() as i64 / 1_000_000),
        answered_at_ms: row
            .get::<Option<time::OffsetDateTime>, _>(5)
            .map(|ts| ts.unix_timestamp_nanos() as i64 / 1_000_000),
        ended_at_ms: {
            let ts: time::OffsetDateTime = row.get(6);
            ts.unix_timestamp_nanos() as i64 / 1_000_000
        },
        duration_ms: row.get(7),
        billable_duration_ms: row.get(8),
        talk_duration_ms: row.get(9),
        ringing_duration_ms: row.get(10),
        access_billable_duration_ms: row.get(11),
        access_charge_amount: row.get(12),
        egress_billable_duration_ms: row.get(13),
        egress_cost_amount: row.get(14),
        status: row.get(15),
        failure_status_code: row.get::<Option<i32>, _>(16).map(|v| v as u16),
        failure_reason: row.get(17),
        caller_rtcp_loss_rate: row.get(18),
        caller_rtcp_jitter_ms: row.get(19),
        caller_rtcp_rtt_ms: row.get::<Option<i32>, _>(20).map(|v| v as u32),
        gateway_rtcp_loss_rate: row.get(21),
        gateway_rtcp_jitter_ms: row.get(22),
        gateway_rtcp_rtt_ms: row.get::<Option<i32>, _>(23).map(|v| v as u32),
        mos: row.get(24),
        dtmf_digits: row.get(25),
        recording_path: row.get(26),
        direction: row.get(27),
        tenant_id: row.get(28),
        tenant_name: row.get(29),
        auth_realm: row.get(30),
        audit: row
            .get::<sqlx::types::Json<call_core::CdrAuditSnapshot>, _>(31)
            .0,
    }
}

pub fn current_hhmm() -> Option<String> {
    let now = time::OffsetDateTime::now_utc();
    Some(format!("{:02}:{:02}", now.hour(), now.minute()))
}
