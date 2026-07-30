use crate::models::SipUser;
use crate::PostgresCdrStore;
use sqlx::Row;

impl PostgresCdrStore {
    /// 插入或更新 SIP 用户，可选关联租户。
    ///
    /// `tenant_id` 为 `Some` 时设置租户关联；为 `None` 时通过 `COALESCE` 保留已有租户关联，
    /// 因此 `update_user` 场景传 `None` 不会清除已设置的 tenant_id。
    pub async fn insert_user(
        &self,
        username: &str,
        password: &str,
        tenant_id: Option<&str>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO sip_users (username, password, tenant_id) VALUES ($1, $2, $3) \
             ON CONFLICT (username) DO UPDATE SET \
                password = EXCLUDED.password, \
                tenant_id = COALESCE(EXCLUDED.tenant_id, sip_users.tenant_id)",
        )
        .bind(username)
        .bind(password)
        .bind(tenant_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn update_user_tenant(
        &self,
        username: &str,
        tenant_id: Option<&str>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE sip_users SET tenant_id = $1 WHERE username = $2")
            .bind(tenant_id)
            .bind(username)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn list_users(&self) -> Result<Vec<SipUser>, sqlx::Error> {
        let rows =
            sqlx::query("SELECT username, tenant_id, created_at FROM sip_users ORDER BY username")
                .fetch_all(&self.pool)
                .await?;
        let mut users = Vec::with_capacity(rows.len());
        for row in rows {
            users.push(SipUser {
                username: row.get(0),
                password: None,
                tenant_id: row.get(1),
                created_at: row.get(2),
            });
        }
        Ok(users)
    }

    /// 按页读取 SIP 用户，可选按关键字和租户在 SQL 层过滤。
    ///
    /// - `q`：对 username 做大小写不敏感的 LIKE 匹配
    /// - `tenant_id`：精确匹配 tenant_id 字段
    pub async fn list_users_page(
        &self,
        limit: i64,
        offset: i64,
        q: Option<&str>,
        tenant_id: Option<&str>,
    ) -> Result<Vec<SipUser>, sqlx::Error> {
        let like = q.map(|s| format!("%{s}%"));
        let rows = sqlx::query(
            "SELECT username, tenant_id, created_at FROM sip_users \
             WHERE ($3::TEXT IS NULL OR LOWER(username) LIKE LOWER($3)) \
               AND ($4::TEXT IS NULL OR tenant_id = $4) \
             ORDER BY username LIMIT $1 OFFSET $2",
        )
        .bind(limit)
        .bind(offset)
        .bind(like)
        .bind(tenant_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| SipUser {
                username: row.get(0),
                password: None,
                tenant_id: row.get(1),
                created_at: row.get(2),
            })
            .collect())
    }

    /// 按租户 ID 读取 SIP 用户列表。
    pub async fn list_users_by_tenant(&self, tenant_id: &str) -> Result<Vec<SipUser>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT username, tenant_id, created_at FROM sip_users WHERE tenant_id = $1 ORDER BY username",
        )
        .bind(tenant_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| SipUser {
                username: row.get(0),
                password: None,
                tenant_id: row.get(1),
                created_at: row.get(2),
            })
            .collect())
    }

    /// 返回与 `list_users_page` 相同过滤条件下的 SIP 用户总数。
    pub async fn count_users(
        &self,
        q: Option<&str>,
        tenant_id: Option<&str>,
    ) -> Result<i64, sqlx::Error> {
        let like = q.map(|s| format!("%{s}%"));
        let row: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM sip_users \
             WHERE ($1::TEXT IS NULL OR LOWER(username) LIKE LOWER($1)) \
               AND ($2::TEXT IS NULL OR tenant_id = $2)",
        )
        .bind(like)
        .bind(tenant_id)
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

    /// 读取全部 SIP 用户凭据，用于启动时预热 Redis 鉴权缓存。
    ///
    /// 返回 `(username, password, tenant_id)` 三元组，`tenant_id` 用于预热
    /// `vos_rs:auth:extension_tenants` 映射，使热路径能按分机关联租户查费率。
    pub async fn list_user_credentials(
        &self,
    ) -> Result<Vec<(String, String, Option<String>)>, sqlx::Error> {
        sqlx::query_as("SELECT username, password, tenant_id FROM sip_users ORDER BY username")
            .fetch_all(&self.pool)
            .await
    }
}
