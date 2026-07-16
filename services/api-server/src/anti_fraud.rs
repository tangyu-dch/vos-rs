use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use cdr_core::{AntiFraudConfigItem, AntiFraudRule};
use serde::Deserialize;

use crate::{ApiError, AppState};

#[derive(Deserialize)]
pub struct CreateAntiFraudRuleRequest {
    pub id: String,
    pub rule_type: String,
    pub target_value: String,
    pub limit_number: Option<i32>,
    pub enabled: bool,
}

#[derive(Deserialize)]
pub struct UpdateAntiFraudRuleRequest {
    pub rule_type: String,
    pub target_value: String,
    pub limit_number: Option<i32>,
    pub enabled: bool,
}

#[derive(Debug, Deserialize)]
pub struct UpdateAntiFraudConfigRequest {
    pub config_value: String,
}

pub async fn list_anti_fraud_rules(
    State(state): State<AppState>,
) -> Result<Json<Vec<AntiFraudRule>>, ApiError> {
    state
        .store
        .list_anti_fraud_rules()
        .await
        .map(Json)
        .map_err(|e| ApiError {
            error: e.to_string(),
        })
}

pub async fn create_anti_fraud_rule(
    State(state): State<AppState>,
    Json(req): Json<CreateAntiFraudRuleRequest>,
) -> Result<StatusCode, ApiError> {
    let rule = AntiFraudRule {
        id: req.id,
        rule_type: req.rule_type,
        target_value: req.target_value,
        limit_number: req.limit_number,
        enabled: req.enabled,
    };
    state
        .store
        .insert_anti_fraud_rule(&rule)
        .await
        .map_err(|e| ApiError {
            error: e.to_string(),
        })?;
    crate::routes::publish_route_reload(&state.nats_client).await;
    Ok(StatusCode::CREATED)
}

pub async fn update_anti_fraud_rule(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateAntiFraudRuleRequest>,
) -> Result<StatusCode, ApiError> {
    let rule = AntiFraudRule {
        id,
        rule_type: req.rule_type,
        target_value: req.target_value,
        limit_number: req.limit_number,
        enabled: req.enabled,
    };
    state
        .store
        .insert_anti_fraud_rule(&rule)
        .await
        .map_err(|e| ApiError {
            error: e.to_string(),
        })?;
    crate::routes::publish_route_reload(&state.nats_client).await;
    Ok(StatusCode::OK)
}

pub async fn delete_anti_fraud_rule(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let deleted = state
        .store
        .delete_anti_fraud_rule(&id)
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

pub async fn list_anti_fraud_config(
    State(state): State<AppState>,
) -> Result<Json<Vec<AntiFraudConfigItem>>, ApiError> {
    state
        .store
        .list_anti_fraud_configs()
        .await
        .map(Json)
        .map_err(|e| ApiError {
            error: e.to_string(),
        })
}

pub async fn update_anti_fraud_config(
    State(state): State<AppState>,
    Path(key): Path<String>,
    Json(req): Json<UpdateAntiFraudConfigRequest>,
) -> Result<StatusCode, ApiError> {
    if req.config_value.len() > 1024 {
        return Err(ApiError::internal("防盗打配置值长度不能超过 1024 个字符"));
    }

    let updated = state
        .store
        .update_anti_fraud_config(&key, &req.config_value)
        .await
        .map_err(|e| ApiError {
            error: e.to_string(),
        })?;

    if updated {
        crate::routes::publish_route_reload(&state.nats_client).await;
        Ok(StatusCode::OK)
    } else {
        Err(ApiError::internal("防盗打配置项不存在"))
    }
}
