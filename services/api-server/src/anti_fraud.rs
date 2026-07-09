use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post, put},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use crate::AppState;

/// 防盗打规则
#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct AntiFraudRule {
    pub id: i64,
    pub rule_type: String,
    pub value: String,
    pub description: Option<String>,
    pub enabled: bool,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// 防盗打配置
#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct AntiFraudConfigItem {
    pub id: i64,
    pub config_key: String,
    pub config_value: String,
    pub description: Option<String>,
    pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// 防盗打事件
#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct AntiFraudEvent {
    pub id: i64,
    pub event_type: String,
    pub source_ip: Option<String>,
    pub account: Option<String>,
    pub destination: Option<String>,
    pub detail: Option<String>,
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// 创建规则请求
#[derive(Debug, Deserialize)]
pub struct CreateRuleRequest {
    pub rule_type: String,
    pub value: String,
    pub description: Option<String>,
    pub enabled: Option<bool>,
}

/// 更新规则请求
#[derive(Debug, Deserialize)]
pub struct UpdateRuleRequest {
    pub description: Option<String>,
    pub enabled: Option<bool>,
}

/// 更新配置请求
#[derive(Debug, Deserialize)]
pub struct UpdateConfigRequest {
    pub config_value: String,
}

/// 获取所有规则
pub async fn list_rules(
    State(state): State<AppState>,
) -> Result<Json<Vec<AntiFraudRule>>, (StatusCode, String)> {
    let pool = state.store.pool();
    let rules = sqlx::query_as::<_, AntiFraudRule>(
        "SELECT id, rule_type, value, description, enabled, created_at, updated_at 
         FROM anti_fraud_rules ORDER BY rule_type, value"
    )
    .fetch_all(pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(rules))
}

/// 获取指定类型的规则
pub async fn list_rules_by_type(
    State(state): State<AppState>,
    Path(rule_type): Path<String>,
) -> Result<Json<Vec<AntiFraudRule>>, (StatusCode, String)> {
    let pool = state.store.pool();
    let rules = sqlx::query_as::<_, AntiFraudRule>(
        "SELECT id, rule_type, value, description, enabled, created_at, updated_at 
         FROM anti_fraud_rules WHERE rule_type = $1 ORDER BY value"
    )
    .bind(&rule_type)
    .fetch_all(pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(rules))
}

/// 创建规则
pub async fn create_rule(
    State(state): State<AppState>,
    Json(req): Json<CreateRuleRequest>,
) -> Result<(StatusCode, Json<AntiFraudRule>), (StatusCode, String)> {
    let pool = state.store.pool();
    let rule = sqlx::query_as::<_, AntiFraudRule>(
        "INSERT INTO anti_fraud_rules (rule_type, value, description, enabled) 
         VALUES ($1, $2, $3, $4) 
         RETURNING id, rule_type, value, description, enabled, created_at, updated_at"
    )
    .bind(&req.rule_type)
    .bind(&req.value)
    .bind(&req.description)
    .bind(req.enabled.unwrap_or(true))
    .fetch_one(pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok((StatusCode::CREATED, Json(rule)))
}

/// 更新规则
pub async fn update_rule(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(req): Json<UpdateRuleRequest>,
) -> Result<Json<AntiFraudRule>, (StatusCode, String)> {
    let pool = state.store.pool();
    let rule = sqlx::query_as::<_, AntiFraudRule>(
        "UPDATE anti_fraud_rules 
         SET description = COALESCE($1, description),
             enabled = COALESCE($2, enabled),
             updated_at = NOW()
         WHERE id = $3
         RETURNING id, rule_type, value, description, enabled, created_at, updated_at"
    )
    .bind(&req.description)
    .bind(req.enabled)
    .bind(id)
    .fetch_one(pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(rule))
}

/// 删除规则
pub async fn delete_rule(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<StatusCode, (StatusCode, String)> {
    let pool = state.store.pool();
    sqlx::query("DELETE FROM anti_fraud_rules WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(StatusCode::OK)
}

/// 获取所有配置
pub async fn list_config(
    State(state): State<AppState>,
) -> Result<Json<Vec<AntiFraudConfigItem>>, (StatusCode, String)> {
    let pool = state.store.pool();
    let config = sqlx::query_as::<_, AntiFraudConfigItem>(
        "SELECT id, config_key, config_value, description, updated_at 
         FROM anti_fraud_config ORDER BY config_key"
    )
    .fetch_all(pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(config))
}

/// 更新配置
pub async fn update_config(
    State(state): State<AppState>,
    Path(key): Path<String>,
    Json(req): Json<UpdateConfigRequest>,
) -> Result<Json<AntiFraudConfigItem>, (StatusCode, String)> {
    let pool = state.store.pool();
    let config = sqlx::query_as::<_, AntiFraudConfigItem>(
        "UPDATE anti_fraud_config 
         SET config_value = $1, updated_at = NOW()
         WHERE config_key = $2
         RETURNING id, config_key, config_value, description, updated_at"
    )
    .bind(&req.config_value)
    .bind(&key)
    .fetch_one(pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(config))
}

/// 获取最近事件
pub async fn list_events(
    State(state): State<AppState>,
) -> Result<Json<Vec<AntiFraudEvent>>, (StatusCode, String)> {
    let pool = state.store.pool();
    let events = sqlx::query_as::<_, AntiFraudEvent>(
        "SELECT id, event_type, source_ip::text, account, destination, detail, created_at 
         FROM anti_fraud_events ORDER BY created_at DESC LIMIT 100"
    )
    .fetch_all(pool)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(events))
}

/// 记录防盗打事件
pub async fn log_event(
    pool: &PgPool,
    event_type: &str,
    source_ip: Option<&str>,
    account: Option<&str>,
    destination: Option<&str>,
    detail: Option<&str>,
) {
    let _ = sqlx::query(
        "INSERT INTO anti_fraud_events (event_type, source_ip, account, destination, detail) 
         VALUES ($1, $2::inet, $3, $4, $5)"
    )
    .bind(event_type)
    .bind(source_ip)
    .bind(account)
    .bind(destination)
    .bind(detail)
    .execute(pool)
    .await;
}

/// 创建路由
pub fn anti_fraud_routes() -> Router<AppState> {
    Router::new()
        .route("/rules", get(list_rules).post(create_rule))
        .route("/rules/{rule_type}", get(list_rules_by_type))
        .route("/rules/id/{id}", put(update_rule).delete(delete_rule))
        .route("/config", get(list_config))
        .route("/config/{key}", put(update_config))
        .route("/events", get(list_events))
}
