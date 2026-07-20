use crate::models::NumberInventory;
use crate::PostgresCdrStore;
use sqlx::Row;

impl PostgresCdrStore {
    /// 加载可用于入站呼叫的号码到分机映射。
    ///
    /// `available` 兼容历史上“绑定了分机但未同步改状态”的号码；明确停用的号码不会加载。
    pub async fn load_number_routes(&self) -> Result<Vec<(String, String)>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT number, username FROM number_inventory \
             WHERE username IS NOT NULL AND BTRIM(username) <> '' \
               AND LOWER(status) IN ('available', 'assigned', 'active')",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| (row.get("number"), row.get("username")))
            .collect())
    }

    pub async fn list_numbers(&self) -> Result<Vec<NumberInventory>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT n.number,n.username,a.source_type,a.source_id,n.gateway_id,n.owner_egress_trunk_id,n.direction,n.max_concurrent,n.current_concurrent,n.status,n.created_at,n.updated_at \
             FROM number_inventory n LEFT JOIN number_allocations a ON a.number=n.number AND a.enabled \
             ORDER BY n.number",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut numbers = Vec::with_capacity(rows.len());
        for row in rows {
            numbers.push(NumberInventory {
                number: row.get(0),
                username: row.get(1),
                allocation_source_type: row.get(2),
                allocation_source_id: row.get(3),
                gateway_id: row.get(4),
                owner_egress_trunk_id: row.get(5),
                direction: row.get(6),
                max_concurrent: row.get(7),
                current_concurrent: row.get(8),
                status: row.get(9),
                created_at: row.get(10),
                updated_at: row.get(11),
            });
        }
        Ok(numbers)
    }

    /// 按页读取号码库存。
    pub async fn list_numbers_page(
        &self,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<NumberInventory>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT n.number,n.username,a.source_type,a.source_id,n.gateway_id,n.owner_egress_trunk_id,n.direction,n.max_concurrent,n.current_concurrent,n.status,n.created_at,n.updated_at \
             FROM number_inventory n LEFT JOIN number_allocations a ON a.number=n.number AND a.enabled \
             ORDER BY n.number LIMIT $1 OFFSET $2",
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| NumberInventory {
                number: row.get(0),
                username: row.get(1),
                allocation_source_type: row.get(2),
                allocation_source_id: row.get(3),
                gateway_id: row.get(4),
                owner_egress_trunk_id: row.get(5),
                direction: row.get(6),
                max_concurrent: row.get(7),
                current_concurrent: row.get(8),
                status: row.get(9),
                created_at: row.get(10),
                updated_at: row.get(11),
            })
            .collect())
    }

    /// 返回号码库存总数。
    pub async fn count_numbers(&self) -> Result<i64, sqlx::Error> {
        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM number_inventory")
            .fetch_one(&self.pool)
            .await?;
        Ok(row.0)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn upsert_number(
        &self,
        number: &str,
        username: Option<&str>,
        gateway_id: Option<&str>,
        owner_egress_trunk_id: Option<&str>,
        direction: Option<&str>,
        max_concurrent: Option<i32>,
        status: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO number_inventory (number, username, gateway_id, owner_egress_trunk_id, direction, max_concurrent, status, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, now()) \
             ON CONFLICT (number) DO UPDATE SET username=EXCLUDED.username, gateway_id=EXCLUDED.gateway_id, \
             owner_egress_trunk_id=EXCLUDED.owner_egress_trunk_id, direction=EXCLUDED.direction, \
             max_concurrent=EXCLUDED.max_concurrent, status=EXCLUDED.status, updated_at=now()",
        )
        .bind(number)
        .bind(username)
        .bind(gateway_id)
        .bind(owner_egress_trunk_id)
        .bind(direction)
        .bind(max_concurrent)
        .bind(status)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn delete_number(&self, number: &str) -> Result<bool, sqlx::Error> {
        let r = sqlx::query("DELETE FROM number_inventory WHERE number=$1")
            .bind(number)
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() > 0)
    }
}
