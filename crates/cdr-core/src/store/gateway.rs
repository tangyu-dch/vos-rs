use crate::models::SipGateway;
use crate::PostgresCdrStore;
use sqlx::Row;
use time::OffsetDateTime;

impl PostgresCdrStore {
    #[allow(clippy::too_many_arguments)]
    pub async fn save_gateway_health(
        &self,
        gateway_id: &str,
        circuit_open: bool,
        consecutive_failures: i32,
        state: &str,
        last_failure_at: Option<OffsetDateTime>,
        half_open_successes: i32,
        last_probe_at: Option<OffsetDateTime>,
        active_calls: i32,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO gateway_health_status \
             (gateway_id, circuit_open, consecutive_failures, state, last_failure_at, half_open_successes, last_probe_at, active_calls, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, now()) \
             ON CONFLICT (gateway_id) DO UPDATE \
             SET circuit_open = EXCLUDED.circuit_open, \
                 consecutive_failures = EXCLUDED.consecutive_failures, \
                 state = EXCLUDED.state, \
                 last_failure_at = EXCLUDED.last_failure_at, \
                 half_open_successes = EXCLUDED.half_open_successes, \
                 last_probe_at = EXCLUDED.last_probe_at, \
                 active_calls = EXCLUDED.active_calls, \
                 updated_at = now()",
        )
        .bind(gateway_id)
        .bind(circuit_open)
        .bind(consecutive_failures)
        .bind(state)
        .bind(last_failure_at)
        .bind(half_open_successes)
        .bind(last_probe_at)
        .bind(active_calls)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn load_gateway_health_list(
        &self,
    ) -> Result<
        Vec<(
            String,
            bool,
            i32,
            String,
            Option<OffsetDateTime>,
            i32,
            Option<OffsetDateTime>,
            i32,
        )>,
        sqlx::Error,
    > {
        let rows = sqlx::query(
            "SELECT gateway_id, circuit_open, consecutive_failures, state, last_failure_at, half_open_successes, last_probe_at, active_calls \
             FROM gateway_health_status",
        )
        .fetch_all(&self.pool)
        .await?;

        let mut list = Vec::new();
        for row in rows {
            let id: String = row.get(0);
            let open: bool = row.get(1);
            let failures: i32 = row.get(2);
            let state: String = row.get(3);
            let last_failure_at: Option<OffsetDateTime> = row.get(4);
            let half_open_successes: i32 = row.get(5);
            let last_probe_at: Option<OffsetDateTime> = row.get(6);
            let active_calls: i32 = row.get(7);
            list.push((
                id,
                open,
                failures,
                state,
                last_failure_at,
                half_open_successes,
                last_probe_at,
                active_calls,
            ));
        }
        Ok(list)
    }

    pub async fn load_gateways(
        &self,
    ) -> Result<
        Vec<(
            String,
            String,
            Option<u16>,
            String,
            Option<u32>,
            Option<String>,
            Option<String>,
            Option<String>,
        )>,
        sqlx::Error,
    > {
        let rows = sqlx::query(
            "SELECT id, host, port, transport, max_capacity, caller_id_mode, virtual_caller, prefix_rules FROM sip_gateways",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut gateways = Vec::new();
        for row in rows {
            let id: String = row.get(0);
            let host: String = row.get(1);
            let port: Option<i32> = row.get(2);
            let transport: String = row.get(3);
            let max_capacity: Option<i32> = row.get(4);
            let caller_id_mode: Option<String> = row.get(5);
            let virtual_caller: Option<String> = row.get(6);
            let prefix_rules: Option<String> = row.get(7);
            gateways.push((
                id,
                host,
                port.map(|p| p as u16),
                transport,
                max_capacity.and_then(|c| u32::try_from(c).ok()),
                caller_id_mode,
                virtual_caller,
                prefix_rules,
            ));
        }
        Ok(gateways)
    }

    pub async fn load_gateway_number_info(
        &self,
    ) -> Result<Vec<(String, String, Option<i32>, i32)>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT n.gateway_id, n.direction, n.max_concurrent, n.current_concurrent \
             FROM number_inventory n WHERE n.gateway_id IS NOT NULL",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut result = Vec::new();
        for row in rows {
            let gateway_id: String = row.get(0);
            let direction: String = row.get(1);
            let max_concurrent: Option<i32> = row.get(2);
            let current_concurrent: i32 = row.get(3);
            result.push((gateway_id, direction, max_concurrent, current_concurrent));
        }
        Ok(result)
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
        let cap_val = max_capacity.map(|c| c as i32);
        sqlx::query(
            "INSERT INTO sip_gateways (id, host, port, transport, max_capacity) \
             VALUES ($1, $2, $3, $4, $5) \
             ON CONFLICT (id) DO UPDATE \
             SET host = EXCLUDED.host, \
                 port = EXCLUDED.port, \
                 transport = EXCLUDED.transport, \
                 max_capacity = EXCLUDED.max_capacity",
        )
        .bind(id)
        .bind(host)
        .bind(port.map(|p| p as i32))
        .bind(transport)
        .bind(cap_val)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn upsert_gateway_full(&self, gw: &SipGateway) -> Result<(), sqlx::Error> {
        let port_val = gw.port.map(|p| p as i32);
        let cap_val = gw.max_capacity.map(|c| c as i32);
        sqlx::query(
            "INSERT INTO sip_gateways (id, host, port, transport, max_capacity, gateway_type, prefix_rules, supports_registration, caller_id_mode, virtual_caller, max_concurrent, account_id, enabled) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13) \
             ON CONFLICT (id) DO UPDATE \
             SET host = EXCLUDED.host, \
                 port = EXCLUDED.port, \
                 transport = EXCLUDED.transport, \
                 max_capacity = EXCLUDED.max_capacity, \
                 gateway_type = EXCLUDED.gateway_type, \
                 prefix_rules = EXCLUDED.prefix_rules, \
                 supports_registration = EXCLUDED.supports_registration, \
                 caller_id_mode = EXCLUDED.caller_id_mode, \
                 virtual_caller = EXCLUDED.virtual_caller, \
                 max_concurrent = EXCLUDED.max_concurrent, \
                 account_id = EXCLUDED.account_id, \
                 enabled = EXCLUDED.enabled"
        )
        .bind(&gw.id)
        .bind(&gw.host)
        .bind(port_val)
        .bind(&gw.transport)
        .bind(cap_val)
        .bind(&gw.gateway_type)
        .bind(&gw.prefix_rules)
        .bind(gw.supports_registration)
        .bind(&gw.caller_id_mode)
        .bind(&gw.virtual_caller)
        .bind(gw.max_concurrent)
        .bind(gw.account_id)
        .bind(gw.enabled)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_gateways_full(&self) -> Result<Vec<SipGateway>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT g.id, g.host, g.port, g.transport, g.max_capacity, g.gateway_type, g.prefix_rules, \
             g.supports_registration, g.caller_id_mode, g.virtual_caller, g.max_concurrent, g.account_id, \
             g.enabled, g.created_at, h.active_calls, h.state \
             FROM sip_gateways g \
             LEFT JOIN gateway_health_status h ON g.id = h.gateway_id \
             ORDER BY g.id",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut gateways = Vec::with_capacity(rows.len());
        for row in rows {
            let active_calls: Option<i32> = row.get(14);
            let state: Option<String> = row.get(15);
            gateways.push(SipGateway {
                id: row.get(0),
                host: row.get(1),
                port: row.get::<Option<i32>, _>(2).map(|p| p as u16),
                transport: row.get(3),
                max_capacity: row
                    .get::<Option<i32>, _>(4)
                    .and_then(|c| u32::try_from(c).ok()),
                gateway_type: row.get(5),
                prefix_rules: row.get(6),
                supports_registration: row.get(7),
                reg_auth_type: None,
                reg_username: None,
                reg_password: None,
                parent_gateway_id: None,
                caller_id_mode: row.get(8),
                virtual_caller: row.get(9),
                current_concurrent: Some(active_calls.unwrap_or(0)),
                circuit_state: Some(state.unwrap_or_else(|| "closed".to_string())),
                account_id: row.get(10),
                max_concurrent: row.get(11),
                enabled: row.get(12),
                created_at: row.get(13),
            });
        }
        Ok(gateways)
    }

    /// 按页读取网关，并保留实时熔断状态与当前并发数。
    pub async fn list_gateways_page(
        &self,
        limit: i64,
        offset: i64,
        gateway_type: Option<&str>,
    ) -> Result<Vec<SipGateway>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT g.id, g.host, g.port, g.transport, g.max_capacity, g.gateway_type, g.prefix_rules, \
             g.supports_registration, g.caller_id_mode, g.virtual_caller, g.max_concurrent, g.account_id, \
             g.enabled, g.created_at, h.active_calls, h.state \
             FROM sip_gateways g \
             LEFT JOIN gateway_health_status h ON g.id = h.gateway_id \
             WHERE ($3::TEXT IS NULL OR g.gateway_type = $3) \
             ORDER BY g.id LIMIT $1 OFFSET $2",
        )
        .bind(limit)
        .bind(offset)
        .bind(gateway_type)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| SipGateway {
                id: row.get(0),
                host: row.get(1),
                port: row.get::<Option<i32>, _>(2).map(|p| p as u16),
                transport: row.get(3),
                max_capacity: row
                    .get::<Option<i32>, _>(4)
                    .and_then(|capacity| u32::try_from(capacity).ok()),
                gateway_type: row.get(5),
                prefix_rules: row.get(6),
                supports_registration: row.get(7),
                reg_auth_type: None,
                reg_username: None,
                reg_password: None,
                parent_gateway_id: None,
                caller_id_mode: row.get(8),
                virtual_caller: row.get(9),
                current_concurrent: Some(row.get::<Option<i32>, _>(14).unwrap_or(0)),
                circuit_state: Some(
                    row.get::<Option<String>, _>(15)
                        .unwrap_or_else(|| "closed".to_string()),
                ),
                account_id: row.get(10),
                max_concurrent: row.get(11),
                enabled: row.get(12),
                created_at: row.get(13),
            })
            .collect())
    }

    /// 返回网关总数。
    pub async fn count_gateways(&self, gateway_type: Option<&str>) -> Result<i64, sqlx::Error> {
        let row: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM sip_gateways WHERE ($1::TEXT IS NULL OR gateway_type = $1)",
        )
        .bind(gateway_type)
        .fetch_one(&self.pool)
        .await?;
        Ok(row.0)
    }

    pub async fn delete_gateway(&self, id: &str) -> Result<bool, sqlx::Error> {
        let result = sqlx::query("DELETE FROM sip_gateways WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }
}
