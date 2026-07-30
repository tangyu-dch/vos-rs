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
    /// 关联租户 ID（可空，不传则不关联租户）。
    pub tenant_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateUserRequest {
    /// 注册密码（编辑时可空，留空表示不修改密码）。
    pub password: Option<String>,
    /// 更新关联租户 ID（可空，不传则保留原值）。
    pub tenant_id: Option<String>,
}

/// 分机列表查询参数：在通用分页基础上增加关键字与租户筛选
#[derive(Debug, Deserialize)]
pub struct ListUsersQuery {
    #[serde(flatten)]
    pub page: PageQuery,
    /// 按分机号模糊搜索
    pub q: Option<String>,
    /// 按租户 ID 精确过滤
    pub tenant_id: Option<String>,
}

pub async fn list_users(
    State(state): State<AppState>,
    Query(query): Query<ListUsersQuery>,
) -> Result<axum::response::Response, ApiError> {
    let (page, page_size, offset) = normalize_page(&query.page);
    let q_trim = query.q.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let tenant_trim = query
        .tenant_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let (items, total) = tokio::try_join!(
        state
            .store
            .list_users_page(page_size, offset, q_trim, tenant_trim),
        state.store.count_users(q_trim, tenant_trim),
    )
    .map_err(|e| ApiError {
        error: e.to_string(),
    })?;

    if query.page.export.unwrap_or(false) {
        let headers = vec!["SIP分机号", "租户ID", "创建时间"];
        let mut rows = Vec::new();
        for item in &items {
            rows.push(vec![
                item.username.clone(),
                item.tenant_id.clone().unwrap_or_default(),
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
    // 检查相同鉴权域（同一租户或系统默认域）下分机号是否已存在
    if let Ok(users) = state
        .store
        .list_users_page(1000, 0, Some(&req.username), req.tenant_id.as_deref())
        .await
    {
        if users.iter().any(|u| u.username == req.username) {
            return Err(ApiError::bad_request("同鉴权域（租户）下分机号已存在"));
        }
    }

    let realm = digest_realm(&state, req.tenant_id.as_deref()).await?;
    // 强制转换为 HA1 哈希，防止明文存储
    let ha1 = format!(
        "{:x}",
        md5::compute(format!("{}:{}:{}", req.username, realm, req.password).as_bytes())
    );
    state
        .store
        .insert_user(&req.username, &ha1, req.tenant_id.as_deref())
        .await
        .map_err(|e| ApiError {
            error: e.to_string(),
        })?;
    crate::system::hot_cache::set_auth_user(&state, &req.username, &ha1).await?;
    // 同步更新 Redis 中的分机-租户映射，使热路径能按租户查找费率
    crate::system::hot_cache::set_extension_tenant(&state, &req.username, req.tenant_id.as_deref())
        .await?;
    Ok(StatusCode::CREATED)
}

pub async fn update_user(
    State(state): State<AppState>,
    Path(username): Path<String>,
    Json(req): Json<UpdateUserRequest>,
) -> Result<StatusCode, ApiError> {
    let effective_tenant_id = match req.tenant_id {
        Some(ref tid) => Some(tid.clone()),
        None => {
            if let Ok(users) = state
                .store
                .list_users_page(1, 0, Some(&username), None)
                .await
            {
                users
                    .into_iter()
                    .find(|u| u.username == username)
                    .and_then(|u| u.tenant_id)
            } else {
                None
            }
        }
    };

    let new_pwd = req
        .password
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if let Some(pwd) = new_pwd {
        let realm = digest_realm(&state, effective_tenant_id.as_deref()).await?;
        let ha1 = format!(
            "{:x}",
            md5::compute(format!("{}:{}:{}", username, realm, pwd).as_bytes())
        );
        state
            .store
            .insert_user(&username, &ha1, req.tenant_id.as_deref())
            .await
            .map_err(|e| ApiError {
                error: e.to_string(),
            })?;
        crate::system::hot_cache::set_auth_user(&state, &username, &ha1).await?;
    } else if req.tenant_id.is_some() {
        state
            .store
            .update_user_tenant(&username, req.tenant_id.as_deref())
            .await
            .map_err(|e| ApiError {
                error: e.to_string(),
            })?;
    }

    if let Some(ref tid) = effective_tenant_id {
        crate::system::hot_cache::set_extension_tenant(&state, &username, Some(tid)).await?;
    }
    Ok(StatusCode::OK)
}

async fn digest_realm(state: &AppState, tenant_id: Option<&str>) -> Result<String, ApiError> {
    if let Some(tid) = tenant_id {
        if !tid.trim().is_empty() {
            let tenant_domain = sqlx::query_scalar::<_, String>(
                "SELECT domain FROM tenants WHERE id = $1 AND enabled = TRUE",
            )
            .bind(tid)
            .fetch_optional(state.store.pool())
            .await
            .map_err(|error| ApiError::internal(format!("读取租户域名失败: {error}")))?;
            if let Some(domain) = tenant_domain {
                if !domain.trim().is_empty() {
                    return Ok(domain);
                }
            }
        }
    }

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
        // 同步删除 Redis 中的分机-租户映射
        crate::system::hot_cache::set_extension_tenant(&state, &username, None).await?;
        Ok(StatusCode::OK)
    } else {
        Ok(StatusCode::NOT_FOUND)
    }
}
