use crate::models::SipRoute;
use crate::PostgresCdrStore;
use sqlx::Row;

impl PostgresCdrStore {
    pub async fn load_routes(
        &self,
    ) -> Result<
        Vec<(
            String,
            String,
            i32,
            String,
            f64,
            i32,
            Option<String>,
            Option<String>,
        )>,
        sqlx::Error,
    > {
        let rows = sqlx::query(
            "SELECT id, prefix, priority, gateway_id, cost, weight, time_start, time_end FROM sip_routes",
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
            let weight: i32 = row.get(5);
            let time_start: Option<String> = row.get(6);
            let time_end: Option<String> = row.get(7);
            routes.push((
                id, prefix, priority, gateway_id, cost, weight, time_start, time_end,
            ));
        }
        Ok(routes)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn insert_route_with_cost(
        &self,
        id: &str,
        prefix: &str,
        priority: i32,
        gateway_id: &str,
        cost: f64,
        weight: i32,
        time_start: Option<&str>,
        time_end: Option<&str>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO sip_routes (id, prefix, priority, gateway_id, cost, weight, time_start, time_end) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
             ON CONFLICT (id) DO UPDATE \
             SET prefix = EXCLUDED.prefix, \
                 priority = EXCLUDED.priority, \
                 gateway_id = EXCLUDED.gateway_id, \
                 cost = EXCLUDED.cost, \
                 weight = EXCLUDED.weight, \
                 time_start = EXCLUDED.time_start, \
                 time_end = EXCLUDED.time_end"
        )
        .bind(id)
        .bind(prefix)
        .bind(priority)
        .bind(gateway_id)
        .bind(cost)
        .bind(weight)
        .bind(time_start)
        .bind(time_end)
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
        self.insert_route_with_cost(id, prefix, priority, gateway_id, 0.0, 1, None, None)
            .await
    }

    pub async fn list_routes_full(&self) -> Result<Vec<SipRoute>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT id, prefix, priority, gateway_id, cost, weight, time_start, time_end, created_at FROM sip_routes ORDER BY priority, id"
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
                weight: row.get(5),
                time_start: row.get(6),
                time_end: row.get(7),
                created_at: row.get(8),
            });
        }
        Ok(routes)
    }

    /// 按页读取路由规则，排序规则与路由引擎保持一致。
    pub async fn list_routes_page(
        &self,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<SipRoute>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT id, prefix, priority, gateway_id, cost, weight, time_start, time_end, created_at \
              FROM sip_routes ORDER BY priority, id LIMIT $1 OFFSET $2",
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| SipRoute {
                id: row.get(0),
                prefix: row.get(1),
                priority: row.get(2),
                gateway_id: row.get(3),
                cost: row.get(4),
                weight: row.get(5),
                time_start: row.get(6),
                time_end: row.get(7),
                created_at: row.get(8),
            })
            .collect())
    }

    /// 返回路由规则总数。
    pub async fn count_routes(&self) -> Result<i64, sqlx::Error> {
        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM sip_routes")
            .fetch_one(&self.pool)
            .await?;
        Ok(row.0)
    }

    pub async fn delete_route(&self, id: &str) -> Result<bool, sqlx::Error> {
        let result = sqlx::query("DELETE FROM sip_routes WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }
}
