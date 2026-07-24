use crate::error::ApiError;
use crate::system::auth::{role_allows, Claims};
use crate::AppState;
use axum::{extract::State, http::HeaderValue};
use jsonwebtoken::{decode, DecodingKey, Validation};
use uuid::Uuid;

/// JWT 认证中间件：验证请求中的 Bearer Token 并检查 RBAC 权限。
pub(crate) async fn jwt_auth(
    State(state): State<AppState>,
    mut req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Result<axum::response::Response, ApiError> {
    let auth_header = req
        .headers()
        .get("authorization")
        .and_then(|h| h.to_str().ok());

    let Some(auth_str) = auth_header else {
        return Err(ApiError::unauthorized("缺少凭证".to_string()));
    };

    if !auth_str.starts_with("Bearer ") {
        return Err(ApiError::unauthorized("凭证格式不正确".to_string()));
    }

    let token = &auth_str[7..];
    let validation = Validation::default();

    match decode::<Claims>(
        token,
        &DecodingKey::from_secret(&state.jwt_secret),
        &validation,
    ) {
        Ok(token_data) => {
            let path = req.uri().path();
            let role = &token_data.claims.role;
            if !role_allows(role, req.method().as_str(), path) {
                return Err(ApiError::forbidden(format!(
                    "越权访问：角色 {role} 无权访问 {path}"
                )));
            }

            req.extensions_mut().insert(token_data.claims);
            Ok(next.run(req).await)
        }
        Err(e) => Err(ApiError::unauthorized(format!("无效 Token: {}", e))),
    }
}

/// 对审计请求体中的敏感字段做递归脱敏。
pub(crate) fn sanitize_audit_json(body: &[u8]) -> String {
    let Ok(mut value) = serde_json::from_slice::<serde_json::Value>(body) else {
        return "[已省略无法解析的 JSON 请求体]".to_string();
    };
    redact_sensitive_json_value(&mut value);
    serde_json::to_string(&value).unwrap_or_else(|_| "[已省略审计请求体]".to_string())
}

/// 递归遍历 JSON 对象和数组，将常见凭据字段替换成固定占位符。
pub(crate) fn redact_sensitive_json_value(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(object) => {
            for (key, child) in object.iter_mut() {
                let normalized_key = key.to_ascii_lowercase();
                if normalized_key.ends_with("password")
                    || matches!(
                        normalized_key.as_str(),
                        "password"
                            | "passwd"
                            | "secret"
                            | "token"
                            | "access_token"
                            | "refresh_token"
                            | "api_key"
                            | "authorization"
                            | "ha1"
                    )
                {
                    *child = serde_json::Value::String("[已脱敏]".to_string());
                } else {
                    redact_sensitive_json_value(child);
                }
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                redact_sensitive_json_value(item);
            }
        }
        _ => {}
    }
}

pub(crate) async fn audit_log(
    State(state): State<AppState>,
    mut req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let method = req.method().clone();
    let uri = req.uri().clone();
    let query_params = uri.query().map(|q| q.to_string());
    let request_id = req
        .headers()
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let source_ip = req
        .headers()
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);

    let username = req
        .extensions()
        .get::<Claims>()
        .map(|c| c.sub.clone())
        .unwrap_or_else(|| "anonymous".to_string());
    let role = req
        .extensions()
        .get::<Claims>()
        .map(|c| c.role.clone())
        .unwrap_or_else(|| "unknown".to_string());

    // 仅记录经过脱敏的 JSON 请求体，避免把密码、Token 等凭据写入审计库。
    let request_body = if matches!(
        method,
        axum::http::Method::POST | axum::http::Method::PUT | axum::http::Method::PATCH
    ) {
        let (parts, body) = req.into_parts();
        const MAX_BODY_BUFFER_BYTES: usize = 15 * 1024 * 1024; // 允许最大 15MB 图像/附件数据流
        const MAX_AUDIT_LOG_TEXT_BYTES: usize = 256 * 1024; // 数据库审计记录上限 256KB
        let content_type = parts
            .headers
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let body_result = axum::body::to_bytes(body, MAX_BODY_BUFFER_BYTES).await;
        let body_bytes = match body_result {
            Ok(bytes) => bytes,
            Err(error) => {
                tracing::warn!(%error, request_id = %request_id, "读取请求体失败");
                axum::body::Bytes::new()
            }
        };
        let body_str = if body_bytes.len() > MAX_AUDIT_LOG_TEXT_BYTES {
            format!(
                "[请求体包含附件/多模态图片，完整大小 {} 字节，审计日志自动省简]",
                body_bytes.len()
            )
        } else if content_type.starts_with("application/json") {
            sanitize_audit_json(&body_bytes)
        } else if body_bytes.is_empty() {
            String::new()
        } else {
            format!("[已省略非 JSON 请求体，大小 {} 字节]", body_bytes.len())
        };
        req = axum::extract::Request::from_parts(parts, axum::body::Body::from(body_bytes));
        (!body_str.is_empty()).then_some(body_str)
    } else {
        None
    };

    let response = next.run(req).await;
    let status = response.status();
    let store = state.store.clone();
    let audit_request_id = request_id.clone();
    let audit_username = username.clone();
    let audit_role = role.clone();
    let audit_method = method.to_string();
    let audit_path = uri.path().to_string();
    let audit_query_params = query_params.clone();
    let audit_request_body = request_body.clone();
    let audit_source_ip = source_ip.clone();
    tokio::spawn(async move {
        let input = cdr_core::AuditLogInput {
            request_id: &audit_request_id,
            username: &audit_username,
            role: &audit_role,
            method: &audit_method,
            path: &audit_path,
            query_params: audit_query_params.as_deref(),
            request_body: audit_request_body.as_deref(),
            status_code: status.as_u16(),
            source_ip: audit_source_ip.as_deref(),
        };
        if let Err(error) = store.insert_audit_log(&input).await {
            tracing::warn!(%error, request_id = %audit_request_id, "写入 API 审计日志失败");
        }
    });

    tracing::info!(
        request_id = %request_id,
        action = %method,
        path = %uri.path(),
        operator = %username,
        role = %role,
        status = status.as_u16(),
        "API audit log"
    );
    let mut response = response;
    if let Ok(value) = HeaderValue::from_str(&request_id) {
        response.headers_mut().insert("X-Request-ID", value);
    }
    response
}

#[cfg(test)]
mod tests {
    use super::{redact_sensitive_json_value, sanitize_audit_json};
    use serde_json::json;

    #[test]
    fn audit_json_redacts_nested_credentials() {
        let mut value = json!({
            "username": "alice",
            "password": "secret",
            "nested": [{"access_token": "token-value"}],
            "profile": {"api_key": "key-value"}
        });

        redact_sensitive_json_value(&mut value);

        assert_eq!(value["password"], "[已脱敏]");
        assert_eq!(value["nested"][0]["access_token"], "[已脱敏]");
        assert_eq!(value["profile"]["api_key"], "[已脱敏]");
        assert_eq!(value["username"], "alice");
    }

    #[test]
    fn invalid_audit_json_is_not_recorded_as_plaintext() {
        let result = sanitize_audit_json(br#"password=secret&token=value"#);

        assert_eq!(result, "[已省略无法解析的 JSON 请求体]");
        assert!(!result.contains("secret"));
    }
}
