use crate::models::{CdrEvent, DashboardStats, DtmfEventRecord, DtmfSource, HourlyTrend};
use crate::utils;
use crate::PostgresCdrStore;
use sqlx::Row;
use time::OffsetDateTime;

impl PostgresCdrStore {
    pub async fn insert_call_cdr(&self, cdr: &call_core::CallCdr) -> Result<(), sqlx::Error> {
        self.insert_event(&CdrEvent::from_call_cdr(cdr)).await
    }

    pub async fn insert_event(&self, event: &CdrEvent) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"
            INSERT INTO call_cdrs (
                call_id, caller, callee, started_at, answered_at, ended_at,
                duration_ms, billable_duration_ms, status, failure_status_code, failure_reason,
                caller_rtcp_loss_rate, caller_rtcp_jitter_ms, caller_rtcp_rtt_ms,
                gateway_rtcp_loss_rate, gateway_rtcp_jitter_ms, gateway_rtcp_rtt_ms,
                mos, dtmf_digits, recording_path, direction, audit
            ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21,$22)
            ON CONFLICT (call_id) DO NOTHING
            "#,
        )
        .bind(&event.call_id)
        .bind(&event.caller)
        .bind(&event.callee)
        .bind(utils::offset_from_millis(event.started_at_ms))
        .bind(event.answered_at_ms.map(utils::offset_from_millis))
        .bind(utils::offset_from_millis(event.ended_at_ms))
        .bind(event.duration_ms)
        .bind(event.billable_duration_ms)
        .bind(&event.status)
        .bind(event.failure_status_code.map(|c| c as i32))
        .bind(&event.failure_reason)
        .bind(event.caller_rtcp_loss_rate)
        .bind(event.caller_rtcp_jitter_ms)
        .bind(event.caller_rtcp_rtt_ms.map(|v| v as i32))
        .bind(event.gateway_rtcp_loss_rate)
        .bind(event.gateway_rtcp_jitter_ms)
        .bind(event.gateway_rtcp_rtt_ms.map(|v| v as i32))
        .bind(event.mos)
        .bind(&event.dtmf_digits)
        .bind(&event.recording_path)
        .bind(&event.direction)
        .bind(sqlx::types::Json(&event.audit))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn insert_events_batch(&self, events: &[CdrEvent]) -> Result<(), sqlx::Error> {
        if events.is_empty() {
            return Ok(());
        }

        let mut call_ids = Vec::with_capacity(events.len());
        let mut callers = Vec::with_capacity(events.len());
        let mut callees = Vec::with_capacity(events.len());
        let mut started_ats = Vec::with_capacity(events.len());
        let mut answered_ats = Vec::with_capacity(events.len());
        let mut ended_ats = Vec::with_capacity(events.len());
        let mut durations = Vec::with_capacity(events.len());
        let mut billable_durations = Vec::with_capacity(events.len());
        let mut statuses = Vec::with_capacity(events.len());
        let mut failure_codes = Vec::with_capacity(events.len());
        let mut failure_reasons = Vec::with_capacity(events.len());
        let mut caller_loss_rates = Vec::with_capacity(events.len());
        let mut caller_jitters = Vec::with_capacity(events.len());
        let mut caller_rtts = Vec::with_capacity(events.len());
        let mut gateway_loss_rates = Vec::with_capacity(events.len());
        let mut gateway_jitters = Vec::with_capacity(events.len());
        let mut gateway_rtts = Vec::with_capacity(events.len());
        let mut moses = Vec::with_capacity(events.len());
        let mut dtmf_digits_list = Vec::with_capacity(events.len());
        let mut recording_paths = Vec::with_capacity(events.len());
        let mut directions = Vec::with_capacity(events.len());
        let mut audits = Vec::with_capacity(events.len());

        for event in events {
            call_ids.push(event.call_id.clone());
            callers.push(event.caller.clone());
            callees.push(event.callee.clone());
            started_ats.push(utils::offset_from_millis(event.started_at_ms));
            answered_ats.push(event.answered_at_ms.map(utils::offset_from_millis));
            ended_ats.push(utils::offset_from_millis(event.ended_at_ms));
            durations.push(event.duration_ms);
            billable_durations.push(event.billable_duration_ms);
            statuses.push(event.status.clone());
            failure_codes.push(event.failure_status_code.map(|c| c as i32));
            failure_reasons.push(event.failure_reason.clone());
            caller_loss_rates.push(event.caller_rtcp_loss_rate);
            caller_jitters.push(event.caller_rtcp_jitter_ms);
            caller_rtts.push(event.caller_rtcp_rtt_ms.map(|v| v as i32));
            gateway_loss_rates.push(event.gateway_rtcp_loss_rate);
            gateway_jitters.push(event.gateway_rtcp_jitter_ms);
            gateway_rtts.push(event.gateway_rtcp_rtt_ms.map(|v| v as i32));
            moses.push(event.mos);
            dtmf_digits_list.push(event.dtmf_digits.clone());
            recording_paths.push(event.recording_path.clone());
            directions.push(event.direction.clone());
            audits.push(sqlx::types::Json(event.audit.clone()));
        }

        sqlx::query(
            r#"
            INSERT INTO call_cdrs (
                call_id, caller, callee, started_at, answered_at, ended_at,
                duration_ms, billable_duration_ms, status, failure_status_code, failure_reason,
                caller_rtcp_loss_rate, caller_rtcp_jitter_ms, caller_rtcp_rtt_ms,
                gateway_rtcp_loss_rate, gateway_rtcp_jitter_ms, gateway_rtcp_rtt_ms,
                mos, dtmf_digits, recording_path, direction, audit
            )
            SELECT * FROM UNNEST(
                $1::text[], $2::text[], $3::text[], $4::timestamptz[], $5::timestamptz[], $6::timestamptz[],
                $7::int8[], $8::int8[], $9::text[], $10::int4[], $11::text[],
                $12::float8[], $13::float8[], $14::int4[],
                $15::float8[], $16::float8[], $17::int4[],
                $18::float8[], $19::text[], $20::text[], $21::text[], $22::jsonb[]
            )
            ON CONFLICT (call_id) DO NOTHING
            "#
        )
        .bind(&call_ids)
        .bind(&callers)
        .bind(&callees)
        .bind(&started_ats)
        .bind(&answered_ats)
        .bind(&ended_ats)
        .bind(&durations)
        .bind(&billable_durations)
        .bind(&statuses)
        .bind(&failure_codes)
        .bind(&failure_reasons)
        .bind(&caller_loss_rates)
        .bind(&caller_jitters)
        .bind(&caller_rtts)
        .bind(&gateway_loss_rates)
        .bind(&gateway_jitters)
        .bind(&gateway_rtts)
        .bind(&moses)
        .bind(&dtmf_digits_list)
        .bind(&recording_paths)
        .bind(&directions)
        .bind(&audits)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn insert_dtmf_event(&self, event: &DtmfEventRecord) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO dtmf_events (call_id, digit, source, timestamp_ms, rtp_timestamp, duration_ms, volume) \
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(&event.call_id)
        .bind(&event.digit)
        .bind(event.source.as_str())
        .bind(event.timestamp_ms)
        .bind(event.rtp_timestamp.map(|v| v as i64))
        .bind(event.duration_ms.map(|v| v as i32))
        .bind(event.volume.map(|v| v as i32))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn insert_dtmf_events_batch(
        &self,
        events: &[DtmfEventRecord],
    ) -> Result<(), sqlx::Error> {
        if events.is_empty() {
            return Ok(());
        }
        let mut query_builder = sqlx::QueryBuilder::new(
            "INSERT INTO dtmf_events (call_id, digit, source, timestamp_ms, rtp_timestamp, duration_ms, volume) "
        );
        query_builder.push_values(events, |mut b, event| {
            b.push_bind(&event.call_id)
                .push_bind(&event.digit)
                .push_bind(event.source.as_str())
                .push_bind(event.timestamp_ms)
                .push_bind(event.rtp_timestamp.map(|v| v as i64))
                .push_bind(event.duration_ms.map(|v| v as i32))
                .push_bind(event.volume.map(|v| v as i32));
        });
        let query = query_builder.build();
        query.execute(&self.pool).await?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn list_cdrs(
        &self,
        page: i64,
        page_size: i64,
        status: Option<&str>,
        call_id: Option<&str>,
        caller: Option<&str>,
        callee: Option<&str>,
        start: Option<OffsetDateTime>,
        end: Option<OffsetDateTime>,
        before_id: Option<i64>,
    ) -> Result<(Vec<CdrEvent>, i64), sqlx::Error> {
        let offset = (page - 1) * page_size;
        let rows = sqlx::query(
            "SELECT call_id, caller, callee, started_at, answered_at, ended_at, \
              duration_ms, billable_duration_ms, status, failure_status_code, failure_reason, \
              caller_rtcp_loss_rate, caller_rtcp_jitter_ms, caller_rtcp_rtt_ms, \
              gateway_rtcp_loss_rate, gateway_rtcp_jitter_ms, gateway_rtcp_rtt_ms, \
              mos, dtmf_digits, recording_path, direction, audit \
              FROM call_cdrs \
              WHERE ($1::text IS NULL OR status = $1) \
                AND ($2::text IS NULL OR call_id LIKE '%' || $2 || '%') \
                AND ($3::text IS NULL OR caller LIKE '%' || $3 || '%') \
                AND ($4::text IS NULL OR callee LIKE '%' || $4 || '%') \
                AND ($5::timestamptz IS NULL OR started_at >= $5) \
                AND ($6::timestamptz IS NULL OR started_at <= $6) \
                AND ($7::int8 IS NULL OR id < $7) \
              ORDER BY started_at DESC, id DESC \
              LIMIT $8 OFFSET $9",
        )
        .bind(status)
        .bind(call_id)
        .bind(caller)
        .bind(callee)
        .bind(start)
        .bind(end)
        .bind(before_id)
        .bind(page_size)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;

        let count_row = sqlx::query_scalar(
            "SELECT COUNT(*) FROM call_cdrs \
              WHERE ($1::text IS NULL OR status = $1) \
                AND ($2::text IS NULL OR call_id LIKE '%' || $2 || '%') \
                AND ($3::text IS NULL OR caller LIKE '%' || $3 || '%') \
                AND ($4::text IS NULL OR callee LIKE '%' || $4 || '%') \
                AND ($5::timestamptz IS NULL OR started_at >= $5) \
                AND ($6::timestamptz IS NULL OR started_at <= $6) \
                AND ($7::int8 IS NULL OR id < $7)",
        )
        .bind(status)
        .bind(call_id)
        .bind(caller)
        .bind(callee)
        .bind(start)
        .bind(end)
        .bind(before_id)
        .fetch_one(&self.pool)
        .await?;

        let total: i64 = count_row;
        let items: Vec<CdrEvent> = rows.iter().map(utils::cdr_event_from_row).collect();
        Ok((items, total))
    }

    pub async fn get_cdr(&self, call_id: &str) -> Result<Option<CdrEvent>, sqlx::Error> {
        let row = sqlx::query(
            "SELECT call_id, caller, callee, started_at, answered_at, ended_at, \
              duration_ms, billable_duration_ms, status, failure_status_code, failure_reason, \
              caller_rtcp_loss_rate, caller_rtcp_jitter_ms, caller_rtcp_rtt_ms, \
              gateway_rtcp_loss_rate, gateway_rtcp_jitter_ms, gateway_rtcp_rtt_ms, \
              mos, dtmf_digits, recording_path, direction, audit \
              FROM call_cdrs WHERE call_id = $1",
        )
        .bind(call_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|r| utils::cdr_event_from_row(&r)))
    }

    pub async fn get_dashboard_stats(
        &self,
        active_calls: i64,
    ) -> Result<DashboardStats, sqlx::Error> {
        let today_start = time::OffsetDateTime::now_utc()
            .to_offset(time::UtcOffset::from_hms(8, 0, 0).unwrap_or(time::UtcOffset::UTC))
            .replace_time(time::Time::from_hms(0, 0, 0).unwrap_or(time::Time::MIDNIGHT));
        let row = sqlx::query(
            "SELECT \
                COUNT(*) as total, \
                COUNT(*) FILTER (WHERE status = 'answered') as answered, \
                COUNT(*) FILTER (WHERE status = 'canceled') as canceled, \
                COUNT(*) FILTER (WHERE status = 'failed') as failed, \
                AVG(mos) as avg_mos, \
                AVG(caller_rtcp_loss_rate) as avg_loss, \
                AVG(caller_rtcp_jitter_ms) as avg_jitter \
              FROM call_cdrs WHERE started_at >= $1",
        )
        .bind(today_start)
        .fetch_one(&self.pool)
        .await?;

        let total: i64 = row.get(0);
        let answered: i64 = row.get(1);
        let canceled: i64 = row.get(2);
        let failed: i64 = row.get(3);
        let avg_mos: Option<f64> = row.get(4);
        let avg_loss: Option<f64> = row.get(5);
        let avg_jitter: Option<f64> = row.get(6);

        let reg_row: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM sip_registrations WHERE expires_at > now()")
                .fetch_one(&self.pool)
                .await?;

        let gw_row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM sip_gateways")
            .fetch_one(&self.pool)
            .await?;

        let answer_rate = if total > 0 {
            answered as f64 / total as f64
        } else {
            0.0
        };

        Ok(DashboardStats {
            active_calls,
            today_total_calls: total,
            today_answered_calls: answered,
            today_canceled_calls: canceled,
            today_failed_calls: failed,
            answer_rate,
            avg_mos,
            avg_loss_rate: avg_loss,
            avg_jitter_ms: avg_jitter,
            registered_users: reg_row.0,
            active_gateways: gw_row.0,
        })
    }

    pub async fn get_hourly_trend(&self) -> Result<Vec<HourlyTrend>, sqlx::Error> {
        let today_start = time::OffsetDateTime::now_utc()
            .to_offset(time::UtcOffset::from_hms(8, 0, 0).unwrap_or(time::UtcOffset::UTC))
            .replace_time(time::Time::from_hms(0, 0, 0).unwrap_or(time::Time::MIDNIGHT));
        let rows = sqlx::query(
            "SELECT EXTRACT(HOUR FROM started_at)::INTEGER as hour, \
                     COUNT(*) as total, \
                     COUNT(*) FILTER (WHERE status = 'answered') as answered \
              FROM call_cdrs WHERE started_at >= $1 \
              GROUP BY hour ORDER BY hour",
        )
        .bind(today_start)
        .fetch_all(&self.pool)
        .await?;

        let trends: Vec<HourlyTrend> = rows
            .iter()
            .map(|row| HourlyTrend {
                hour: row.get(0),
                total: row.get(1),
                answered: row.get(2),
            })
            .collect();
        Ok(trends)
    }

    pub async fn get_security_and_errors_24h(
        &self,
    ) -> Result<(u64, u64, std::collections::HashMap<String, u64>), sqlx::Error> {
        let day_ago = time::OffsetDateTime::now_utc() - time::Duration::hours(24);
        
        let blocked: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM call_cdrs WHERE started_at >= $1 AND (failure_reason LIKE '%Anti-Fraud%' OR failure_reason LIKE '%Limit%' OR failure_reason LIKE '%ACL%')"
        )
        .bind(day_ago)
        .fetch_one(&self.pool)
        .await?;

        let auth_failed: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM call_cdrs WHERE started_at >= $1 AND (failure_reason LIKE '%Auth%' OR failure_status_code = 401 OR failure_status_code = 403)"
        )
        .bind(day_ago)
        .fetch_one(&self.pool)
        .await?;

        let rows = sqlx::query(
            "SELECT failure_status_code, COUNT(*) FROM call_cdrs WHERE started_at >= $1 AND failure_status_code >= 400 GROUP BY failure_status_code"
        )
        .bind(day_ago)
        .fetch_all(&self.pool)
        .await?;

        let mut breakdown = std::collections::HashMap::new();
        for r in rows {
            let code: Option<i32> = r.get(0);
            let count: i64 = r.get(1);
            if let Some(c) = code {
                breakdown.insert(c.to_string(), count as u64);
            }
        }

        Ok((blocked.0 as u64, auth_failed.0 as u64, breakdown))
    }

    pub async fn get_dtmf_events(
        &self,
        call_id: &str,
    ) -> Result<Vec<DtmfEventRecord>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT call_id, digit, source, timestamp_ms, rtp_timestamp, duration_ms, volume \
              FROM dtmf_events WHERE call_id = $1 ORDER BY timestamp_ms",
        )
        .bind(call_id)
        .fetch_all(&self.pool)
        .await?;
        let mut events = Vec::with_capacity(rows.len());
        for row in rows {
            let source_str: String = row.get(2);
            let source = match source_str.as_str() {
                "rtp" => DtmfSource::Rtp,
                "sip-info" => DtmfSource::SipInfo,
                _ => DtmfSource::SipInfo,
            };
            events.push(DtmfEventRecord {
                call_id: row.get(0),
                digit: row.get(1),
                source,
                timestamp_ms: row.get(3),
                rtp_timestamp: row.get::<Option<i64>, _>(4).map(|v| v as u32),
                duration_ms: row.get::<Option<i32>, _>(5).map(|v| v as u16),
                volume: row.get::<Option<i32>, _>(6).map(|v| v as u8),
            });
        }
        Ok(events)
    }
}
