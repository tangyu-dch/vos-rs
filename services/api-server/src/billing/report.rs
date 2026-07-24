use axum::{
    extract::{Query, State},
    http::{header, HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use time::{Duration, OffsetDateTime};

use crate::{parse_dt, AppState};

#[derive(Debug, Deserialize)]
pub struct ReportQuery {
    pub start_time: Option<String>,
    pub end_time: Option<String>,
    pub caller: Option<String>,
    pub callee: Option<String>,
    pub status: Option<String>,
}

#[derive(Serialize)]
pub struct StatusBucket {
    pub status: String,
    pub count: i64,
    pub duration_ms: i64,
}

#[derive(Serialize)]
pub struct DayBucket {
    pub day: String,
    pub total: i64,
    pub answered: i64,
}

#[derive(Serialize)]
pub struct ReportSummary {
    pub start: String,
    pub end: String,
    pub total: i64,
    pub answered: i64,
    pub canceled: i64,
    pub failed: i64,
    pub total_duration_ms: i64,
    pub total_billable_ms: i64,
    pub avg_mos: Option<f64>,
    pub avg_ring_ms: Option<f64>,
    pub avg_setup_ms: Option<f64>,
    pub avg_rtt_ms: Option<f64>,
    pub avg_loss_rate: Option<f64>,
    pub avg_jitter_ms: Option<f64>,
    pub by_status: Vec<StatusBucket>,
    pub by_day: Vec<DayBucket>,
}

fn range_or_default(q: &ReportQuery) -> (OffsetDateTime, OffsetDateTime) {
    let end = q
        .end_time
        .as_deref()
        .and_then(parse_dt)
        .unwrap_or_else(OffsetDateTime::now_utc);
    let start = q
        .start_time
        .as_deref()
        .and_then(parse_dt)
        .unwrap_or(end - Duration::days(7));
    (start, end)
}

pub async fn get_report_summary(
    State(state): State<AppState>,
    Query(q): Query<ReportQuery>,
) -> Result<Json<ReportSummary>, (StatusCode, String)> {
    let (start, end) = range_or_default(&q);
    let pool = state.store.pool();

    let row = sqlx::query(
        "SELECT COUNT(*), \
         SUM(CASE WHEN status='answered' THEN 1 ELSE 0 END), \
         SUM(CASE WHEN status='canceled' THEN 1 ELSE 0 END), \
         SUM(CASE WHEN status='failed' THEN 1 ELSE 0 END), \
         COALESCE(SUM(duration_ms), 0)::bigint, \
         COALESCE(SUM(billable_duration_ms), 0)::bigint, \
         AVG(mos)::double precision, \
         AVG(CASE WHEN answered_at IS NOT NULL THEN EXTRACT(EPOCH FROM (answered_at - started_at)) * 1000 END)::double precision, \
         AVG(CASE WHEN answered_at IS NOT NULL THEN EXTRACT(EPOCH FROM (answered_at - started_at)) * 1000 END)::double precision, \
         AVG(CASE WHEN caller_rtcp_rtt_ms IS NOT NULL THEN caller_rtcp_rtt_ms::double precision END)::double precision, \
         AVG(CASE WHEN caller_rtcp_loss_rate IS NOT NULL THEN caller_rtcp_loss_rate END)::double precision, \
         AVG(CASE WHEN caller_rtcp_jitter_ms IS NOT NULL THEN caller_rtcp_jitter_ms END)::double precision \
         FROM call_cdrs WHERE started_at >= $1 AND started_at <= $2",
    )
    .bind(start)
    .bind(end)
    .fetch_one(pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let total: i64 = row.get(0);
    let answered: Option<i64> = row.get(1);
    let canceled: Option<i64> = row.get(2);
    let failed: Option<i64> = row.get(3);
    let total_duration_ms: i64 = row.get(4);
    let total_billable_ms: i64 = row.get(5);
    let avg_mos: Option<f64> = row.get(6);
    let avg_ring_ms: Option<f64> = row.get(7);
    let avg_setup_ms: Option<f64> = row.get(8);
    let avg_rtt_ms: Option<f64> = row.get(9);
    let avg_loss_rate: Option<f64> = row.get(10);
    let avg_jitter_ms: Option<f64> = row.get(11);

    let status_rows = sqlx::query(
        "SELECT status, COUNT(*), COALESCE(SUM(duration_ms), 0)::bigint \
         FROM call_cdrs WHERE started_at >= $1 AND started_at <= $2 \
         GROUP BY status ORDER BY status",
    )
    .bind(start)
    .bind(end)
    .fetch_all(pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let by_status: Vec<StatusBucket> = status_rows
        .into_iter()
        .map(|r| StatusBucket {
            status: r.get(0),
            count: r.get(1),
            duration_ms: r.get(2),
        })
        .collect();

    let day_rows = sqlx::query(
        "SELECT to_char(date_trunc('day', started_at AT TIME ZONE 'UTC'), 'YYYY-MM-DD') AS day, \
         COUNT(*), \
         SUM(CASE WHEN status='answered' THEN 1 ELSE 0 END) \
         FROM call_cdrs WHERE started_at >= $1 AND started_at <= $2 \
         GROUP BY day ORDER BY day",
    )
    .bind(start)
    .bind(end)
    .fetch_all(pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let by_day: Vec<DayBucket> = day_rows
        .into_iter()
        .map(|r| DayBucket {
            day: r.get(0),
            total: r.get(1),
            answered: r.get(2),
        })
        .collect();

    Ok(Json(ReportSummary {
        start: start
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap(),
        end: end
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap(),
        total,
        answered: answered.unwrap_or(0),
        canceled: canceled.unwrap_or(0),
        failed: failed.unwrap_or(0),
        total_duration_ms,
        total_billable_ms,
        avg_mos,
        avg_ring_ms: avg_ring_ms.map(|v| (v * 100.0).round() / 100.0),
        avg_setup_ms: avg_setup_ms.map(|v| (v * 100.0).round() / 100.0),
        avg_rtt_ms: avg_rtt_ms.map(|v| (v * 100.0).round() / 100.0),
        avg_loss_rate: avg_loss_rate.map(|v| (v * 10000.0).round() / 100.0),
        avg_jitter_ms: avg_jitter_ms.map(|v| (v * 100.0).round() / 100.0),
        by_status,
        by_day,
    }))
}

fn csv_quote(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

pub async fn export_cdrs_csv(
    State(state): State<AppState>,
    Query(q): Query<ReportQuery>,
) -> Result<axum::response::Response, (StatusCode, String)> {
    let (start, end) = range_or_default(&q);
    let pool = state.store.pool();

    let mut query = String::from(
        "SELECT call_id, COALESCE(caller,''), COALESCE(callee,''), \
         started_at, answered_at, ended_at, duration_ms, billable_duration_ms, status, \
         COALESCE(failure_status_code::text,''), COALESCE(failure_reason,''), \
         COALESCE(mos::text,''), COALESCE(dtmf_digits,''), \
         COALESCE(caller_rtcp_rtt_ms::text,''), COALESCE(caller_rtcp_jitter_ms::text,''), \
         COALESCE(caller_rtcp_loss_rate::text,'') \
         FROM call_cdrs WHERE started_at >= $1 AND started_at <= $2",
    );

    if let Some(ref caller) = q.caller {
        if !caller.is_empty() {
            query.push_str(&format!(
                " AND caller LIKE '%{}%'",
                caller.replace('\'', "")
            ));
        }
    }
    if let Some(ref callee) = q.callee {
        if !callee.is_empty() {
            query.push_str(&format!(
                " AND callee LIKE '%{}%'",
                callee.replace('\'', "")
            ));
        }
    }
    if let Some(ref status) = q.status {
        if !status.is_empty() {
            query.push_str(&format!(" AND status = '{}'", status.replace('\'', "")));
        }
    }

    query.push_str(" ORDER BY started_at DESC LIMIT 5000");

    let rows = sqlx::query(&query)
        .bind(start)
        .bind(end)
        .fetch_all(pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let mut csv = String::from(
        "call_id,caller,callee,started_at,answered_at,ended_at,duration_ms,billable_duration_ms,status,failure_status_code,failure_reason,mos,dtmf_digits,ring_ms,rtt_ms,jitter_ms,loss_rate\n",
    );
    for r in rows {
        let call_id: String = r.get(0);
        let caller: String = r.get(1);
        let callee: String = r.get(2);
        let started_at: OffsetDateTime = r.get(3);
        let answered_at: Option<OffsetDateTime> = r.get(4);
        let ended_at: OffsetDateTime = r.get(5);
        let duration_ms: i64 = r.get(6);
        let billable_ms: i64 = r.get(7);
        let status: String = r.get(8);
        let fcode: String = r.get(9);
        let freason: String = r.get(10);
        let mos: String = r.get(11);
        let dtmf: String = r.get(12);
        let rtt: String = r.get(13);
        let jitter: String = r.get(14);
        let loss: String = r.get(15);
        let ring_ms = answered_at
            .map(|a| ((a - started_at).whole_milliseconds()).to_string())
            .unwrap_or_default();
        csv.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}\n",
            csv_quote(&call_id),
            csv_quote(&caller),
            csv_quote(&callee),
            csv_quote(
                &started_at
                    .format(&time::format_description::well_known::Rfc3339)
                    .unwrap()
            ),
            csv_quote(
                &answered_at
                    .map(|a| a
                        .format(&time::format_description::well_known::Rfc3339)
                        .unwrap_or_default())
                    .unwrap_or_default()
            ),
            csv_quote(
                &ended_at
                    .format(&time::format_description::well_known::Rfc3339)
                    .unwrap()
            ),
            duration_ms,
            billable_ms,
            csv_quote(&status),
            csv_quote(&fcode),
            csv_quote(&freason),
            csv_quote(&mos),
            csv_quote(&dtmf),
            ring_ms,
            rtt,
            jitter,
            loss,
        ));
    }

    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        "text/csv; charset=utf-8".parse().unwrap(),
    );
    headers.insert(
        header::CONTENT_DISPOSITION,
        "attachment; filename=\"cdrs.csv\"".parse().unwrap(),
    );
    Ok((StatusCode::OK, headers, csv).into_response())
}
