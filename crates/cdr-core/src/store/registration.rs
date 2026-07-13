use crate::models::SipRegistration;
use crate::PostgresCdrStore;
use sqlx::Row;
use time::OffsetDateTime;

impl PostgresCdrStore {
    pub async fn get_registrations(
        &self,
        aor: &str,
    ) -> Result<Vec<(String, String, OffsetDateTime, Vec<String>)>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT contact_uri, received_from, expires_at, path FROM sip_registrations \
             WHERE aor = $1 AND expires_at > now()",
        )
        .bind(aor)
        .fetch_all(&self.pool)
        .await?;
        let mut list = Vec::new();
        for row in rows {
            let contact: String = row.get(0);
            let received: String = row.get(1);
            let expires: OffsetDateTime = row.get(2);
            let path_str: Option<String> = row.get(3);
            let path = path_str
                .unwrap_or_default()
                .split(',')
                .map(|s| s.to_string())
                .filter(|s| !s.is_empty())
                .collect();
            list.push((contact, received, expires, path));
        }
        Ok(list)
    }

    pub async fn get_all_active_received_from(&self) -> Result<Vec<String>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT DISTINCT received_from FROM sip_registrations WHERE expires_at > now()",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut list = Vec::new();
        for row in rows {
            let addr: String = row.get(0);
            list.push(addr);
        }
        Ok(list)
    }

    pub async fn upsert_registration(
        &self,
        aor: &str,
        contact_uri: &str,
        received_from: &str,
        expires_at: OffsetDateTime,
        path: &[String],
    ) -> Result<(), sqlx::Error> {
        let path_str = path.join(",");
        sqlx::query(
            "INSERT INTO sip_registrations (aor, contact_uri, received_from, expires_at, path, updated_at) \
             VALUES ($1, $2, $3, $4, $5, now()) \
             ON CONFLICT (aor, contact_uri) DO UPDATE \
             SET received_from = EXCLUDED.received_from, \
                 expires_at = EXCLUDED.expires_at, \
                 path = EXCLUDED.path, \
                 updated_at = now()",
        )
        .bind(aor)
        .bind(contact_uri)
        .bind(received_from)
        .bind(expires_at)
        .bind(path_str)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn delete_registration(
        &self,
        aor: &str,
        contact_uri: &str,
    ) -> Result<bool, sqlx::Error> {
        let result =
            sqlx::query("DELETE FROM sip_registrations WHERE aor = $1 AND contact_uri = $2")
                .bind(aor)
                .bind(contact_uri)
                .execute(&self.pool)
                .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn delete_all_registrations(&self, aor: &str) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM sip_registrations WHERE aor = $1")
            .bind(aor)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn prune_expired_registrations(&self) -> Result<u64, sqlx::Error> {
        let result = sqlx::query("DELETE FROM sip_registrations WHERE expires_at <= now()")
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected())
    }

    pub async fn list_registrations(&self) -> Result<Vec<SipRegistration>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT aor, contact_uri, received_from, expires_at, path, updated_at \
             FROM sip_registrations WHERE expires_at > now() ORDER BY aor",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut registrations = Vec::with_capacity(rows.len());
        for row in rows {
            let path_str: Option<String> = row.get(4);
            let path = path_str
                .unwrap_or_default()
                .split(',')
                .map(|s| s.to_string())
                .filter(|s| !s.is_empty())
                .collect();
            registrations.push(SipRegistration {
                aor: row.get(0),
                contact_uri: row.get(1),
                received_from: row.get(2),
                expires_at: row.get(3),
                path,
                updated_at: row.get(5),
            });
        }
        Ok(registrations)
    }

    /// 按页读取当前有效注册，并支持 AOR、联系地址和来源地址筛选。
    pub async fn list_registrations_page(
        &self,
        keyword: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<SipRegistration>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT aor, contact_uri, received_from, expires_at, path, updated_at \
             FROM sip_registrations \
             WHERE expires_at > now() \
               AND ($1::TEXT IS NULL OR aor ILIKE '%' || $1 || '%' \
                    OR contact_uri ILIKE '%' || $1 || '%' \
                    OR received_from ILIKE '%' || $1 || '%') \
             ORDER BY aor LIMIT $2 OFFSET $3",
        )
        .bind(keyword)
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| SipRegistration {
                aor: row.get(0),
                contact_uri: row.get(1),
                received_from: row.get(2),
                expires_at: row.get(3),
                path: row
                    .get::<Option<String>, _>(4)
                    .unwrap_or_default()
                    .split(',')
                    .filter(|value| !value.is_empty())
                    .map(str::to_string)
                    .collect(),
                updated_at: row.get(5),
            })
            .collect())
    }

    /// 返回当前有效注册总数，可按关键字筛选。
    pub async fn count_registrations(&self, keyword: Option<&str>) -> Result<i64, sqlx::Error> {
        let row: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM sip_registrations \
             WHERE expires_at > now() \
               AND ($1::TEXT IS NULL OR aor ILIKE '%' || $1 || '%' \
                    OR contact_uri ILIKE '%' || $1 || '%' \
                    OR received_from ILIKE '%' || $1 || '%')",
        )
        .bind(keyword)
        .fetch_one(&self.pool)
        .await?;
        Ok(row.0)
    }
}
