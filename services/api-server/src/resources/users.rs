use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
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
) -> Result<axum::response::Response, ApiError> {
    let (page, page_size, offset) = normalize_page(&query);
    let (items, total) = tokio::try_join!(
        state.store.list_users_page(page_size, offset),
        state.store.count_users(),
    )
    .map_err(|e| ApiError {
        error: e.to_string(),
    })?;

    if query.export.unwrap_or(false) {
        let headers = vec!["SIP分机号", "创建时间"];
        let mut rows = Vec::new();
        for item in items {
            rows.push(vec![
                item.username.clone(),
                item.created_at.map(|t| t.to_string()).unwrap_or_default(),
            ]);
        }
        return Ok(crate::system::utils::to_csv_response(
            "sip_users.csv",
            &headers,
            &rows,
        ));
    }

    use axum::response::IntoResponse;
    Ok(Json(PaginatedResponse {
        items,
        total,
        page,
        page_size,
    })
    .into_response())
}

pub async fn create_user(
    State(state): State<AppState>,
    Json(req): Json<CreateUserRequest>,
) -> Result<StatusCode, ApiError> {
    let realm = digest_realm(&state).await?;
    // 强制转换为 HA1 哈希，防止明文存储
    let ha1 = format!(
        "{:x}",
        md5::compute(format!("{}:{}:{}", req.username, realm, req.password).as_bytes())
    );
    state
        .store
        .insert_user(&req.username, &ha1)
        .await
        .map_err(|e| ApiError {
            error: e.to_string(),
        })?;
    crate::system::hot_cache::set_auth_user(&state, &req.username, &ha1).await?;
    Ok(StatusCode::CREATED)
}

pub async fn update_user(
    State(state): State<AppState>,
    Path(username): Path<String>,
    Json(req): Json<UpdateUserRequest>,
) -> Result<StatusCode, ApiError> {
    let realm = digest_realm(&state).await?;
    // 强制转换为 HA1 哈希，防止明文存储
    let ha1 = format!(
        "{:x}",
        md5::compute(format!("{}:{}:{}", username, realm, req.password).as_bytes())
    );
    state
        .store
        .insert_user(&username, &ha1)
        .await
        .map_err(|e| ApiError {
            error: e.to_string(),
        })?;
    crate::system::hot_cache::set_auth_user(&state, &username, &ha1).await?;
    Ok(StatusCode::OK)
}

async fn digest_realm(state: &AppState) -> Result<String, ApiError> {
    let realm = sqlx::query_scalar::<_, String>(
        "SELECT config_value FROM system_configs WHERE config_key = 'realm'",
    )
    .fetch_optional(state.store.pool())
    .await
    .map_err(|error| ApiError::internal(format!("读取 SIP realm 失败: {error}")))?
    .unwrap_or_else(|| "vos-rs".to_string());
    if realm.trim().is_empty() {
        return Err(ApiError::internal("SIP realm 不能为空"));
    }
    Ok(realm)
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
        crate::system::hot_cache::delete_auth_user(&state, &username).await?;
        Ok(StatusCode::OK)
    } else {
        Ok(StatusCode::NOT_FOUND)
    }
}
