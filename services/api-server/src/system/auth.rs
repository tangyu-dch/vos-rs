use axum::{extract::State, Extension, Json};
use jsonwebtoken::{encode, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use std::time::SystemTime;

use crate::{access_control, ApiError, AppState};

/// JWT 声明，仅保存身份和权限版本；权限本身始终从数据库快照读取。
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Claims {
    pub sub: String,
    pub role: String,
    #[serde(default)]
    pub auth_version: i64,
    pub exp: usize,
}

/// 登录请求。
#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

/// 登录响应，包含动态权限和菜单。
#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub token: String,
    #[serde(flatten)]
    pub profile: access_control::SessionProfile,
}

/// 从数据库验证控制台用户并签发 JWT。
pub async fn login(
    State(state): State<AppState>,
    Json(request): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, ApiError> {
    let credential = state
        .store
        .get_console_credential(request.username.trim())
        .await
        .map_err(|error| ApiError::internal(format!("读取登录用户失败: {error}")))?
        .filter(|item| item.enabled)
        .ok_or_else(|| invalid_credentials(&request.username))?;
    if !access_control::verify_password(&credential, &request.password) {
        return Err(invalid_credentials(&request.username));
    }

    let exp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as usize
        + 24 * 3600;
    let claims = Claims {
        sub: credential.username.clone(),
        role: credential.role_key.clone(),
        auth_version: credential.auth_version,
        exp,
    };
    let token = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(&state.jwt_secret),
    )
    .map_err(|error| ApiError::internal(format!("JWT 签名失败: {error}")))?;
    let profile = access_control::session_profile(&state, &claims).await?;
    tracing::info!(username = %claims.sub, role = %claims.role, "登录成功");
    Ok(Json(LoginResponse { token, profile }))
}

/// 返回当前数据库会话对应的动态权限和菜单。
pub async fn current_session(
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> Result<Json<access_control::SessionProfile>, ApiError> {
    Ok(Json(
        access_control::session_profile(&state, &claims).await?,
    ))
}

fn invalid_credentials(username: &str) -> ApiError {
    tracing::warn!(username, "登录失败：用户名或密码错误");
    ApiError::unauthorized("用户名或密码错误")
}
