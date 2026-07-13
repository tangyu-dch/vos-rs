use crate::models::NumberInventory;
use crate::PostgresCdrStore;
use sqlx::Row;

impl PostgresCdrStore {
    pub async fn list_numbers(&self) -> Result<Vec<NumberInventory>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT number, username, gateway_id, direction, max_concurrent, current_concurrent, status, created_at, updated_at FROM number_inventory ORDER BY number",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut numbers = Vec::with_capacity(rows.len());
        for row in rows {
            numbers.push(NumberInventory {
                number: row.get(0),
                username: row.get(1),
                gateway_id: row.get(2),
                direction: row.get(3),
                max_concurrent: row.get(4),
                current_concurrent: row.get(5),
                status: row.get(6),
                created_at: row.get(7),
                updated_at: row.get(8),
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
            "SELECT number, username, gateway_id, direction, max_concurrent, current_concurrent, status, created_at, updated_at \
             FROM number_inventory ORDER BY number LIMIT $1 OFFSET $2",
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
                gateway_id: row.get(2),
                direction: row.get(3),
                max_concurrent: row.get(4),
                current_concurrent: row.get(5),
                status: row.get(6),
                created_at: row.get(7),
                updated_at: row.get(8),
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

    pub async fn upsert_number(
        &self,
        number: &str,
        username: Option<&str>,
        gateway_id: Option<&str>,
        direction: Option<&str>,
        max_concurrent: Option<i32>,
        status: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO number_inventory (number, username, gateway_id, direction, max_concurrent, status, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, now()) \
             ON CONFLICT (number) DO UPDATE SET username=EXCLUDED.username, gateway_id=EXCLUDED.gateway_id, \
             direction=EXCLUDED.direction, max_concurrent=EXCLUDED.max_concurrent, status=EXCLUDED.status, updated_at=now()",
        )
        .bind(number)
        .bind(username)
        .bind(gateway_id)
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
