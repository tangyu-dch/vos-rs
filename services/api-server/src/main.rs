mod billing;
mod calls;
mod metrics;
mod numbers;
mod recording;
mod report;

use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::IntoResponse,
    routing::{get, post, put},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::env;
use std::sync::Arc;
use time::OffsetDateTime;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use billing::{
    create_rate, credit_account, delete_rate, list_accounts, list_ledger, list_rates,
    reconcile as billing_reconcile, update_rate,
};
use calls::{list_active, media_metrics, route_preview, terminate_call as calls_terminate};
use cdr_core::{
    AntiFraudRule, CdrEvent, DashboardStats, DtmfEventRecord, HourlyTrend, PostgresCdrStore,
    SipGateway, SipRegistration, SipRoute, SipUser,
};
use metrics::{MediaMetricsSnapshot, Metrics};
use numbers::{create_number, delete_number, list_numbers, update_number};
use recording::{get_recording_audio, list_recordings};
use report::{export_cdrs_csv, get_report_summary};

use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Claims {
    pub sub: String,  // 登录用户名
    pub role: String, // 角色
    pub exp: usize,   // 过期时间戳
}

#[derive(Debug, Deserialize)]
struct LoginRequest {
    username: String,
    password: String,
}

#[derive(Debug, Serialize)]
struct LoginResponse {
    token: String,
    username: String,
    role: String,
}

#[derive(Clone)]
pub(crate) struct AppState {
    store: Arc<PostgresCdrStore>,
    recording_storage: Arc<dyn storage_core::StorageBackend>,
    sip_manage_base: String,
    nats_client: Option<async_nats::Client>,
    jwt_secret: Vec<u8>,
}

#[derive(Debug, Deserialize)]
struct ListCdrsQuery {
    page: Option<i64>,
    page_size: Option<i64>,
    status: Option<String>,
    caller: Option<String>,
    callee: Option<String>,
    start_time: Option<String>,
    end_time: Option<String>,
}

#[derive(Debug, Serialize)]
struct PaginatedResponse<T> {
    items: Vec<T>,
    total: i64,
    page: i64,
    page_size: i64,
}

#[derive(Debug, Deserialize)]
struct CreateUserRequest {
    username: String,
    password: String,
}

#[derive(Debug, Deserialize)]
struct UpdateUserRequest {
    password: String,
}

#[derive(Debug, Deserialize)]
struct CreateGatewayRequest {
    id: String,
    host: String,
    port: Option<u16>,
    transport: String,
    max_capacity: Option<u32>,
    gateway_type: Option<String>,
    prefix_rules: Option<String>,
    supports_registration: Option<bool>,
    reg_auth_type: Option<String>,
    reg_username: Option<String>,
    caller_id_mode: Option<String>,
    virtual_caller: Option<String>,
    max_concurrent: Option<i32>,
    account_id: Option<i64>,
    enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct UpdateGatewayRequest {
    host: String,
    port: Option<u16>,
    transport: String,
    max_capacity: Option<u32>,
    gateway_type: Option<String>,
    prefix_rules: Option<String>,
    supports_registration: Option<bool>,
    reg_auth_type: Option<String>,
    reg_username: Option<String>,
    caller_id_mode: Option<String>,
    virtual_caller: Option<String>,
    max_concurrent: Option<i32>,
    account_id: Option<i64>,
    enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct CreateRouteRequest {
    id: String,
    prefix: String,
    priority: i32,
    gateway_id: String,
    cost: f64,
    weight: Option<i32>,
    time_start: Option<String>,
    time_end: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UpdateRouteRequest {
    prefix: String,
    priority: i32,
    gateway_id: String,
    cost: f64,
    weight: Option<i32>,
    time_start: Option<String>,
    time_end: Option<String>,
}

#[derive(Debug, Serialize)]
struct ApiError {
    error: String,
}

impl ApiError {
    fn internal(msg: impl Into<String>) -> Self {
        Self { error: msg.into() }
    }

    fn unauthorized(msg: impl Into<String>) -> Self {
        Self { error: msg.into() }
    }

    fn forbidden(msg: impl Into<String>) -> Self {
        Self { error: msg.into() }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        let is_unauthorized = self.error.contains("用户名或密码错误")
            || self.error.contains("缺少凭证")
            || self.error.contains("凭证格式不正确")
            || self.error.contains("无效 Token");

        let is_forbidden = self.error.contains("越权") || self.error.contains("无权");

        let status = if is_unauthorized {
            StatusCode::UNAUTHORIZED
        } else if is_forbidden {
            StatusCode::FORBIDDEN
        } else {
            StatusCode::INTERNAL_SERVER_ERROR
        };
        (status, Json(self)).into_response()
    }
}

// ===== 路由处理器 =====

async fn health() -> &'static str {
    "OK"
}

async fn get_dashboard_stats(
    State(state): State<AppState>,
) -> Result<Json<DashboardStats>, ApiError> {
    let active_calls = {
        let url = format!("{}/manage/active-calls", state.sip_manage_base);
        match reqwest::get(&url).await {
            Ok(resp) => {
                let text = resp.text().await.unwrap_or_default();
                serde_json::from_str::<serde_json::Value>(&text)
                    .ok()
                    .and_then(|v| v["active_calls"].as_i64())
                    .unwrap_or(0)
            }
            Err(_) => 0,
        }
    };
    state
        .store
        .get_dashboard_stats(active_calls)
        .await
        .map(Json)
        .map_err(|e| ApiError {
            error: e.to_string(),
        })
}

async fn get_dashboard_trend(
    State(state): State<AppState>,
) -> Result<Json<Vec<HourlyTrend>>, ApiError> {
    state
        .store
        .get_hourly_trend()
        .await
        .map(Json)
        .map_err(|e| ApiError {
            error: e.to_string(),
        })
}

async fn list_cdrs(
    State(state): State<AppState>,
    Query(query): Query<ListCdrsQuery>,
) -> Result<Json<PaginatedResponse<CdrEvent>>, ApiError> {
    let page = query.page.unwrap_or(1);
    let page_size = query.page_size.unwrap_or(20).min(100);

    let start = query.start_time.as_deref().and_then(parse_dt);
    let end = query.end_time.as_deref().and_then(parse_dt);

    let (items, total) = state
        .store
        .list_cdrs(
            page,
            page_size,
            query.status.as_deref(),
            query.caller.as_deref(),
            query.callee.as_deref(),
            start,
            end,
        )
        .await
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

pub(crate) fn parse_dt(s: &str) -> Option<OffsetDateTime> {
    OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339).ok()
}

async fn get_cdr(
    State(state): State<AppState>,
    Path(call_id): Path<String>,
) -> Result<Json<Option<CdrEvent>>, ApiError> {
    state
        .store
        .get_cdr(&call_id)
        .await
        .map(Json)
        .map_err(|e| ApiError {
            error: e.to_string(),
        })
}

async fn get_dtmf_events(
    State(state): State<AppState>,
    Path(call_id): Path<String>,
) -> Result<Json<Vec<DtmfEventRecord>>, ApiError> {
    state
        .store
        .get_dtmf_events(&call_id)
        .await
        .map(Json)
        .map_err(|e| ApiError {
            error: e.to_string(),
        })
}

async fn list_users(State(state): State<AppState>) -> Result<Json<Vec<SipUser>>, ApiError> {
    state
        .store
        .list_users()
        .await
        .map(Json)
        .map_err(|e| ApiError {
            error: e.to_string(),
        })
}

async fn create_user(
    State(state): State<AppState>,
    Json(req): Json<CreateUserRequest>,
) -> Result<StatusCode, ApiError> {
    // 强制转换为 HA1 哈希，防止明文存储
    let ha1 = format!(
        "{:x}",
        md5::compute(format!("{}:{}:{}", req.username, "vos-rs", req.password).as_bytes())
    );
    state
        .store
        .insert_user(&req.username, &ha1)
        .await
        .map_err(|e| ApiError {
            error: e.to_string(),
        })?;
    Ok(StatusCode::CREATED)
}

async fn update_user(
    State(state): State<AppState>,
    Path(username): Path<String>,
    Json(req): Json<UpdateUserRequest>,
) -> Result<StatusCode, ApiError> {
    // 强制转换为 HA1 哈希，防止明文存储
    let ha1 = format!(
        "{:x}",
        md5::compute(format!("{}:{}:{}", username, "vos-rs", req.password).as_bytes())
    );
    state
        .store
        .insert_user(&username, &ha1)
        .await
        .map_err(|e| ApiError {
            error: e.to_string(),
        })?;
    Ok(StatusCode::OK)
}

async fn delete_user(
    State(state): State<AppState>,
    Path(username): Path<String>,
) -> Result<StatusCode, ApiError> {
    let deleted = state
        .store
        .delete_user(&username)
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

async fn list_gateways(State(state): State<AppState>) -> Result<Json<Vec<SipGateway>>, ApiError> {
    state
        .store
        .list_gateways_full()
        .await
        .map(Json)
        .map_err(|e| ApiError {
            error: e.to_string(),
        })
}

async fn create_gateway(
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

async fn update_gateway(
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

async fn delete_gateway(
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

async fn publish_route_reload(nats: &Option<async_nats::Client>) {
    if let Some(client) = nats {
        if let Err(e) = client
            .publish("vos_rs.routing.reload", axum::body::Bytes::from("reload"))
            .await
        {
            tracing::warn!(error = %e, "NATS 路由重载广播发布失败");
        }
    }
}

async fn list_routes(State(state): State<AppState>) -> Result<Json<Vec<SipRoute>>, ApiError> {
    state
        .store
        .list_routes_full()
        .await
        .map(Json)
        .map_err(|e| ApiError {
            error: e.to_string(),
        })
}

async fn create_route(
    State(state): State<AppState>,
    Json(req): Json<CreateRouteRequest>,
) -> Result<StatusCode, ApiError> {
    let weight = req.weight.unwrap_or(100).clamp(1, 10000);
    state
        .store
        .insert_route_with_cost(
            &req.id,
            &req.prefix,
            req.priority,
            &req.gateway_id,
            req.cost,
            weight,
            req.time_start.as_deref(),
            req.time_end.as_deref(),
        )
        .await
        .map_err(|e| ApiError {
            error: e.to_string(),
        })?;
    publish_route_reload(&state.nats_client).await;
    Ok(StatusCode::CREATED)
}

async fn update_route(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateRouteRequest>,
) -> Result<StatusCode, ApiError> {
    let weight = req.weight.unwrap_or(100).clamp(1, 10000);
    state
        .store
        .insert_route_with_cost(
            &id,
            &req.prefix,
            req.priority,
            &req.gateway_id,
            req.cost,
            weight,
            req.time_start.as_deref(),
            req.time_end.as_deref(),
        )
        .await
        .map_err(|e| ApiError {
            error: e.to_string(),
        })?;
    publish_route_reload(&state.nats_client).await;
    Ok(StatusCode::OK)
}

async fn delete_route(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let deleted = state.store.delete_route(&id).await.map_err(|e| ApiError {
        error: e.to_string(),
    })?;
    if deleted {
        publish_route_reload(&state.nats_client).await;
        Ok(StatusCode::OK)
    } else {
        Ok(StatusCode::NOT_FOUND)
    }
}

async fn list_registrations(
    State(state): State<AppState>,
) -> Result<Json<Vec<SipRegistration>>, ApiError> {
    state
        .store
        .list_registrations()
        .await
        .map(Json)
        .map_err(|e| ApiError {
            error: e.to_string(),
        })
}

#[derive(Deserialize)]
struct CreateAntiFraudRuleRequest {
    id: String,
    rule_type: String,
    target_value: String,
    limit_number: Option<i32>,
    enabled: bool,
}

#[derive(Deserialize)]
struct UpdateAntiFraudRuleRequest {
    rule_type: String,
    target_value: String,
    limit_number: Option<i32>,
    enabled: bool,
}

#[derive(Serialize)]
struct AntiFraudConfigItem {
    key: String,
    value: String,
    description: String,
}

async fn list_anti_fraud_rules(
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

async fn create_anti_fraud_rule(
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
    Ok(StatusCode::CREATED)
}

async fn update_anti_fraud_rule(
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
    Ok(StatusCode::OK)
}

async fn delete_anti_fraud_rule(
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
        Ok(StatusCode::OK)
    } else {
        Ok(StatusCode::NOT_FOUND)
    }
}

async fn list_anti_fraud_config() -> Result<Json<Vec<AntiFraudConfigItem>>, ApiError> {
    Ok(Json(Vec::new()))
}

async fn prometheus_metrics(State(state): State<AppState>) -> impl IntoResponse {
    let url = format!("{}/manage/media-metrics", state.sip_manage_base);
    let secret = match env::var("VOS_RS_INTERNAL_SECRET") {
        Ok(val) => val,
        Err(_) => {
            tracing::warn!("VOS_RS_INTERNAL_SECRET 未配置，跳过 sip-edge 指标拉取");
            let mut headers = HeaderMap::new();
            headers.insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/plain; version=0.0.4; charset=utf-8"),
            );
            return (headers, Metrics::encode_metrics());
        }
    };
    match reqwest::Client::new()
        .get(&url)
        .header("X-VOS-Token", secret)
        .send()
        .await
    {
        Ok(response) => match response.json::<MediaMetricsSnapshot>().await {
            Ok(snapshot) => Metrics::update_media_metrics(&snapshot),
            Err(error) => tracing::debug!(%error, "failed to decode sip-edge media metrics"),
        },
        Err(error) => tracing::debug!(%error, "failed to fetch sip-edge media metrics"),
    }

    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; version=0.0.4; charset=utf-8"),
    );
    (headers, Metrics::encode_metrics())
}

use std::time::SystemTime;

async fn login(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, ApiError> {
    let admin_password = env::var("VOS_RS_ADMIN_PASSWORD")
        .map_err(|_| ApiError::internal("VOS_RS_ADMIN_PASSWORD 未配置，无法登录"))?;
    let operator_password = env::var("VOS_RS_OPERATOR_PASSWORD")
        .map_err(|_| ApiError::internal("VOS_RS_OPERATOR_PASSWORD 未配置，无法登录"))?;
    let financier_password = env::var("VOS_RS_FINANCIER_PASSWORD")
        .map_err(|_| ApiError::internal("VOS_RS_FINANCIER_PASSWORD 未配置，无法登录"))?;

    let role = if req.username == "admin" && req.password == admin_password {
        "admin".to_string()
    } else if req.username == "operator" && req.password == operator_password {
        "operator".to_string()
    } else if req.username == "financier" && req.password == financier_password {
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

fn role_allows(role: &str, method: &str, path: &str) -> bool {
    if role == "admin" {
        return true;
    }

    // SIP user credentials are security-sensitive and remain administrator-only.
    if path.starts_with("/api/users") {
        return false;
    }

    let finance_path = path.starts_with("/api/rates")
        || path.starts_with("/api/accounts")
        || path.starts_with("/api/ledger")
        || path.starts_with("/api/billing");
    if finance_path {
        return role == "financier";
    }

    let operations_path = path.starts_with("/api/gateways")
        || path.starts_with("/api/routes")
        || path.starts_with("/api/numbers")
        || path.starts_with("/api/anti-fraud")
        || (path.starts_with("/api/calls/") && method == "POST");
    if operations_path {
        return role == "operator";
    }

    let read_only_path = path.starts_with("/api/dashboard")
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

async fn jwt_auth(
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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "api_server=debug,tower_http=debug,info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let database_url = env::var("VOS_RS_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://localhost/vos_rs".to_string());

    let store = PostgresCdrStore::connect(&database_url).await?;
    let storage_config = storage_core::StorageConfig::from_env();
    let recording_storage: Arc<dyn storage_core::StorageBackend> =
        storage_core::create_storage(&storage_config).await?.into();
    let nats_url =
        env::var("VOS_RS_NATS_URL").unwrap_or_else(|_| "nats://127.0.0.1:4222".to_string());
    let nats_client = async_nats::connect(&nats_url).await.ok();
    let sip_manage_base =
        env::var("VOS_RS_MANAGE_BASE").unwrap_or_else(|_| "http://127.0.0.1:8082".to_string());

    let jwt_secret = match env::var("VOS_RS_API_JWT_SECRET") {
        Ok(val) if !val.trim().is_empty() => val.into_bytes(),
        _ => {
            if env::var("VOS_RS_ENV").unwrap_or_default() == "production" {
                panic!("致命安全错误: 生产环境下必须配置 VOS_RS_API_JWT_SECRET 密钥！");
            } else {
                tracing::warn!(
                    "警告: 未配置 VOS_RS_API_JWT_SECRET，将回退为默认不安全密钥进行开发调试"
                );
                b"vos-rs-secret-key-change-in-production".to_vec()
            }
        }
    };

    let state = AppState {
        store: Arc::new(store),
        recording_storage,
        sip_manage_base,
        nats_client,
        jwt_secret,
    };

    let cors_origins_raw = env::var("VOS_RS_API_ALLOWED_ORIGINS").unwrap_or_default();
    let cors = CorsLayer::new();
    let cors = if !cors_origins_raw.trim().is_empty() {
        let mut origins = Vec::new();
        for origin in cors_origins_raw.split(',') {
            if let Ok(val) = origin.trim().parse::<HeaderValue>() {
                origins.push(val);
            }
        }
        cors.allow_origin(origins)
    } else {
        tracing::warn!("警告: 未配置 VOS_RS_API_ALLOWED_ORIGINS，默认只允许 localhost:3000 和 localhost:8080 开发域名跨域访问");
        cors.allow_origin([
            "http://localhost:3000".parse().unwrap(),
            "http://localhost:8080".parse().unwrap(),
        ])
    };
    let cors = cors
        .allow_methods([
            axum::http::Method::GET,
            axum::http::Method::POST,
            axum::http::Method::PUT,
            axum::http::Method::DELETE,
        ])
        .allow_headers([
            axum::http::header::AUTHORIZATION,
            axum::http::header::CONTENT_TYPE,
        ]);

    let public_routes = Router::new()
        .route("/health", get(health))
        .route("/metrics", get(prometheus_metrics))
        .route("/api/auth/login", post(login));

    let protected_routes = Router::new()
        .route("/api/dashboard/stats", get(get_dashboard_stats))
        .route("/api/dashboard/trend", get(get_dashboard_trend))
        .route("/api/cdrs", get(list_cdrs))
        .route("/api/cdrs/:call_id", get(get_cdr))
        .route("/api/cdrs/:call_id/dtmf", get(get_dtmf_events))
        .route("/api/users", get(list_users).post(create_user))
        .route("/api/users/:username", put(update_user).delete(delete_user))
        .route("/api/gateways", get(list_gateways).post(create_gateway))
        .route(
            "/api/gateways/:id",
            put(update_gateway).delete(delete_gateway),
        )
        .route("/api/routes", get(list_routes).post(create_route))
        .route("/api/routes/:id", put(update_route).delete(delete_route))
        .route("/api/registrations", get(list_registrations))
        .route("/api/recordings", get(list_recordings))
        .route("/api/recordings/:call_id/audio", get(get_recording_audio))
        .route("/api/reports/summary", get(get_report_summary))
        .route("/api/reports/export", get(export_cdrs_csv))
        .route("/api/rates", get(list_rates).post(create_rate))
        .route("/api/rates/:id", put(update_rate).delete(delete_rate))
        .route("/api/accounts", get(list_accounts))
        .route("/api/accounts/:username/credit", post(credit_account))
        .route("/api/ledger", get(list_ledger))
        .route("/api/billing/reconcile", post(billing_reconcile))
        .route("/api/calls/active", get(list_active))
        .route("/api/calls/:call_id/terminate", post(calls_terminate))
        .route("/api/route-preview", get(route_preview))
        .route("/api/media/metrics", get(media_metrics))
        .route("/api/numbers", get(list_numbers).post(create_number))
        .route(
            "/api/numbers/:number",
            put(update_number).delete(delete_number),
        )
        .route(
            "/api/anti-fraud/rules",
            get(list_anti_fraud_rules).post(create_anti_fraud_rule),
        )
        .route(
            "/api/anti-fraud/rules/:id",
            put(update_anti_fraud_rule).delete(delete_anti_fraud_rule),
        )
        .route("/api/anti-fraud/config", get(list_anti_fraud_config))
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            jwt_auth,
        ));

    let app = Router::new()
        .merge(public_routes)
        .merge(protected_routes)
        .with_state(state)
        .layer(cors)
        .layer(TraceLayer::new_for_http());

    let port: u16 = env::var("API_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(8080);
    let addr: std::net::SocketAddr = ([0, 0, 0, 0], port).into();
    tracing::info!("API server listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
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
}
