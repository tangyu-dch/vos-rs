//! 公告管理与当前用户公告收件箱 API。

use crate::{system::auth::Claims, ApiError, AppState};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Extension, Json,
};
use serde::{Deserialize, Serialize};

mod validation;

use validation::{validate_payload, AnnouncementPayload};

const DEFAULT_PAGE_SIZE: i64 = 20;
const MAX_PAGE_SIZE: i64 = 100;

/// 公告管理列表查询。
#[derive(Debug, Deserialize)]
pub(crate) struct AnnouncementAdminQuery {
    page: Option<i64>,
    page_size: Option<i64>,
    status: Option<String>,
    category: Option<String>,
    q: Option<String>,
}

/// 当前用户公告列表查询。
#[derive(Debug, Deserialize)]
pub(crate) struct MyAnnouncementQuery {
    page: Option<i64>,
    page_size: Option<i64>,
    unread_only: Option<bool>,
    category: Option<String>,
    q: Option<String>,
}

/// 公告管理分页响应。
#[derive(Debug, Serialize)]
pub(crate) struct AnnouncementListResponse {
    items: Vec<cdr_core::Announcement>,
    total: i64,
    page: i64,
    page_size: i64,
}

/// 当前用户公告分页响应。
#[derive(Debug, Serialize)]
pub(crate) struct MyAnnouncementListResponse {
    items: Vec<cdr_core::Announcement>,
    total: i64,
    unread: i64,
    page: i64,
    page_size: i64,
}

/// 管理端分页列出公告。
pub(crate) async fn list_announcements(
    State(state): State<AppState>,
    Query(query): Query<AnnouncementAdminQuery>,
) -> Result<Json<AnnouncementListResponse>, ApiError> {
    let (page, page_size, offset) = normalize_page(query.page, query.page_size);
    let status = normalize_status_filter(query.status.as_deref())?;
    let category = query
        .category
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty());
    let search = query.q.as_deref().map(str::trim).filter(|v| !v.is_empty());
    let (items, total) = state
        .store
        .list_announcements(status, category, search, page_size, offset)
        .await
        .map_err(announcement_error)?;
    Ok(Json(AnnouncementListResponse {
        items,
        total,
        page,
        page_size,
    }))
}

/// 管理端读取公告详情。
pub(crate) async fn get_announcement(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<cdr_core::Announcement>, ApiError> {
    state
        .store
        .get_announcement(id)
        .await
        .map_err(announcement_error)?
        .map(Json)
        .ok_or_else(|| ApiError::not_found("公告不存在"))
}

/// 创建草稿公告。
pub(crate) async fn create_announcement(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(payload): Json<AnnouncementPayload>,
) -> Result<(StatusCode, Json<cdr_core::Announcement>), ApiError> {
    let input = validate_payload(payload)?;
    ensure_audience_users_exist(&state, &input).await?;
    let item = state
        .store
        .create_announcement(&input, &claims.sub)
        .await
        .map_err(announcement_error)?;
    Ok((StatusCode::CREATED, Json(item)))
}

/// 更新公告内容和投放配置。
pub(crate) async fn update_announcement(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(payload): Json<AnnouncementPayload>,
) -> Result<Json<cdr_core::Announcement>, ApiError> {
    let input = validate_payload(payload)?;
    ensure_audience_users_exist(&state, &input).await?;
    state
        .store
        .update_announcement(id, &input)
        .await
        .map_err(announcement_error)?
        .map(Json)
        .ok_or_else(|| ApiError::not_found("公告不存在"))
}

/// 发布公告；未来计划时间到达前不会出现在用户收件箱。
pub(crate) async fn publish_announcement(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<i64>,
) -> Result<Json<cdr_core::Announcement>, ApiError> {
    state
        .store
        .publish_announcement(id, &claims.sub)
        .await
        .map_err(announcement_error)?
        .map(Json)
        .ok_or_else(|| ApiError::not_found("公告不存在"))
}

/// 删除公告。
pub(crate) async fn delete_announcement(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    if state
        .store
        .delete_announcement(id)
        .await
        .map_err(announcement_error)?
    {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::not_found("公告不存在"))
    }
}

/// 列出当前用户可见且已到投放时间的公告。
pub(crate) async fn list_my_announcements(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Query(query): Query<MyAnnouncementQuery>,
) -> Result<Json<MyAnnouncementListResponse>, ApiError> {
    let (page, page_size, offset) = normalize_page(query.page, query.page_size);
    let search = query.q.as_deref().map(str::trim).filter(|v| !v.is_empty());
    let category = query
        .category
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty());
    let (items, summary) = state
        .store
        .list_visible_announcements(
            &claims.sub,
            query.unread_only.unwrap_or(false),
            search,
            category,
            page_size,
            offset,
        )
        .await
        .map_err(announcement_error)?;
    Ok(Json(MyAnnouncementListResponse {
        items,
        total: summary.total,
        unread: summary.unread,
        page,
        page_size,
    }))
}

/// 读取当前用户可见的公告详情。
pub(crate) async fn get_my_announcement(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<i64>,
) -> Result<Json<cdr_core::Announcement>, ApiError> {
    state
        .store
        .get_visible_announcement(id, &claims.sub)
        .await
        .map_err(announcement_error)?
        .map(Json)
        .ok_or_else(|| ApiError::not_found("公告不存在或不可见"))
}

/// 将当前用户可见的公告标记为已读。
pub(crate) async fn mark_my_announcement_read(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    if state
        .store
        .mark_announcement_read(id, &claims.sub)
        .await
        .map_err(announcement_error)?
    {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::not_found("公告不存在或不可见"))
    }
}

fn normalize_status_filter(status: Option<&str>) -> Result<Option<&str>, ApiError> {
    match status.map(str::trim).filter(|value| !value.is_empty()) {
        Some(status @ ("draft" | "published")) => Ok(Some(status)),
        Some(_) => Err(ApiError::bad_request(
            "参数无效：状态仅支持 draft 或 published",
        )),
        None => Ok(None),
    }
}

fn normalize_page(page: Option<i64>, page_size: Option<i64>) -> (i64, i64, i64) {
    let page = page.unwrap_or(1).max(1);
    let page_size = page_size
        .unwrap_or(DEFAULT_PAGE_SIZE)
        .clamp(1, MAX_PAGE_SIZE);
    (page, page_size, (page - 1) * page_size)
}

fn announcement_error(error: sqlx::Error) -> ApiError {
    ApiError::internal(format!("公告数据库操作失败: {error}"))
}

async fn ensure_audience_users_exist(
    state: &AppState,
    input: &cdr_core::UpsertAnnouncementInput,
) -> Result<(), ApiError> {
    if input.audience != "specified" {
        return Ok(());
    }
    let snapshot = state.access_snapshot.read().await;
    let missing: Vec<&str> = input
        .audience_users
        .iter()
        .map(String::as_str)
        .filter(|username| !snapshot.users.contains_key(*username))
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(ApiError::bad_request(format!(
            "参数无效：指定用户不存在：{}",
            missing.join("、")
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_and_status_filters_are_bounded() {
        assert_eq!(normalize_page(Some(0), Some(1_000)), (1, 100, 0));
        assert_eq!(
            normalize_status_filter(Some("published")).expect("合法状态"),
            Some("published")
        );
        assert!(normalize_status_filter(Some("deleted")).is_err());
    }
}
