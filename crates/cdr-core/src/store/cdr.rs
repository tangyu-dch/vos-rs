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
                mos, dtmf_digits, recording_path, direction
            ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21)
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
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn insert_events_batch(&self, events: &[CdrEvent]) -> Result<(), sqlx::Error> {
        if events.is_empty() {
            return Ok(());
        }
        let mut query_builder = sqlx::QueryBuilder::new(
            r#"
            INSERT INTO call_cdrs (
                call_id, caller, callee, started_at, answered_at, ended_at,
                duration_ms, billable_duration_ms, status, failure_status_code, failure_reason,
                caller_rtcp_loss_rate, caller_rtcp_jitter_ms, caller_rtcp_rtt_ms,
                gateway_rtcp_loss_rate, gateway_rtcp_jitter_ms, gateway_rtcp_rtt_ms,
                mos, dtmf_digits, recording_path, direction
            ) 
            "#,
        );
        query_builder.push_values(events, |mut b, event| {
            b.push_bind(&event.call_id)
                .push_bind(&event.caller)
                .push_bind(&event.callee)
                .push_bind(utils::offset_from_millis(event.started_at_ms))
                .push_bind(event.answered_at_ms.map(utils::offset_from_millis))
                .push_bind(utils::offset_from_millis(event.ended_at_ms))
                .push_bind(event.duration_ms)
                .push_bind(event.billable_duration_ms)
                .push_bind(&event.status)
                .push_bind(event.failure_status_code.map(|c| c as i32))
                .push_bind(&event.failure_reason)
                .push_bind(event.caller_rtcp_loss_rate)
                .push_bind(event.caller_rtcp_jitter_ms)
                .push_bind(event.caller_rtcp_rtt_ms.map(|v| v as i32))
                .push_bind(event.gateway_rtcp_loss_rate)
                .push_bind(event.gateway_rtcp_jitter_ms)
                .push_bind(event.gateway_rtcp_rtt_ms.map(|v| v as i32))
                .push_bind(event.mos)
                .push_bind(&event.dtmf_digits)
                .push_bind(&event.recording_path)
                .push_bind(&event.direction);
        });
        query_builder.push(" ON CONFLICT (call_id) DO NOTHING ");
        let query = query_builder.build();
        query.execute(&self.pool).await?;
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
    ) -> Result<(Vec<CdrEvent>, i64), sqlx::Error> {
        let offset = (page - 1) * page_size;
        let rows = sqlx::query(
            "SELECT call_id, caller, callee, started_at, answered_at, ended_at, \
              duration_ms, billable_duration_ms, status, failure_status_code, failure_reason, \
              caller_rtcp_loss_rate, caller_rtcp_jitter_ms, caller_rtcp_rtt_ms, \
              gateway_rtcp_loss_rate, gateway_rtcp_jitter_ms, gateway_rtcp_rtt_ms, \
              mos, dtmf_digits, recording_path, direction \
              FROM call_cdrs \
              WHERE ($1::text IS NULL OR status = $1) \
                AND ($2::text IS NULL OR call_id LIKE '%' || $2 || '%') \
                AND ($3::text IS NULL OR caller LIKE '%' || $3 || '%') \
                AND ($4::text IS NULL OR callee LIKE '%' || $4 || '%') \
                AND ($5::timestamptz IS NULL OR started_at >= $5) \
                AND ($6::timestamptz IS NULL OR started_at <= $6) \
              ORDER BY started_at DESC \
              LIMIT $7 OFFSET $8",
        )
        .bind(status)
        .bind(call_id)
        .bind(caller)
        .bind(callee)
        .bind(start)
        .bind(end)
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
                AND ($6::timestamptz IS NULL OR started_at <= $6)",
        )
        .bind(status)
        .bind(call_id)
        .bind(caller)
        .bind(callee)
        .bind(start)
        .bind(end)
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
              mos, dtmf_digits, recording_path, direction \
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
