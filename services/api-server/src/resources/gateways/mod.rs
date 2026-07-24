use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;

use crate::{normalize_page, ApiError, AppState, PageQuery, PaginatedResponse};

mod manage;
pub(crate) use manage::{create_gateway, update_gateway};

#[derive(Debug, Deserialize)]
pub struct CreateGatewayRequest {
    pub id: String,
    #[serde(default)]
    pub host: String,
    pub port: Option<u16>,
    pub transport: String,
    pub max_capacity: Option<u32>,
    pub gateway_type: Option<String>,
    pub role: Option<String>,
    pub access_auth_mode: Option<String>,
    pub access_username: Option<String>,
    pub access_realm: Option<String>,
    pub access_password: Option<String>,
    pub prefix_rules: Option<String>,
    pub supports_registration: Option<bool>,
    pub reg_auth_type: Option<String>,
    pub reg_username: Option<String>,
    pub reg_password: Option<String>,
    pub caller_id_mode: Option<String>,
    pub virtual_caller: Option<String>,
    pub max_concurrent: Option<i32>,
    pub account_id: Option<i64>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateGatewayRequest {
    #[serde(default)]
    pub host: String,
    pub port: Option<u16>,
    pub transport: String,
    pub max_capacity: Option<u32>,
    pub gateway_type: Option<String>,
    pub role: Option<String>,
    pub access_auth_mode: Option<String>,
    pub access_username: Option<String>,
    pub access_realm: Option<String>,
    pub access_password: Option<String>,
    pub prefix_rules: Option<String>,
    pub supports_registration: Option<bool>,
    pub reg_auth_type: Option<String>,
    pub reg_username: Option<String>,
    pub reg_password: Option<String>,
    pub caller_id_mode: Option<String>,
    pub virtual_caller: Option<String>,
    pub max_concurrent: Option<i32>,
    pub account_id: Option<i64>,
    pub enabled: Option<bool>,
}

#[allow(clippy::too_many_arguments)]
fn validate_gateway(
    id: &str,
    host: &str,
    port: Option<u16>,
    transport: &str,
    caller_id_mode: Option<&str>,
    virtual_caller: Option<&str>,
    role: Option<&str>,
    access_auth_mode: Option<&str>,
    access_username: Option<&str>,
    access_realm: Option<&str>,
    has_access_password: bool,
) -> Result<(), ApiError> {
    if id.trim().is_empty() {
        return Err(ApiError::internal("参数无效: 中继 ID 不能为空"));
    }
    let role = role.unwrap_or("egress");
    if !matches!(role, "access" | "egress") {
        return Err(ApiError::internal(
            "参数无效: 中继类型只能是 access 或 egress",
        ));
    }
    if (role == "egress" && host.trim().is_empty())
        || (!host.trim().is_empty() && host.chars().any(char::is_whitespace))
    {
        return Err(ApiError::internal(
            "参数无效: 中继主机不能为空或包含空白字符",
        ));
    }
    if port == Some(0) {
        return Err(ApiError::internal(
            "参数无效: SIP 端口必须在 1 到 65535 之间",
        ));
    }
    if transport != "udp" {
        return Err(ApiError::internal(
            "参数无效: 当前中继出站链路仅支持 udp，tcp/tls 尚未接通",
        ));
    }
    if let Some(mode) = caller_id_mode {
        if !matches!(mode, "passthrough" | "virtual" | "random") {
            return Err(ApiError::internal(
                "参数无效: 主叫号码策略只能是 passthrough、virtual 或 random",
            ));
        }
        if mode == "virtual" && virtual_caller.is_none_or(|caller| caller.trim().is_empty()) {
            return Err(ApiError::internal(
                "参数无效: virtual 主叫号码策略必须配置虚拟主叫号码",
            ));
        }
    }
    let auth_mode = match access_auth_mode.unwrap_or("none") {
        "ip_whitelist" => "ip_allowlist",
        mode => mode,
    };
    if !matches!(
        auth_mode,
        "none" | "ip_allowlist" | "digest_register" | "ip_and_digest"
    ) {
        return Err(ApiError::internal("参数无效: 注册认证模式不受支持"));
    }
    if role == "access" && auth_mode == "none" {
        return Err(ApiError::internal(
            "参数无效: 接入中继必须选择 IP 白名单、注册认证或组合认证",
        ));
    }
    if matches!(auth_mode, "digest_register" | "ip_and_digest")
        && (access_username.is_none_or(|value| value.trim().is_empty())
            || access_realm.is_none_or(|value| value.trim().is_empty())
            || !has_access_password)
    {
        return Err(ApiError::internal(
            "参数无效: 注册认证必须配置用户名、Realm 和密码",
        ));
    }
    Ok(())
}

fn access_ha1(username: &str, realm: &str, password: &str) -> String {
    format!(
        "{:x}",
        md5::compute(format!("{username}:{realm}:{password}"))
    )
}

fn reject_unsupported_egress_secret(_role: &str, _password: Option<&str>) -> Result<(), ApiError> {
    Ok(())
}

pub async fn list_gateways(
    State(state): State<AppState>,
    Query(query): Query<PageQuery>,
) -> Result<axum::response::Response, ApiError> {
    let (page, page_size, offset) = normalize_page(&query);
    let (items, total) = tokio::try_join!(
        state.store.list_gateways_page(
            page_size,
            offset,
            query.gateway_type.as_deref(),
            query.role.as_deref()
        ),
        state
            .store
            .count_gateways(query.gateway_type.as_deref(), query.role.as_deref()),
    )
    .map_err(|e| ApiError {
        error: e.to_string(),
    })?;

    if query.export.unwrap_or(false) {
        let headers = vec![
            "中继标识",
            "中继类型",
            "认证方式/对端地址",
            "注册用户",
            "最大并发",
            "启用状态",
            "传输协议",
            "内部端口",
        ];
        let mut rows = Vec::new();
        for item in items {
            let role = item.role.as_deref().unwrap_or("egress");
            let role_name = if role == "access" {
                "接入中继"
            } else {
                "落地中继"
            };
            let auth_or_host = if role == "access" {
                match item.access_auth_mode.as_deref().unwrap_or("ip_allowlist") {
                    "digest_register" => "注册认证",
                    "ip_and_digest" => "IP加认证",
                    _ => "IP白名单",
                }
            } else {
                &item.host
            };
            let enabled_str = if item.enabled.unwrap_or(true) {
                "启用"
            } else {
                "停用"
            };
            rows.push(vec![
                item.id.clone(),
                role_name.to_string(),
                auth_or_host.to_string(),
                item.access_username.clone().unwrap_or_default(),
                item.max_capacity.map(|c| c.to_string()).unwrap_or_default(),
                enabled_str.to_string(),
                item.transport.clone(),
                item.port.map(|p| p.to_string()).unwrap_or_default(),
            ]);
        }
        return Ok(crate::system::utils::to_csv_response(
            "gateways.csv",
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

pub async fn delete_gateway(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let deleted = state
        .store
        .delete_gateway(&id)
        .await
        .map_err(|e| ApiError {
            error: e.to_string(),
        })?;
    if deleted {
        crate::resources::routes::publish_route_reload(&state.nats_client).await;
        Ok(StatusCode::OK)
    } else {
        Ok(StatusCode::NOT_FOUND)
    }
}

#[cfg(test)]
mod tests {
    use super::{reject_unsupported_egress_secret, validate_gateway};

    #[test]
    fn validates_transport_port_and_virtual_caller() {
        let validate = |host: &str, port, transport, role, auth, user, realm, password| {
            validate_gateway(
                "gw", host, port, transport, None, None, role, auth, user, realm, password,
            )
        };
        assert!(validate(
            "127.0.0.1",
            Some(5060),
            "udp",
            Some("egress"),
            None,
            None,
            None,
            false
        )
        .is_ok());
        assert!(validate("127.0.0.1", Some(0), "udp", None, None, None, None, false).is_err());
        assert!(validate(
            "127.0.0.1",
            Some(5060),
            "tcp",
            None,
            None,
            None,
            None,
            false
        )
        .is_err());
        assert!(validate(
            "127.0.0.1",
            Some(5060),
            "udp",
            Some("access"),
            Some("none"),
            None,
            None,
            false
        )
        .is_err());
        assert!(validate(
            "",
            Some(5060),
            "udp",
            Some("access"),
            Some("ip_allowlist"),
            None,
            None,
            false
        )
        .is_ok());
        assert!(validate(
            "",
            Some(5060),
            "udp",
            Some("access"),
            Some("digest_register"),
            Some("carrier"),
            Some("vos-rs"),
            true
        )
        .is_ok());
    }

    #[test]
    fn rejects_plaintext_upstream_registration_password() {
        assert!(reject_unsupported_egress_secret("egress", Some("secret")).is_ok());
        assert!(reject_unsupported_egress_secret("egress", None).is_ok());
        assert!(reject_unsupported_egress_secret("access", Some("secret")).is_ok());
    }
}
