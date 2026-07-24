use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use cdr_core::SipGateway;

use super::{
    access_ha1, reject_unsupported_egress_secret, validate_gateway, CreateGatewayRequest,
    UpdateGatewayRequest,
};
use crate::{ApiError, AppState};

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
    crate::resources::routes::publish_route_reload(&state.nats_client).await;
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
    crate::resources::routes::publish_route_reload(&state.nats_client).await;
    Ok(StatusCode::OK)
}
