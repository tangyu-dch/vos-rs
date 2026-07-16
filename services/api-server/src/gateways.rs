use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use cdr_core::SipGateway;
use serde::Deserialize;

use crate::{normalize_page, ApiError, AppState, PageQuery, PaginatedResponse};

#[derive(Debug, Deserialize)]
pub struct CreateGatewayRequest {
    pub id: String,
    pub host: String,
    pub port: Option<u16>,
    pub transport: String,
    pub max_capacity: Option<u32>,
    pub gateway_type: Option<String>,
    pub role: Option<String>,
    pub access_auth_mode: Option<String>,
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
    pub host: String,
    pub port: Option<u16>,
    pub transport: String,
    pub max_capacity: Option<u32>,
    pub gateway_type: Option<String>,
    pub role: Option<String>,
    pub access_auth_mode: Option<String>,
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

fn validate_gateway(
    id: &str,
    host: &str,
    port: Option<u16>,
    transport: &str,
    caller_id_mode: Option<&str>,
    virtual_caller: Option<&str>,
    role: Option<&str>,
    access_auth_mode: Option<&str>,
) -> Result<(), ApiError> {
    if id.trim().is_empty() {
        return Err(ApiError::internal("参数无效: 中继 ID 不能为空"));
    }
    if host.trim().is_empty() || host.chars().any(char::is_whitespace) {
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
    let role = role.unwrap_or("egress");
    if !matches!(role, "access" | "egress") {
        return Err(ApiError::internal(
            "参数无效: 中继类型只能是 access 或 egress",
        ));
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
    Ok(())
}

pub async fn list_gateways(
    State(state): State<AppState>,
    Query(query): Query<PageQuery>,
) -> Result<Json<PaginatedResponse<SipGateway>>, ApiError> {
    let (page, page_size, offset) = normalize_page(&query);
    let (items, total) = tokio::try_join!(
        state
            .store
            .list_gateways_page(page_size, offset, query.gateway_type.as_deref()),
        state.store.count_gateways(query.gateway_type.as_deref()),
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

pub async fn create_gateway(
    State(state): State<AppState>,
    Json(req): Json<CreateGatewayRequest>,
) -> Result<StatusCode, ApiError> {
    validate_gateway(
        &req.id,
        &req.host,
        req.port,
        &req.transport,
        req.caller_id_mode.as_deref(),
        req.virtual_caller.as_deref(),
        req.role.as_deref(),
        req.access_auth_mode.as_deref(),
    )?;
    let gw = SipGateway {
        id: req.id,
        host: req.host,
        port: req.port,
        transport: req.transport,
        max_capacity: req.max_capacity.filter(|capacity| *capacity > 0),
        gateway_type: req.gateway_type,
        role: req.role,
        access_auth_mode: req.access_auth_mode.map(|mode| {
            if mode == "ip_whitelist" {
                "ip_allowlist".to_string()
            } else {
                mode
            }
        }),
        prefix_rules: req.prefix_rules,
        supports_registration: req.supports_registration,
        reg_auth_type: req.reg_auth_type,
        reg_username: req.reg_username,
        reg_password: req.reg_password,
        parent_gateway_id: None,
        caller_id_mode: req.caller_id_mode,
        virtual_caller: req.virtual_caller,
        current_concurrent: Some(0),
        circuit_state: Some("closed".to_string()),
        account_id: req.account_id,
        max_concurrent: req.max_concurrent,
        enabled: req.enabled,
        created_at: None,
    };
    state
        .store
        .upsert_gateway_full(&gw)
        .await
        .map_err(|e| ApiError {
            error: e.to_string(),
        })?;
    crate::routes::publish_route_reload(&state.nats_client).await;
    Ok(StatusCode::CREATED)
}

pub async fn update_gateway(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateGatewayRequest>,
) -> Result<StatusCode, ApiError> {
    let existing = state
        .store
        .list_gateways_full()
        .await
        .map_err(|e| ApiError {
            error: e.to_string(),
        })?;
    let old = existing
        .iter()
        .find(|g| g.id == id)
        .ok_or_else(|| ApiError {
            error: "网关不存在".into(),
        })?;
    let caller_id_mode = req
        .caller_id_mode
        .clone()
        .or_else(|| old.caller_id_mode.clone());
    let virtual_caller = req
        .virtual_caller
        .clone()
        .or_else(|| old.virtual_caller.clone());
    validate_gateway(
        &id,
        &req.host,
        req.port,
        &req.transport,
        caller_id_mode.as_deref(),
        virtual_caller.as_deref(),
        req.role.as_deref().or(old.role.as_deref()),
        req.access_auth_mode
            .as_deref()
            .or(old.access_auth_mode.as_deref()),
    )?;
    let gw = SipGateway {
        id: id.clone(),
        host: req.host,
        port: req.port,
        transport: req.transport,
        max_capacity: req.max_capacity.filter(|capacity| *capacity > 0),
        gateway_type: req.gateway_type.or_else(|| old.gateway_type.clone()),
        role: req.role.or_else(|| old.role.clone()),
        access_auth_mode: req
            .access_auth_mode
            .or_else(|| old.access_auth_mode.clone())
            .map(|mode| {
                if mode == "ip_whitelist" {
                    "ip_allowlist".to_string()
                } else {
                    mode
                }
            }),
        prefix_rules: req.prefix_rules.or_else(|| old.prefix_rules.clone()),
        supports_registration: req.supports_registration.or(old.supports_registration),
        reg_auth_type: req.reg_auth_type.or_else(|| old.reg_auth_type.clone()),
        reg_username: req.reg_username.or_else(|| old.reg_username.clone()),
        // The store uses COALESCE, so omission preserves the current secret.
        reg_password: req.reg_password,
        parent_gateway_id: old.parent_gateway_id.clone(),
        caller_id_mode,
        virtual_caller,
        current_concurrent: old.current_concurrent,
        circuit_state: old.circuit_state.clone(),
        account_id: req.account_id.or(old.account_id),
        max_concurrent: req.max_concurrent.or(old.max_concurrent),
        enabled: req.enabled.or(old.enabled),
        created_at: old.created_at,
    };
    state
        .store
        .upsert_gateway_full(&gw)
        .await
        .map_err(|e| ApiError {
            error: e.to_string(),
        })?;
    crate::routes::publish_route_reload(&state.nats_client).await;
    Ok(StatusCode::OK)
}

#[cfg(test)]
mod tests {
    use super::validate_gateway;

    #[test]
    fn validates_transport_port_and_virtual_caller() {
        assert!(validate_gateway(
            "gw",
            "127.0.0.1",
            Some(5060),
            "udp",
            None,
            None,
            Some("egress"),
            None
        )
        .is_ok());
        assert!(
            validate_gateway("gw", "127.0.0.1", Some(0), "udp", None, None, None, None).is_err()
        );
        assert!(
            validate_gateway("gw", "127.0.0.1", Some(5060), "ws", None, None, None, None).is_err()
        );
        assert!(
            validate_gateway("gw", "127.0.0.1", Some(5060), "tcp", None, None, None, None).is_err()
        );
        assert!(validate_gateway(
            "gw",
            "127.0.0.1",
            Some(5060),
            "udp",
            Some("virtual"),
            None,
            None,
            None
        )
        .is_err());
        assert!(validate_gateway(
            "access",
            "127.0.0.1",
            Some(5060),
            "udp",
            None,
            None,
            Some("access"),
            Some("none")
        )
        .is_err());
        assert!(validate_gateway(
            "access",
            "127.0.0.1",
            Some(5060),
            "udp",
            None,
            None,
            Some("access"),
            Some("ip_allowlist")
        )
        .is_ok());
    }
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
        crate::routes::publish_route_reload(&state.nats_client).await;
        Ok(StatusCode::OK)
    } else {
        Ok(StatusCode::NOT_FOUND)
    }
}
