use axum::{extract::State, Extension, Json};
use jsonwebtoken::{encode, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use std::time::SystemTime;

use crate::{ApiError, AppState};

/// JWT 声明：包含用户身份和权限信息。
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Claims {
    pub sub: String,
    pub role: String,
    pub exp: usize,
}

/// 登录请求
#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

/// 登录响应
#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub token: String,
    pub username: String,
    pub role: String,
}

/// 用户登录：验证凭据并返回 JWT Token。
pub async fn login(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, ApiError> {
    let admin_password = &state.admin_password;
    let operator_password = &state.operator_password;
    let financier_password = &state.financier_password;

    let role = if req.username == "admin" && req.password == *admin_password {
        "admin".to_string()
    } else if req.username == "operator" && req.password == *operator_password {
        "operator".to_string()
    } else if req.username == "financier" && req.password == *financier_password {
        "financier".to_string()
    } else {
        tracing::warn!(username = %req.username, "登录失败：用户名或密码错误");
        return Err(ApiError::unauthorized("用户名或密码错误".to_string()));
    };

    tracing::info!(username = %req.username, role = %role, "登录成功");

    let exp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as usize
        + 24 * 3600;

    let claims = Claims {
        sub: req.username,
        role,
        exp,
    };

    let token = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(&state.jwt_secret),
    )
    .map_err(|e| ApiError::internal(format!("JWT 签名失败: {}", e)))?;

    Ok(Json(LoginResponse {
        token,
        username: claims.sub,
        role: claims.role,
    }))
}

/// Returns the identity and role carried by the current verified session.
pub async fn current_session(Extension(claims): Extension<Claims>) -> Json<Claims> {
    Json(claims)
}

/// RBAC 权限检查：根据角色、HTTP 方法 and 路径判断是否允许访问。
pub fn role_allows(role: &str, method: &str, path: &str) -> bool {
    if role == "admin" {
        return true;
    }

    if path == "/api/v1/auth/me" {
        return matches!(role, "operator" | "financier");
    }

    // SIP user credentials are security-sensitive and remain administrator-only.
    if path.starts_with("/api/users") || path.starts_with("/api/v1/extensions") {
        return false;
    }

    let finance_path = path.starts_with("/api/v1/billing")
        || path.starts_with("/api/rates")
        || path.starts_with("/api/accounts")
        || path.starts_with("/api/ledger")
        || path.starts_with("/api/billing");
    if finance_path {
        return role == "financier";
    }

    let operations_path = path.starts_with("/api/v1/trunks")
        || path.starts_with("/api/v1/routing")
        || path.starts_with("/api/v1/numbers")
        || path.starts_with("/api/v1/security/anti-fraud")
        || (path.starts_with("/api/v1/calls/") && method == "POST")
        || path.starts_with("/api/gateways")
        || path.starts_with("/api/routes")
        || path.starts_with("/api/numbers")
        || path.starts_with("/api/anti-fraud")
        || (path.starts_with("/api/calls/") && method == "POST");
    if operations_path {
        return role == "operator";
    }

    let read_only_path = path.starts_with("/api/v1/overview")
        || path.starts_with("/api/v1/calls")
        || path.starts_with("/api/v1/registrations")
        || path.starts_with("/api/v1/reports")
        || path.starts_with("/api/v1/infrastructure/media/metrics")
        || path.starts_with("/api/dashboard")
        || path.starts_with("/api/cdrs")
        || path.starts_with("/api/registrations")
        || path.starts_with("/api/recordings")
        || path.starts_with("/api/reports")
        || path == "/api/calls/active"
        || path == "/api/route-preview"
        || path == "/api/media/metrics";
    if read_only_path {
        return role == "operator" || role == "financier";
    }

    false
}

#[cfg(test)]
mod tests {
    use super::role_allows;

    #[test]
    fn admin_can_access_every_protected_route() {
        assert!(role_allows("admin", "POST", "/api/accounts/alice/credit"));
        assert!(role_allows("admin", "DELETE", "/api/users/alice"));
    }

    #[test]
    fn operator_cannot_access_finance_or_user_credentials() {
        assert!(!role_allows(
            "operator",
            "POST",
            "/api/accounts/alice/credit"
        ));
        assert!(!role_allows("operator", "PUT", "/api/users/alice"));
        assert!(role_allows("operator", "POST", "/api/routes"));
        assert!(role_allows("operator", "POST", "/api/calls/id/terminate"));
    }

    #[test]
    fn financier_cannot_change_operations_configuration() {
        assert!(!role_allows("financier", "POST", "/api/routes"));
        assert!(!role_allows("financier", "POST", "/api/gateways"));
        assert!(role_allows("financier", "POST", "/api/billing/reconcile"));
        assert!(role_allows("financier", "GET", "/api/cdrs"));
    }

    #[test]
    fn operator_can_read_cdrs_and_recordings() {
        assert!(role_allows("operator", "GET", "/api/cdrs"));
        assert!(role_allows(
            "operator",
            "GET",
            "/api/recordings/call-1/audio"
        ));
        assert!(role_allows("operator", "GET", "/api/dashboard/stats"));
        assert!(role_allows("operator", "GET", "/api/registrations"));
    }

    #[test]
    fn operator_can_access_operations_endpoints() {
        assert!(role_allows("operator", "POST", "/api/routes"));
        assert!(role_allows("operator", "POST", "/api/gateways"));
        assert!(role_allows("operator", "POST", "/api/numbers"));
        assert!(role_allows("operator", "POST", "/api/anti-fraud/rules"));
        assert!(role_allows("operator", "POST", "/api/calls/id/terminate"));
    }

    #[test]
    fn operator_cannot_access_finance_endpoints() {
        assert!(!role_allows("operator", "GET", "/api/accounts"));
        assert!(!role_allows("operator", "POST", "/api/rates"));
        assert!(!role_allows("operator", "POST", "/api/billing/reconcile"));
        assert!(!role_allows("operator", "GET", "/api/ledger"));
    }

    #[test]
    fn operator_cannot_access_user_credentials() {
        assert!(!role_allows("operator", "GET", "/api/users"));
        assert!(!role_allows("operator", "POST", "/api/users"));
        assert!(!role_allows("operator", "PUT", "/api/users/alice"));
        assert!(!role_allows("operator", "DELETE", "/api/users/alice"));
    }

    #[test]
    fn financier_can_access_billing_endpoints() {
        assert!(role_allows("financier", "GET", "/api/cdrs"));
        assert!(role_allows("financier", "GET", "/api/rates"));
        assert!(role_allows("financier", "POST", "/api/rates"));
        assert!(role_allows("financier", "GET", "/api/accounts"));
        assert!(role_allows("financier", "POST", "/api/billing/reconcile"));
        assert!(role_allows("financier", "GET", "/api/ledger"));
    }

    #[test]
    fn financier_cannot_access_operations_endpoints() {
        assert!(!role_allows("financier", "POST", "/api/routes"));
        assert!(!role_allows("financier", "POST", "/api/gateways"));
        assert!(!role_allows("financier", "POST", "/api/numbers"));
        assert!(!role_allows("financier", "POST", "/api/anti-fraud/rules"));
    }

    #[test]
    fn financier_cannot_access_user_credentials() {
        assert!(!role_allows("financier", "GET", "/api/users"));
        assert!(!role_allows("financier", "POST", "/api/users"));
    }

    #[test]
    fn no_role_cannot_access_anything() {
        assert!(!role_allows("unknown", "GET", "/api/cdrs"));
        assert!(!role_allows("unknown", "POST", "/api/routes"));
        assert!(!role_allows("unknown", "GET", "/api/accounts"));
        assert!(!role_allows("", "GET", "/api/cdrs"));
    }

    #[test]
    fn admin_can_access_all_read_only_endpoints() {
        assert!(role_allows("admin", "GET", "/api/cdrs"));
        assert!(role_allows("admin", "GET", "/api/recordings/call-1/audio"));
        assert!(role_allows("admin", "GET", "/api/dashboard/stats"));
        assert!(role_allows("admin", "GET", "/api/registrations"));
        assert!(role_allows("admin", "GET", "/api/media/metrics"));
        assert!(role_allows("admin", "GET", "/api/route-preview"));
    }

    #[test]
    fn admin_can_access_all_write_endpoints() {
        assert!(role_allows("admin", "POST", "/api/routes"));
        assert!(role_allows("admin", "POST", "/api/gateways"));
        assert!(role_allows("admin", "POST", "/api/users"));
        assert!(role_allows("admin", "POST", "/api/rates"));
        assert!(role_allows("admin", "POST", "/api/billing/reconcile"));
        assert!(role_allows("admin", "POST", "/api/anti-fraud/rules"));
        assert!(role_allows("admin", "DELETE", "/api/routes/r1"));
        assert!(role_allows("admin", "DELETE", "/api/gateways/gw1"));
        assert!(role_allows("admin", "DELETE", "/api/users/alice"));
    }

    #[test]
    fn operator_can_access_route_preview_and_media_metrics() {
        assert!(role_allows("operator", "GET", "/api/route-preview"));
        assert!(role_allows("operator", "GET", "/api/media/metrics"));
    }

    #[test]
    fn authenticated_roles_can_read_their_v1_session() {
        assert!(role_allows("admin", "GET", "/api/v1/auth/me"));
        assert!(role_allows("operator", "GET", "/api/v1/auth/me"));
        assert!(role_allows("financier", "GET", "/api/v1/auth/me"));
        assert!(!role_allows("unknown", "GET", "/api/v1/auth/me"));
    }

    #[test]
    fn v1_call_media_recording_and_controls_follow_rbac() {
        for path in [
            "/api/v1/calls/call-1/media",
            "/api/v1/calls/call-1/dtmf",
            "/api/v1/calls/call-1/recording",
        ] {
            assert!(role_allows("operator", "GET", path));
            assert!(role_allows("financier", "GET", path));
        }
        for action in [
            "terminate",
            "play",
            "stop-play",
            "mute",
            "unmute",
            "monitor",
        ] {
            let path = format!("/api/v1/calls/call-1/actions/{action}");
            assert!(role_allows("operator", "POST", &path));
            assert!(!role_allows("financier", "POST", &path));
        }
    }

    #[test]
    fn financier_can_read_cdrs_and_registrations() {
        assert!(role_allows("financier", "GET", "/api/cdrs"));
        assert!(role_allows("financier", "GET", "/api/registrations"));
        assert!(role_allows(
            "financier",
            "GET",
            "/api/recordings/call-1/audio"
        ));
    }
}
