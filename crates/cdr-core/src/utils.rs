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

pub(crate) fn match_rate(callee: &str, rates: &[(String, f64)]) -> f64 {
    let mut best_match: Option<(&str, f64)> = None;
    let mut fallback: Option<f64> = None;
    for (prefix, rate) in rates {
        if prefix.is_empty() {
            fallback = Some(*rate);
            continue;
        }
        if callee.starts_with(prefix.as_str()) {
            match &best_match {
                Some((best, _)) if best.len() >= prefix.len() => {}
                _ => best_match = Some((prefix.as_str(), *rate)),
            }
        }
    }
    best_match.map(|(_, r)| r).or(fallback).unwrap_or(0.0)
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
        answered_at_ms: row
            .get::<Option<time::OffsetDateTime>, _>(4)
            .map(|ts| ts.unix_timestamp_nanos() as i64 / 1_000_000),
        ended_at_ms: {
            let ts: time::OffsetDateTime = row.get(5);
            ts.unix_timestamp_nanos() as i64 / 1_000_000
        },
        duration_ms: row.get(6),
        billable_duration_ms: row.get(7),
        status: row.get(8),
        failure_status_code: row.get::<Option<i32>, _>(9).map(|v| v as u16),
        failure_reason: row.get(10),
        caller_rtcp_loss_rate: row.get(11),
        caller_rtcp_jitter_ms: row.get(12),
        caller_rtcp_rtt_ms: row.get::<Option<i32>, _>(13).map(|v| v as u32),
        gateway_rtcp_loss_rate: row.get(14),
        gateway_rtcp_jitter_ms: row.get(15),
        gateway_rtcp_rtt_ms: row.get::<Option<i32>, _>(16).map(|v| v as u32),
        mos: row.get(17),
        dtmf_digits: row.get(18),
        recording_path: row.get(19),
        direction: row.get(20),
    }
}

pub fn current_hhmm() -> Option<String> {
    let now = time::OffsetDateTime::now_utc();
    Some(format!("{:02}:{:02}", now.hour(), now.minute()))
}
