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
) -> Result<Json<PaginatedResponse<SipGateway>>, ApiError> {
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
    let role = req.role.as_deref().unwrap_or("egress");
    let is_access = role == "access";
    let access_auth_mode = req.access_auth_mode.as_deref().map(|mode| {
        if mode == "ip_whitelist" {
            "ip_allowlist"
        } else {
            mode
        }
    });
    let uses_access_digest =
        is_access && matches!(access_auth_mode, Some("digest_register" | "ip_and_digest"));
    reject_unsupported_egress_secret(role, req.reg_password.as_deref())?;
    let access_username = uses_access_digest
        .then(|| {
            req.access_username
                .clone()
                .or_else(|| req.reg_username.clone())
        })
        .flatten();
    if req
        .access_realm
        .as_deref()
        .is_some_and(|realm| realm.trim() != state.sip_auth_realm)
    {
        return Err(ApiError::internal(format!(
            "参数无效: 认证 Realm 必须使用系统配置 {}",
            state.sip_auth_realm
        )));
    }
    let access_realm = is_access.then(|| state.sip_auth_realm.clone());
    let access_password = uses_access_digest
        .then(|| {
            req.access_password
                .clone()
                .or_else(|| req.reg_password.clone())
        })
        .flatten();
    validate_gateway(
        &req.id,
        &req.host,
        req.port,
        &req.transport,
        req.caller_id_mode.as_deref(),
        req.virtual_caller.as_deref(),
        req.role.as_deref(),
        access_auth_mode,
        access_username.as_deref(),
        access_realm.as_deref(),
        access_password
            .as_deref()
            .is_some_and(|value| !value.is_empty()),
    )?;
    let access_password_hash = access_password.as_deref().and_then(|password| {
        Some(access_ha1(
            access_username.as_deref()?,
            access_realm.as_deref()?,
            password,
        ))
    });
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
        access_username,
        access_realm,
        access_password_hash,
        has_access_password: access_password.is_some(),
        prefix_rules: req.prefix_rules,
        supports_registration: is_access
            .then_some(uses_access_digest)
            .or(req.supports_registration),
        reg_auth_type: req.reg_auth_type,
        reg_username: req.reg_username,
        // Legacy reg_password must never be persisted as plaintext. Access
        // credentials are converted to Digest HA1 above; active upstream
        // registration will use a dedicated encrypted credential model.
        reg_password: if is_access {
            None
        } else {
            req.reg_password.clone()
        },
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
    let role = req
        .role
        .as_deref()
        .or(old.role.as_deref())
        .unwrap_or("egress");
    let is_access = role == "access";
    let access_auth_mode = req
        .access_auth_mode
        .as_deref()
        .or(old.access_auth_mode.as_deref())
        .map(|mode| {
            if mode == "ip_whitelist" {
                "ip_allowlist"
            } else {
                mode
            }
        });
    let uses_access_digest =
        is_access && matches!(access_auth_mode, Some("digest_register" | "ip_and_digest"));
    reject_unsupported_egress_secret(role, req.reg_password.as_deref())?;
    if req
        .role
        .as_deref()
        .is_some_and(|role| old.role.as_deref() != Some(role))
    {
        return Err(ApiError::internal(
            "参数无效: 中继创建后不能切换接入/落地类型，请新建对应类型中继",
        ));
    }
    let caller_id_mode = req
        .caller_id_mode
        .clone()
        .or_else(|| old.caller_id_mode.clone());
    let virtual_caller = req
        .virtual_caller
        .clone()
        .or_else(|| old.virtual_caller.clone());
    let access_username = uses_access_digest
        .then(|| {
            req.access_username
                .clone()
                .or_else(|| old.access_username.clone())
        })
        .flatten();
    if req
        .access_realm
        .as_deref()
        .is_some_and(|realm| realm.trim() != state.sip_auth_realm)
    {
        return Err(ApiError::internal(format!(
            "参数无效: 认证 Realm 必须使用系统配置 {}",
            state.sip_auth_realm
        )));
    }
    let access_realm = req
        .role
        .as_deref()
        .or(old.role.as_deref())
        .filter(|role| *role == "access")
        .map(|_| state.sip_auth_realm.clone());
    let identity_changed = uses_access_digest
        && (access_username != old.access_username || access_realm != old.access_realm);
    if identity_changed
        && req
            .access_password
            .as_deref()
            .is_none_or(|password| password.is_empty())
    {
        return Err(ApiError::internal(
            "参数无效: 修改注册用户名或 Realm 时必须重新输入密码",
        ));
    }
    validate_gateway(
        &id,
        &req.host,
        req.port,
        &req.transport,
        caller_id_mode.as_deref(),
        virtual_caller.as_deref(),
        req.role.as_deref().or(old.role.as_deref()),
        access_auth_mode,
        access_username.as_deref(),
        access_realm.as_deref(),
        req.access_password
            .as_deref()
            .is_some_and(|value| !value.is_empty())
            || old.has_access_password,
    )?;
    let access_password_hash = if uses_access_digest {
        req.access_password.as_deref().and_then(|password| {
            Some(access_ha1(
                access_username.as_deref()?,
                access_realm.as_deref()?,
                password,
            ))
        })
    } else {
        // Explicitly clear dormant Digest material when authentication no longer
        // uses it. A later switch back must provide a fresh password.
        Some(String::new())
    };
    let gw = SipGateway {
        id: id.clone(),
        host: req.host,
        port: req.port,
        transport: req.transport,
        max_capacity: req.max_capacity.filter(|capacity| *capacity > 0),
        gateway_type: req.gateway_type.or_else(|| old.gateway_type.clone()),
        role: req.role.or_else(|| old.role.clone()),
        access_auth_mode: access_auth_mode.map(str::to_string),
        access_username,
        access_realm,
        access_password_hash,
        has_access_password: uses_access_digest
            && (req
                .access_password
                .as_deref()
                .is_some_and(|password| !password.is_empty())
                || old.has_access_password),
        prefix_rules: req.prefix_rules.or_else(|| old.prefix_rules.clone()),
        supports_registration: is_access
            .then_some(uses_access_digest)
            .or(req.supports_registration)
            .or(old.supports_registration),
        reg_auth_type: req.reg_auth_type.or_else(|| old.reg_auth_type.clone()),
        reg_username: req.reg_username.or_else(|| old.reg_username.clone()),
        reg_password: if is_access {
            None
        } else {
            req.reg_password.clone()
        },
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
