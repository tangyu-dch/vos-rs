//! 站内通知管理 API 与周期告警扫描入口。

use axum::{
    extract::{Path, Query, State},
    Extension, Json,
};
use serde::{Deserialize, Serialize};

use crate::{system::auth::Claims, ApiError, AppState};

const DEFAULT_PAGE_SIZE: i64 = 20;
const MAX_PAGE_SIZE: i64 = 100;

/// 通知列表查询参数。
#[derive(Debug, Deserialize)]
pub(crate) struct NotificationQuery {
    page: Option<i64>,
    page_size: Option<i64>,
    unread_only: Option<bool>,
}

/// 通知分页响应。
#[derive(Debug, Serialize)]
pub(crate) struct NotificationListResponse {
    items: Vec<cdr_core::Notification>,
    total: i64,
    unread: i64,
    page: i64,
    page_size: i64,
}

/// 未读数量响应。
#[derive(Debug, Serialize)]
pub(crate) struct UnreadCountResponse {
    unread_count: i64,
}

/// 批量操作结果。
#[derive(Debug, Serialize)]
pub(crate) struct NotificationMutationResponse {
    affected: u64,
}

/// 告警扫描结果。
#[derive(Debug, Serialize)]
pub(crate) struct NotificationScanResponse {
    active_alerts: usize,
}

/// 分页列出当前操作员可见的站内通知。
pub(crate) async fn list_notifications(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Query(query): Query<NotificationQuery>,
) -> Result<Json<NotificationListResponse>, ApiError> {
    let (page, page_size, offset) = normalize_notification_page(&query);
    let visible_categories = visible_categories(&state, &claims).await;
    let (items, summary) = state
        .store
        .list_notifications(
            &claims.sub,
            &visible_categories,
            query.unread_only.unwrap_or(false),
            page_size,
            offset,
        )
        .await
        .map_err(notification_error)?;
    Ok(Json(NotificationListResponse {
        items,
        total: summary.total,
        unread: summary.unread,
        page,
        page_size,
    }))
}

/// 返回当前操作员的未读通知数量。
pub(crate) async fn unread_count(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> Result<Json<UnreadCountResponse>, ApiError> {
    let unread = state
        .store
        .count_unread_notifications(&claims.sub, &visible_categories(&state, &claims).await)
        .await
        .map_err(notification_error)?;
    Ok(Json(UnreadCountResponse {
        unread_count: unread,
    }))
}

/// 将单条通知标记为当前操作员已读。
pub(crate) async fn mark_read(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<i64>,
) -> Result<Json<NotificationMutationResponse>, ApiError> {
    let exists = state
        .store
        .mark_notification_read(id, &claims.sub, &visible_categories(&state, &claims).await)
        .await
        .map_err(notification_error)?;
    if !exists {
        return Err(ApiError::not_found("通知不存在"));
    }
    Ok(Json(NotificationMutationResponse { affected: 1 }))
}

/// 将当前全部通知标记为当前操作员已读。
pub(crate) async fn mark_all_read(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> Result<Json<NotificationMutationResponse>, ApiError> {
    let affected = state
        .store
        .mark_all_notifications_read(&claims.sub, &visible_categories(&state, &claims).await)
        .await
        .map_err(notification_error)?;
    Ok(Json(NotificationMutationResponse { affected }))
}

/// 手动触发一次运行数据告警扫描。
pub(crate) async fn scan_now(
    State(state): State<AppState>,
) -> Result<Json<NotificationScanResponse>, ApiError> {
    let active_alerts = scan_and_create_notifications(state.store.as_ref()).await?;
    Ok(Json(NotificationScanResponse { active_alerts }))
}

/// 扫描网关健康、计费余额与近期通话指标，并创建去重告警。
pub(crate) async fn scan_and_create_notifications(
    store: &cdr_core::PostgresCdrStore,
) -> Result<usize, ApiError> {
    store.scan_notifications().await.map_err(notification_error)
}

/// 后台周期扫描入口。单轮失败仅记录日志，不会终止后续扫描。
pub(crate) async fn start_notification_scan_loop(state: AppState) {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
    loop {
        interval.tick().await;
        match scan_and_create_notifications(state.store.as_ref()).await {
            Ok(active_alerts) => {
                tracing::debug!(active_alerts, "站内通知告警扫描完成");
            }
            Err(error) => {
                tracing::error!(error = %error.error, "站内通知告警扫描失败");
            }
        }
    }
}

fn normalize_notification_page(query: &NotificationQuery) -> (i64, i64, i64) {
    let page = query.page.unwrap_or(1).max(1);
    let page_size = query
        .page_size
        .unwrap_or(DEFAULT_PAGE_SIZE)
        .clamp(1, MAX_PAGE_SIZE);
    (page, page_size, (page - 1) * page_size)
}

fn notification_error(error: sqlx::Error) -> ApiError {
    ApiError::internal(format!("通知服务数据库操作失败: {error}"))
}

async fn visible_categories(state: &AppState, claims: &Claims) -> Vec<&'static str> {
    const ALL: &[&str] = &[
        "server",
        "trunk",
        "registration",
        "billing",
        "call_quality",
        "risk_control",
        "security",
        "system",
    ];
    const OPERATIONS: &[&str] = &[
        "server",
        "trunk",
        "registration",
        "call_quality",
        "risk_control",
        "security",
        "system",
    ];
    const FINANCE: &[&str] = &["billing", "system"];
    let snapshot = state.access_snapshot.read().await;
    let permissions = snapshot.role_permissions.get(&claims.role);
    if permissions.is_some_and(|items| items.contains("*")) {
        return ALL.to_vec();
    }
    let can_bill = permissions.is_some_and(|items| {
        items.contains("billing.access_accounts.view")
            || items.contains("billing.egress_accounts.view")
            || items.contains("billing.ledger.view")
    });
    let can_operate = permissions.is_some_and(|items| {
        items.contains("infrastructure.view")
            || items.contains("trunks.view")
            || items.contains("security.view")
    });
    match (can_operate, can_bill) {
        (true, true) => ALL.to_vec(),
        (true, false) => OPERATIONS.to_vec(),
        (false, true) => FINANCE.to_vec(),
        (false, false) => vec!["system"],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notification_page_is_bounded() {
        let query = NotificationQuery {
            page: Some(0),
            page_size: Some(1_000),
            unread_only: Some(true),
        };
        assert_eq!(normalize_notification_page(&query), (1, 100, 0));
    }

    #[test]
    fn notification_page_has_stable_defaults() {
        let query = NotificationQuery {
            page: None,
            page_size: None,
            unread_only: None,
        };
        assert_eq!(normalize_notification_page(&query), (1, 20, 0));
    }
}
