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
    pub prefix_rules: Option<String>,
    pub supports_registration: Option<bool>,
    pub reg_auth_type: Option<String>,
    pub reg_username: Option<String>,
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
    pub prefix_rules: Option<String>,
    pub supports_registration: Option<bool>,
    pub reg_auth_type: Option<String>,
    pub reg_username: Option<String>,
    pub caller_id_mode: Option<String>,
    pub virtual_caller: Option<String>,
    pub max_concurrent: Option<i32>,
    pub account_id: Option<i64>,
    pub enabled: Option<bool>,
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
    let gw = SipGateway {
        id: req.id,
        host: req.host,
        port: req.port,
        transport: req.transport,
        max_capacity: req.max_capacity,
        gateway_type: req.gateway_type,
        prefix_rules: req.prefix_rules,
        supports_registration: req.supports_registration,
        reg_auth_type: req.reg_auth_type,
        reg_username: req.reg_username,
        reg_password: None,
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
    let gw = SipGateway {
        id: id.clone(),
        host: req.host,
        port: req.port,
        transport: req.transport,
        max_capacity: req.max_capacity,
        gateway_type: req.gateway_type.or_else(|| old.gateway_type.clone()),
        prefix_rules: req.prefix_rules.or_else(|| old.prefix_rules.clone()),
        supports_registration: req.supports_registration.or(old.supports_registration),
        reg_auth_type: req.reg_auth_type.or_else(|| old.reg_auth_type.clone()),
        reg_username: req.reg_username.or_else(|| old.reg_username.clone()),
        reg_password: None,
        parent_gateway_id: old.parent_gateway_id.clone(),
        caller_id_mode: req.caller_id_mode.or_else(|| old.caller_id_mode.clone()),
        virtual_caller: req.virtual_caller.or_else(|| old.virtual_caller.clone()),
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
        Ok(StatusCode::OK)
    } else {
        Ok(StatusCode::NOT_FOUND)
    }
}
