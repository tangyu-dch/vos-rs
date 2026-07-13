use crate::models::SipUser;
use crate::PostgresCdrStore;
use sqlx::Row;

impl PostgresCdrStore {
    pub async fn insert_user(&self, username: &str, password: &str) -> Result<(), sqlx::Error> {
        sqlx::query("INSERT INTO sip_users (username, password) VALUES ($1, $2) ON CONFLICT (username) DO UPDATE SET password = EXCLUDED.password")
            .bind(username)
            .bind(password)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn list_users(&self) -> Result<Vec<SipUser>, sqlx::Error> {
        let rows = sqlx::query("SELECT username, created_at FROM sip_users ORDER BY username")
            .fetch_all(&self.pool)
            .await?;
        let mut users = Vec::with_capacity(rows.len());
        for row in rows {
            users.push(SipUser {
                username: row.get(0),
                password: None,
                created_at: row.get(1),
            });
        }
        Ok(users)
    }

    /// 按页读取 SIP 用户，避免管理端随数据增长一次性加载全部记录。
    pub async fn list_users_page(
        &self,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<SipUser>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT username, created_at FROM sip_users ORDER BY username LIMIT $1 OFFSET $2",
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| SipUser {
                username: row.get(0),
                password: None,
                created_at: row.get(1),
            })
            .collect())
    }

    /// 返回 SIP 用户总数，用于构造分页响应。
    pub async fn count_users(&self) -> Result<i64, sqlx::Error> {
        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM sip_users")
            .fetch_one(&self.pool)
            .await?;
        Ok(row.0)
    }

    pub async fn delete_user(&self, username: &str) -> Result<bool, sqlx::Error> {
        let result = sqlx::query("DELETE FROM sip_users WHERE username = $1")
            .bind(username)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn get_user_password(&self, username: &str) -> Result<Option<String>, sqlx::Error> {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT password FROM sip_users WHERE username = $1")
                .bind(username)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.map(|(pw,)| pw))
    }
}
