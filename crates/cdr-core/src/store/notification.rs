//! 站内通知存储、阅读回执与基于运行数据的告警扫描。

use crate::PostgresCdrStore;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{Postgres, Row, Transaction};
use time::OffsetDateTime;

/// 通知类别。字符串值是稳定的 API 与数据库契约。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationCategory {
    Server,
    Trunk,
    Registration,
    Billing,
    CallQuality,
    RiskControl,
    Security,
    System,
}

impl NotificationCategory {
    fn as_str(self) -> &'static str {
        match self {
            Self::Server => "server",
            Self::Trunk => "trunk",
            Self::Registration => "registration",
            Self::Billing => "billing",
            Self::CallQuality => "call_quality",
            Self::RiskControl => "risk_control",
            Self::Security => "security",
            Self::System => "system",
        }
    }
}

/// 通知严重程度。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationSeverity {
    Info,
    Warning,
    Critical,
}

impl NotificationSeverity {
    fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Critical => "critical",
        }
    }
}

/// 返回给管理 API 的通知记录。
#[derive(Debug, Clone, Serialize)]
pub struct Notification {
    pub id: i64,
    pub category: NotificationCategory,
    pub severity: NotificationSeverity,
    pub title: String,
    pub content: String,
    pub source: String,
    pub metadata: Value,
    pub is_read: bool,
    pub resolved: bool,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

/// 通知列表计数摘要。
#[derive(Debug, Clone, Copy, Serialize)]
pub struct NotificationSummary {
    pub total: i64,
    pub unread: i64,
}

/// 业务模块创建通知时使用的稳定输入类型。
#[derive(Debug, Clone)]
pub struct CreateNotificationInput {
    pub category: NotificationCategory,
    pub severity: NotificationSeverity,
    pub title: String,
    pub content: String,
    pub source: String,
    pub dedup_key: String,
    pub metadata: Value,
}

#[derive(Debug)]
struct AlertCandidate {
    category: NotificationCategory,
    severity: NotificationSeverity,
    title: String,
    content: String,
    source: String,
    dedup_key: String,
    metadata: Value,
}

impl PostgresCdrStore {
    /// 创建通知；若同一去重键仍处于活动状态，则刷新原通知并返回原 ID。
    pub async fn create_or_refresh_notification(
        &self,
        input: &CreateNotificationInput,
    ) -> Result<i64, sqlx::Error> {
        sqlx::query_scalar(
            "INSERT INTO notifications \
               (category, severity, title, content, source, dedup_key, metadata) \
             VALUES ($1, $2, $3, $4, $5, $6, $7) \
             ON CONFLICT (dedup_key) WHERE resolved_at IS NULL DO UPDATE SET \
               severity = EXCLUDED.severity, title = EXCLUDED.title, \
               content = EXCLUDED.content, metadata = EXCLUDED.metadata, updated_at = now() \
             RETURNING id",
        )
        .bind(input.category.as_str())
        .bind(input.severity.as_str())
        .bind(&input.title)
        .bind(&input.content)
        .bind(&input.source)
        .bind(&input.dedup_key)
        .bind(&input.metadata)
        .fetch_one(&self.pool)
        .await
    }

    /// 按创建时间倒序列出通知；阅读状态按登录操作员独立计算。
    pub async fn list_notifications(
        &self,
        operator: &str,
        visible_categories: &[&str],
        unread_only: bool,
        limit: i64,
        offset: i64,
    ) -> Result<(Vec<Notification>, NotificationSummary), sqlx::Error> {
        let rows = sqlx::query(
            "SELECT n.id, n.category, n.severity, n.title, n.content, n.source, n.metadata, \
                    (r.notification_id IS NOT NULL) AS is_read, \
                    (n.resolved_at IS NOT NULL) AS resolved, n.created_at, n.updated_at \
             FROM notifications n LEFT JOIN notification_reads r \
               ON r.notification_id = n.id AND r.operator = $1 \
             WHERE n.category = ANY($2) AND (NOT $3 OR r.notification_id IS NULL) \
             ORDER BY n.created_at DESC LIMIT $4 OFFSET $5",
        )
        .bind(operator)
        .bind(visible_categories)
        .bind(unread_only)
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;
        let counts: (i64, i64) = sqlx::query_as(
            "SELECT COUNT(*), COUNT(*) FILTER (WHERE r.notification_id IS NULL) \
             FROM notifications n LEFT JOIN notification_reads r \
               ON r.notification_id = n.id AND r.operator = $1 \
             WHERE n.category = ANY($2)",
        )
        .bind(operator)
        .bind(visible_categories)
        .fetch_one(&self.pool)
        .await?;
        Ok((
            rows.into_iter().map(parse_notification).collect(),
            NotificationSummary {
                total: counts.0,
                unread: counts.1,
            },
        ))
    }

    /// 返回指定操作员的未读通知数。
    pub async fn count_unread_notifications(
        &self,
        operator: &str,
        visible_categories: &[&str],
    ) -> Result<i64, sqlx::Error> {
        sqlx::query_scalar(
            "SELECT COUNT(*) FROM notifications n WHERE n.category = ANY($2) AND NOT EXISTS (\
             SELECT 1 FROM notification_reads r \
             WHERE r.notification_id = n.id AND r.operator = $1)",
        )
        .bind(operator)
        .bind(visible_categories)
        .fetch_one(&self.pool)
        .await
    }

    /// 将单条通知标为指定操作员已读；返回通知是否存在。
    pub async fn mark_notification_read(
        &self,
        id: i64,
        operator: &str,
        visible_categories: &[&str],
    ) -> Result<bool, sqlx::Error> {
        let result = sqlx::query(
            "INSERT INTO notification_reads (notification_id, operator) \
             SELECT id, $2 FROM notifications WHERE id = $1 AND category = ANY($3) \
             ON CONFLICT (notification_id, operator) DO NOTHING",
        )
        .bind(id)
        .bind(operator)
        .bind(visible_categories)
        .execute(&self.pool)
        .await?;
        if result.rows_affected() > 0 {
            return Ok(true);
        }
        sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM notifications WHERE id = $1 AND category = ANY($2))",
        )
        .bind(id)
        .bind(visible_categories)
        .fetch_one(&self.pool)
        .await
    }

    /// 将当前全部通知标为指定操作员已读。
    pub async fn mark_all_notifications_read(
        &self,
        operator: &str,
        visible_categories: &[&str],
    ) -> Result<u64, sqlx::Error> {
        sqlx::query(
            "INSERT INTO notification_reads (notification_id, operator) \
             SELECT id, $1 FROM notifications WHERE category = ANY($2) \
             ON CONFLICT (notification_id, operator) DO NOTHING",
        )
        .bind(operator)
        .bind(visible_categories)
        .execute(&self.pool)
        .await
        .map(|result| result.rows_affected())
    }

    /// 扫描真实运行数据并创建或刷新活动告警，返回本轮异常项数。
    pub async fn scan_notifications(&self) -> Result<usize, sqlx::Error> {
        let mut transaction = self.pool.begin().await?;
        let candidates = collect_alert_candidates(&mut transaction).await?;
        resolve_previous_scanner_alerts(&mut transaction, &candidates).await?;
        for candidate in &candidates {
            upsert_alert(&mut transaction, candidate).await?;
        }
        transaction.commit().await?;
        Ok(candidates.len())
    }
}

async fn collect_alert_candidates(
    transaction: &mut Transaction<'_, Postgres>,
) -> Result<Vec<AlertCandidate>, sqlx::Error> {
    let mut candidates = collect_gateway_alerts(transaction).await?;
    let accounts: Vec<(String, f64, f64, String)> = sqlx::query_as(
        "SELECT username, balance::DOUBLE PRECISION, credit_limit::DOUBLE PRECISION, currency \
         FROM billing_accounts \
         WHERE balance + credit_limit <= 10",
    )
    .fetch_all(&mut **transaction)
    .await?;
    candidates.extend(accounts.into_iter().map(balance_alert));
    candidates.extend(collect_quality_alerts(transaction).await?);
    Ok(candidates)
}

async fn collect_gateway_alerts(
    transaction: &mut Transaction<'_, Postgres>,
) -> Result<Vec<AlertCandidate>, sqlx::Error> {
    let gateways: Vec<(String, String, i32)> = sqlx::query_as(
        "SELECT gateway_id, state, consecutive_failures FROM gateway_health_status \
         WHERE circuit_open OR state <> 'closed' OR consecutive_failures >= 3",
    )
    .fetch_all(&mut **transaction)
    .await?;
    Ok(gateways
        .into_iter()
        .map(|(id, state, failures)| AlertCandidate {
            category: NotificationCategory::Trunk,
            severity: NotificationSeverity::Critical,
            title: "中继状态异常".to_string(),
            content: format!("中继 {id} 当前状态为 {state}，已连续失败 {failures} 次"),
            source: "gateway_health_status".to_string(),
            dedup_key: format!("scanner:trunk:{id}"),
            metadata: json!({
                "gateway_id": id,
                "state": state,
                "consecutive_failures": failures
            }),
        })
        .collect())
}

async fn collect_quality_alerts(
    transaction: &mut Transaction<'_, Postgres>,
) -> Result<Vec<AlertCandidate>, sqlx::Error> {
    let (total, average_mos, failed): (i64, Option<f64>, i64) = sqlx::query_as(
        "SELECT COUNT(*), AVG(mos), COUNT(*) FILTER (WHERE status <> 'answered') \
         FROM call_cdrs WHERE started_at >= now() - INTERVAL '15 minutes'",
    )
    .fetch_one(&mut **transaction)
    .await?;
    let mut candidates = Vec::new();
    if let Some(mos) = average_mos.filter(|mos| total >= 3 && *mos < 3.5) {
        candidates.push(AlertCandidate {
            category: NotificationCategory::CallQuality,
            severity: NotificationSeverity::Warning,
            title: "通话质量下降".to_string(),
            content: format!("近十五分钟平均语音质量评分为 {mos:.2}，低于 3.50"),
            source: "call_cdrs".to_string(),
            dedup_key: "scanner:quality:low-mos".to_string(),
            metadata: json!({"average_mos": mos, "sample_count": total}),
        });
    }
    if total >= 10 && failed * 100 / total >= 30 {
        candidates.push(AlertCandidate {
            category: NotificationCategory::Server,
            severity: NotificationSeverity::Critical,
            title: "呼叫失败率异常".to_string(),
            content: format!("近十五分钟共 {total} 通呼叫，其中 {failed} 通未接通"),
            source: "call_cdrs".to_string(),
            dedup_key: "scanner:server:high-failure-rate".to_string(),
            metadata: json!({"total": total, "failed": failed}),
        });
    }
    Ok(candidates)
}

fn balance_alert(
    (username, balance, credit_limit, currency): (String, f64, f64, String),
) -> AlertCandidate {
    let available = balance + credit_limit;
    let severity = if available <= 0.0 {
        NotificationSeverity::Critical
    } else {
        NotificationSeverity::Warning
    };
    AlertCandidate {
        category: NotificationCategory::Billing,
        severity,
        title: "商户可用余额不足".to_string(),
        content: format!("计费账户 {username} 当前可用余额为 {available:.2} {currency}"),
        source: "billing_accounts".to_string(),
        dedup_key: format!("scanner:billing:{username}"),
        metadata: json!({
            "username": username,
            "balance": balance,
            "credit_limit": credit_limit,
            "available": available,
            "currency": currency
        }),
    }
}

async fn resolve_previous_scanner_alerts(
    transaction: &mut Transaction<'_, Postgres>,
    candidates: &[AlertCandidate],
) -> Result<(), sqlx::Error> {
    let active_keys: Vec<&str> = candidates
        .iter()
        .map(|candidate| candidate.dedup_key.as_str())
        .collect();
    sqlx::query(
        "UPDATE notifications SET resolved_at = now(), updated_at = now() \
         WHERE source IN ('gateway_health_status', 'billing_accounts', 'call_cdrs') \
           AND resolved_at IS NULL AND NOT (dedup_key = ANY($1))",
    )
    .bind(active_keys)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn upsert_alert(
    transaction: &mut Transaction<'_, Postgres>,
    candidate: &AlertCandidate,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO notifications \
           (category, severity, title, content, source, dedup_key, metadata) \
         VALUES ($1, $2, $3, $4, $5, $6, $7) \
         ON CONFLICT (dedup_key) WHERE resolved_at IS NULL DO UPDATE SET \
           severity = EXCLUDED.severity, title = EXCLUDED.title, \
           content = EXCLUDED.content, metadata = EXCLUDED.metadata, updated_at = now()",
    )
    .bind(candidate.category.as_str())
    .bind(candidate.severity.as_str())
    .bind(&candidate.title)
    .bind(&candidate.content)
    .bind(&candidate.source)
    .bind(&candidate.dedup_key)
    .bind(&candidate.metadata)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

fn parse_notification(row: sqlx::postgres::PgRow) -> Notification {
    Notification {
        id: row.get("id"),
        category: parse_category(row.get("category")),
        severity: parse_severity(row.get("severity")),
        title: row.get("title"),
        content: row.get("content"),
        source: row.get("source"),
        metadata: row.get("metadata"),
        is_read: row.get("is_read"),
        resolved: row.get("resolved"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

fn parse_category(value: &str) -> NotificationCategory {
    match value {
        "server" => NotificationCategory::Server,
        "trunk" => NotificationCategory::Trunk,
        "registration" => NotificationCategory::Registration,
        "billing" => NotificationCategory::Billing,
        "call_quality" => NotificationCategory::CallQuality,
        "risk_control" => NotificationCategory::RiskControl,
        "security" => NotificationCategory::Security,
        _ => NotificationCategory::System,
    }
}

fn parse_severity(value: &str) -> NotificationSeverity {
    match value {
        "critical" => NotificationSeverity::Critical,
        "warning" => NotificationSeverity::Warning,
        _ => NotificationSeverity::Info,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn balance_alert_includes_credit_limit_in_available_funds() {
        let alert = balance_alert(("merchant-a".to_string(), -5.0, 3.0, "CNY".to_string()));
        assert_eq!(alert.severity, NotificationSeverity::Critical);
        assert_eq!(alert.dedup_key, "scanner:billing:merchant-a");
        assert!(alert.content.contains("-2.00"));
    }

    #[test]
    fn category_contract_covers_all_supported_categories() {
        let categories = [
            NotificationCategory::Server,
            NotificationCategory::Trunk,
            NotificationCategory::Registration,
            NotificationCategory::Billing,
            NotificationCategory::CallQuality,
            NotificationCategory::RiskControl,
            NotificationCategory::Security,
            NotificationCategory::System,
        ];
        assert_eq!(categories.map(NotificationCategory::as_str).len(), 8);
        assert_eq!(NotificationCategory::CallQuality.as_str(), "call_quality");
    }
}
