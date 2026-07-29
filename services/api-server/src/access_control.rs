//! 动态控制台用户、角色权限与菜单管理 API。

use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use axum::{
    extract::{Extension, Path, State},
    http::StatusCode,
    Json,
};
use cdr_core::{
    AccessMenuGroup, AccessPermission, AccessRole, AccessSnapshot, ConsoleCredential, ConsoleUser,
};
use serde::{Deserialize, Serialize};

use crate::{system::auth::Claims, ApiError, AppState};

#[derive(Debug, Serialize)]
pub struct SessionProfile {
    pub username: String,
    pub display_name: String,
    pub role: String,
    pub role_name: String,
    pub permissions: Vec<String>,
    pub menus: Vec<AccessMenuGroup>,
}

#[derive(Debug, Serialize)]
pub struct AccessControlOverview {
    pub users: Vec<ConsoleUser>,
    pub roles: Vec<AccessRole>,
    pub permissions: Vec<AccessPermission>,
    pub menus: Vec<AccessMenuGroup>,
}

#[derive(Debug, Deserialize)]
pub struct CreateUserRequest {
    pub username: String,
    pub password: String,
    pub display_name: String,
    pub role_key: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateUserRequest {
    pub display_name: String,
    pub role_key: String,
    pub enabled: bool,
    pub password: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateRoleRequest {
    pub role_key: String,
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateRoleRequest {
    pub name: String,
    pub description: String,
    pub enabled: bool,
}

#[derive(Debug, Deserialize)]
pub struct ReplacePermissionsRequest {
    pub permission_keys: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct UserRoleAssignment {
    pub username: String,
    pub role_key: String,
}

#[derive(Debug, Deserialize)]
pub struct AssignUserRolesRequest {
    pub assignments: Vec<UserRoleAssignment>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateMenuRequest {
    pub label: String,
    pub sort_order: i32,
    pub enabled: bool,
}

pub async fn initialize_access_control(
    store: &cdr_core::PostgresCdrStore,
    bootstrap_password: Option<&str>,
    production: bool,
) -> anyhow::Result<AccessSnapshot> {
    if store.count_console_users().await? == 0 {
        let password = match bootstrap_password {
            Some(value) if value.len() >= 12 => value,
            Some(_) if production => {
                return Err(anyhow::anyhow!(
                    "VOS_RS_BOOTSTRAP_ADMIN_PASSWORD 在生产环境中至少需要 12 个字符"
                ));
            }
            Some(value) => value,
            None if production => {
                return Err(anyhow::anyhow!(
                    "数据库中没有控制台用户，请设置 VOS_RS_BOOTSTRAP_ADMIN_PASSWORD 完成首次初始化"
                ));
            }
            None => "admin12345",
        };
        let hash = hash_password(password)?;
        store.bootstrap_console_admin(&hash).await?;
        tracing::warn!("已创建首个数据库管理员账号 admin；请登录后立即在权限管理页面修改密码");
    }
    Ok(store.load_access_snapshot().await?)
}

pub async fn refresh_access_snapshot(state: &AppState) -> Result<(), ApiError> {
    let snapshot = state
        .store
        .load_access_snapshot()
        .await
        .map_err(access_error)?;
    *state.access_snapshot.write().await = snapshot;
    Ok(())
}

pub fn verify_password(credential: &ConsoleCredential, password: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(&credential.password_hash) else {
        return false;
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

pub async fn session_profile(
    state: &AppState,
    claims: &Claims,
) -> Result<SessionProfile, ApiError> {
    let credential = state
        .store
        .get_console_credential(&claims.sub)
        .await
        .map_err(access_error)?
        .filter(|item| {
            item.enabled && item.role_key == claims.role && item.auth_version == claims.auth_version
        })
        .ok_or_else(|| ApiError::unauthorized("当前会话权限已变更，请重新登录"))?;
    let snapshot = state.access_snapshot.read().await;
    let permissions = snapshot
        .role_permissions
        .get(&claims.role)
        .cloned()
        .unwrap_or_default();
    let all_menus = state
        .store
        .list_access_menus()
        .await
        .map_err(access_error)?;
    let menus = filter_menus(all_menus, &permissions);
    let mut permissions: Vec<String> = permissions.into_iter().collect();
    permissions.sort();
    Ok(SessionProfile {
        username: credential.username,
        display_name: credential.display_name,
        role: credential.role_key,
        role_name: credential.role_name,
        permissions,
        menus,
    })
}

pub async fn overview(
    State(state): State<AppState>,
) -> Result<Json<AccessControlOverview>, ApiError> {
    let (users, roles, permissions, menus) = tokio::try_join!(
        state.store.list_console_users(),
        state.store.list_access_roles(),
        state.store.list_access_permissions(),
        state.store.list_access_menus(),
    )
    .map_err(access_error)?;
    Ok(Json(AccessControlOverview {
        users,
        roles,
        permissions,
        menus,
    }))
}

pub async fn create_user(
    State(state): State<AppState>,
    Json(request): Json<CreateUserRequest>,
) -> Result<Json<ConsoleUser>, ApiError> {
    validate_identifier("用户名", &request.username)?;
    validate_password(&request.password)?;
    validate_required("显示名称", &request.display_name)?;
    let hash = hash_password(&request.password)
        .map_err(|error| ApiError::internal(format!("密码加密失败: {error}")))?;
    state
        .store
        .create_console_user(
            &request.username,
            &hash,
            request.display_name.trim(),
            &request.role_key,
        )
        .await
        .map_err(access_error)?;
    refresh_access_snapshot(&state).await?;
    find_user(&state, &request.username).await
}

pub async fn update_user(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(username): Path<String>,
    Json(request): Json<UpdateUserRequest>,
) -> Result<Json<ConsoleUser>, ApiError> {
    validate_required("显示名称", &request.display_name)?;
    if username == claims.sub && !request.enabled {
        return Err(ApiError::bad_request("参数无效：不能停用当前登录账号"));
    }
    let password_hash = match request
        .password
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        Some(password) => {
            validate_password(password)?;
            Some(
                hash_password(password)
                    .map_err(|error| ApiError::internal(format!("密码加密失败: {error}")))?,
            )
        }
        None => None,
    };
    let updated = state
        .store
        .update_console_user(
            &username,
            request.display_name.trim(),
            &request.role_key,
            request.enabled,
            password_hash.as_deref(),
        )
        .await
        .map_err(access_error)?;
    if !updated {
        return Err(ApiError::not_found("控制台用户不存在"));
    }
    refresh_access_snapshot(&state).await?;
    find_user(&state, &username).await
}

pub async fn delete_user(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Path(username): Path<String>,
) -> Result<StatusCode, ApiError> {
    if username == claims.sub {
        return Err(ApiError::bad_request("参数无效：不能删除当前登录账户"));
    }
    let deleted = state
        .store
        .delete_console_user(&username)
        .await
        .map_err(access_error)?;
    if !deleted {
        return Err(ApiError::bad_request(
            "参数无效：账户不存在或内置账户不能删除",
        ));
    }
    refresh_access_snapshot(&state).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn create_role(
    State(state): State<AppState>,
    Json(request): Json<CreateRoleRequest>,
) -> Result<Json<AccessRole>, ApiError> {
    validate_identifier("角色标识", &request.role_key)?;
    validate_required("角色名称", &request.name)?;
    state
        .store
        .create_access_role(
            &request.role_key,
            request.name.trim(),
            request.description.as_deref().unwrap_or_default().trim(),
        )
        .await
        .map_err(access_error)?;
    refresh_access_snapshot(&state).await?;
    find_role(&state, &request.role_key).await
}

pub async fn update_role(
    State(state): State<AppState>,
    Path(role_key): Path<String>,
    Json(request): Json<UpdateRoleRequest>,
) -> Result<Json<AccessRole>, ApiError> {
    validate_required("角色名称", &request.name)?;
    if role_key == "admin" && !request.enabled {
        return Err(ApiError::bad_request("参数无效：不能停用系统管理员角色"));
    }
    let updated = state
        .store
        .update_access_role(
            &role_key,
            request.name.trim(),
            request.description.trim(),
            request.enabled,
        )
        .await
        .map_err(access_error)?;
    if !updated {
        return Err(ApiError::not_found("角色不存在"));
    }
    refresh_access_snapshot(&state).await?;
    find_role(&state, &role_key).await
}

pub async fn delete_role(
    State(state): State<AppState>,
    Path(role_key): Path<String>,
) -> Result<StatusCode, ApiError> {
    let deleted = state
        .store
        .delete_access_role(&role_key)
        .await
        .map_err(access_error)?;
    if !deleted {
        return Err(ApiError::bad_request(
            "参数无效：系统管理员或已分配账户的角色不能删除",
        ));
    }
    refresh_access_snapshot(&state).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn assign_user_roles(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
    Json(request): Json<AssignUserRolesRequest>,
) -> Result<StatusCode, ApiError> {
    if request.assignments.len() > 500 {
        return Err(ApiError::bad_request("参数无效：单次最多分配 500 个账户"));
    }
    let (users, roles) = tokio::try_join!(
        state.store.list_console_users(),
        state.store.list_access_roles(),
    )
    .map_err(access_error)?;
    let known_users: std::collections::HashSet<_> =
        users.into_iter().map(|user| user.username).collect();
    let enabled_roles: std::collections::HashSet<_> = roles
        .into_iter()
        .filter(|role| role.enabled)
        .map(|role| role.role_key)
        .collect();
    let mut assignments = Vec::with_capacity(request.assignments.len());
    for assignment in request.assignments {
        validate_identifier("登录账号", &assignment.username)?;
        validate_identifier("角色标识", &assignment.role_key)?;
        if assignment.username == claims.sub {
            return Err(ApiError::bad_request(
                "参数无效：不能在当前会话中修改自己的角色",
            ));
        }
        if !known_users.contains(&assignment.username) {
            return Err(ApiError::bad_request("参数无效：账户不存在"));
        }
        if !enabled_roles.contains(&assignment.role_key) {
            return Err(ApiError::bad_request("参数无效：角色不存在或已停用"));
        }
        assignments.push((assignment.username, assignment.role_key));
    }
    state
        .store
        .assign_console_user_roles(&assignments)
        .await
        .map_err(access_error)?;
    refresh_access_snapshot(&state).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn replace_permissions(
    State(state): State<AppState>,
    Path(role_key): Path<String>,
    Json(mut request): Json<ReplacePermissionsRequest>,
) -> Result<Json<AccessRole>, ApiError> {
    request.permission_keys.sort();
    request.permission_keys.dedup();
    if role_key == "admin" && request.permission_keys != ["*".to_string()] {
        return Err(ApiError::bad_request(
            "参数无效：系统管理员角色必须保留全部权限",
        ));
    }
    state
        .store
        .replace_role_permissions(&role_key, &request.permission_keys)
        .await
        .map_err(access_error)?;
    refresh_access_snapshot(&state).await?;
    find_role(&state, &role_key).await
}

pub async fn update_menu(
    State(state): State<AppState>,
    Path(item_key): Path<String>,
    Json(request): Json<UpdateMenuRequest>,
) -> Result<Json<AccessMenuGroup>, ApiError> {
    validate_required("菜单名称", &request.label)?;
    let updated = state
        .store
        .update_access_menu_item(
            &item_key,
            request.label.trim(),
            request.sort_order,
            request.enabled,
        )
        .await
        .map_err(access_error)?;
    if !updated {
        return Err(ApiError::not_found("菜单项不存在"));
    }
    let menus = state
        .store
        .list_access_menus()
        .await
        .map_err(access_error)?;
    let group = menus
        .into_iter()
        .find(|group| group.items.iter().any(|item| item.item_key == item_key))
        .ok_or_else(|| ApiError::not_found("菜单分组不存在"))?;
    Ok(Json(group))
}

fn filter_menus(
    groups: Vec<AccessMenuGroup>,
    permissions: &std::collections::HashSet<String>,
) -> Vec<AccessMenuGroup> {
    groups
        .into_iter()
        .filter(|group| group.enabled)
        .filter_map(|mut group| {
            group.items.retain(|item| {
                item.enabled && permission_set_allows(permissions, &item.permission_key)
            });
            (!group.items.is_empty()).then_some(group)
        })
        .collect()
}

fn permission_set_allows(
    permissions: &std::collections::HashSet<String>,
    permission: &str,
) -> bool {
    permissions.contains("*")
        || permissions.contains(permission)
        || (permission.starts_with("access.accounts.") && permissions.contains("access.users"))
        || (permission.starts_with("access.roles.") && permissions.contains("access.roles"))
        || (permission.starts_with("llm.") && permissions.contains("llm.manage"))
}

fn hash_password(password: &str) -> anyhow::Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    Ok(Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|error| anyhow::anyhow!(error.to_string()))?
        .to_string())
}

fn validate_identifier(label: &str, value: &str) -> Result<(), ApiError> {
    let valid = !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'));
    if valid {
        Ok(())
    } else {
        Err(ApiError::bad_request(format!(
            "参数无效：{label}只能包含字母、数字、点、短横线和下划线"
        )))
    }
}

fn validate_required(label: &str, value: &str) -> Result<(), ApiError> {
    if value.trim().is_empty() || value.chars().count() > 64 {
        Err(ApiError::bad_request(format!(
            "参数无效：{label}不能为空且不能超过 64 个字符"
        )))
    } else {
        Ok(())
    }
}

fn validate_password(password: &str) -> Result<(), ApiError> {
    if password.len() < 10 || password.len() > 128 {
        Err(ApiError::bad_request(
            "参数无效：密码长度必须在 10 到 128 个字符之间",
        ))
    } else {
        Ok(())
    }
}

async fn find_user(state: &AppState, username: &str) -> Result<Json<ConsoleUser>, ApiError> {
    state
        .store
        .list_console_users()
        .await
        .map_err(access_error)?
        .into_iter()
        .find(|item| item.username == username)
        .map(Json)
        .ok_or_else(|| ApiError::not_found("控制台用户不存在"))
}

async fn find_role(state: &AppState, role_key: &str) -> Result<Json<AccessRole>, ApiError> {
    state
        .store
        .list_access_roles()
        .await
        .map_err(access_error)?
        .into_iter()
        .find(|item| item.role_key == role_key)
        .map(Json)
        .ok_or_else(|| ApiError::not_found("角色不存在"))
}

fn access_error(error: sqlx::Error) -> ApiError {
    match &error {
        sqlx::Error::Database(database) if database.is_unique_violation() => {
            ApiError::bad_request("参数无效：标识已存在")
        }
        sqlx::Error::Database(database) if database.is_foreign_key_violation() => {
            ApiError::bad_request("参数无效：关联的角色或权限不存在")
        }
        _ => ApiError::internal(format!("权限数据操作失败: {error}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cdr_core::AccessMenuItem;
    use std::collections::HashSet;

    #[test]
    fn menu_filter_respects_permissions_and_enabled_state() {
        let groups = vec![AccessMenuGroup {
            group_key: "g".to_string(),
            label: "测试分组".to_string(),
            icon_key: "grid".to_string(),
            sort_order: 1,
            enabled: true,
            items: vec![AccessMenuItem {
                item_key: "one".to_string(),
                label: "测试菜单".to_string(),
                path: "/one".to_string(),
                icon_key: "grid".to_string(),
                permission_key: "one.view".to_string(),
                sort_order: 1,
                enabled: true,
            }],
        }];
        assert!(filter_menus(groups.clone(), &HashSet::new()).is_empty());
        assert_eq!(
            filter_menus(groups, &HashSet::from(["one.view".to_string()])).len(),
            1
        );
    }

    #[test]
    fn legacy_access_permissions_keep_new_menus_visible() {
        assert!(permission_set_allows(
            &HashSet::from(["access.users".to_string()]),
            "access.accounts.view"
        ));
        assert!(permission_set_allows(
            &HashSet::from(["access.roles".to_string()]),
            "access.roles.view"
        ));
        assert!(permission_set_allows(
            &HashSet::from(["llm.manage".to_string()]),
            "llm.view"
        ));
    }

    #[test]
    fn identifiers_reject_spaces_and_passwords_require_length() {
        assert!(validate_identifier("角色", "ops-team").is_ok());
        assert!(validate_identifier("角色", "ops team").is_err());
        assert!(validate_password("short").is_err());
        assert!(validate_password("long-enough-password").is_ok());
    }
}
