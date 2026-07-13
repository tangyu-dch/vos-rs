use crate::models::{AuditLog, AuditLogInput};
use crate::PostgresCdrStore;

impl PostgresCdrStore {
    /// 持久化一条管理 API 审计记录。
    pub async fn insert_audit_log(&self, input: &AuditLogInput<'_>) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO api_audit_logs (request_id, username, role, method, path, query_params, request_body, status_code, source_ip) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9::inet)",
        )
        .bind(input.request_id)
        .bind(input.username)
        .bind(input.role)
        .bind(input.method)
        .bind(input.path)
        .bind(input.query_params)
        .bind(input.request_body)
        .bind(i32::from(input.status_code))
        .bind(input.source_ip)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// 分页查询管理 API 审计日志。
    pub async fn list_audit_logs(
        &self,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<AuditLog>, sqlx::Error> {
        sqlx::query_as::<_, AuditLog>(
            "SELECT id, request_id, username, role, method, path, query_params, request_body, status_code, \
                    host(source_ip) AS source_ip, created_at \
             FROM api_audit_logs ORDER BY created_at DESC, id DESC LIMIT $1 OFFSET $2",
        )
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await
    }

    /// 返回审计日志总数，用于管理台分页。
    pub async fn count_audit_logs(&self) -> Result<i64, sqlx::Error> {
        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM api_audit_logs")
            .fetch_one(&self.pool)
            .await?;
        Ok(row.0)
    }
}
