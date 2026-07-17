use crate::models::SipFlowRecord;
use crate::PostgresCdrStore;
use time::OffsetDateTime;

impl PostgresCdrStore {
    pub async fn insert_sip_flows_batch(&self, records: &[SipFlowRecord]) -> Result<(), sqlx::Error> {
        if records.is_empty() {
            return Ok(());
        }

        let mut call_ids = Vec::with_capacity(records.len());
        let mut methods = Vec::with_capacity(records.len());
        let mut directions = Vec::with_capacity(records.len());
        let mut from_addrs = Vec::with_capacity(records.len());
        let mut to_addrs = Vec::with_capacity(records.len());
        let mut raw_messages = Vec::with_capacity(records.len());
        let mut timestamps = Vec::with_capacity(records.len());

        for record in records {
            call_ids.push(record.call_id.clone());
            methods.push(record.method.clone());
            directions.push(record.direction.clone());
            from_addrs.push(record.from_addr.clone());
            to_addrs.push(record.to_addr.clone());
            raw_messages.push(record.raw_message.clone());
            timestamps.push(record.timestamp);
        }

        sqlx::query(
            r#"
            INSERT INTO sip_flows (
                call_id, method, direction, from_addr, to_addr, raw_message, timestamp
            )
            SELECT * FROM UNNEST(
                $1::text[], $2::text[], $3::text[], $4::text[], $5::text[], $6::text[], $7::timestamptz[]
            )
            "#,
        )
        .bind(&call_ids)
        .bind(&methods)
        .bind(&directions)
        .bind(&from_addrs)
        .bind(&to_addrs)
        .bind(&raw_messages)
        .bind(&timestamps)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn get_sip_flows(&self, call_id: &str) -> Result<Vec<SipFlowRecord>, sqlx::Error> {
        sqlx::query_as::<_, SipFlowRecord>(
            "SELECT id, call_id, method, direction, from_addr, to_addr, raw_message, timestamp \
             FROM sip_flows WHERE call_id = $1 ORDER BY timestamp ASC, id ASC"
        )
        .bind(call_id)
        .fetch_all(&self.pool)
        .await
    }

    pub async fn delete_expired_sip_flows(&self, retention_days: i32) -> Result<u64, sqlx::Error> {
        let threshold = OffsetDateTime::now_utc() - time::Duration::days(retention_days as i64);
        let result = sqlx::query("DELETE FROM sip_flows WHERE timestamp < $1")
            .bind(threshold)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
    }
}
