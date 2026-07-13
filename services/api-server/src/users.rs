use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use cdr_core::SipUser;
use serde::Deserialize;

use crate::{normalize_page, ApiError, AppState, PageQuery, PaginatedResponse};

#[derive(Debug, Deserialize)]
pub struct CreateUserRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateUserRequest {
    pub password: String,
}

pub async fn list_users(
    State(state): State<AppState>,
    Query(query): Query<PageQuery>,
) -> Result<Json<PaginatedResponse<SipUser>>, ApiError> {
    let (page, page_size, offset) = normalize_page(&query);
    let (items, total) = tokio::try_join!(
        state.store.list_users_page(page_size, offset),
        state.store.count_users(),
    )
    .map_err(|e| ApiError {
        error: e.to_string(),
    })?;
    Ok(Json(PaginatedResponse {
        items,
        total,
        page,
        page_size,
    }))
}

pub async fn create_user(
    State(state): State<AppState>,
    Json(req): Json<CreateUserRequest>,
) -> Result<StatusCode, ApiError> {
    // 强制转换为 HA1 哈希，防止明文存储
    let ha1 = format!(
        "{:x}",
        md5::compute(format!("{}:{}:{}", req.username, "vos-rs", req.password).as_bytes())
    );
    state
        .store
        .insert_user(&req.username, &ha1)
        .await
        .map_err(|e| ApiError {
            error: e.to_string(),
        })?;
    Ok(StatusCode::CREATED)
}

pub async fn update_user(
    State(state): State<AppState>,
    Path(username): Path<String>,
    Json(req): Json<UpdateUserRequest>,
) -> Result<StatusCode, ApiError> {
    // 强制转换为 HA1 哈希，防止明文存储
    let ha1 = format!(
        "{:x}",
        md5::compute(format!("{}:{}:{}", username, "vos-rs", req.password).as_bytes())
    );
    state
        .store
        .insert_user(&username, &ha1)
        .await
        .map_err(|e| ApiError {
            error: e.to_string(),
        })?;
    Ok(StatusCode::OK)
}

pub async fn delete_user(
    State(state): State<AppState>,
    Path(username): Path<String>,
) -> Result<StatusCode, ApiError> {
    let deleted = state
        .store
        .delete_user(&username)
        .await
        .map_err(|e| ApiError {
            error: e.to_string(),
        })?;
    if deleted {
        Ok(StatusCode::OK)
    } else {
        Ok(StatusCode::NOT_FOUND)
    }
}
