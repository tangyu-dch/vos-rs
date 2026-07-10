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
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use billing::{
    create_rate, credit_account, delete_rate, list_accounts, list_ledger, list_rates,
    reconcile as billing_reconcile, update_rate,
};
use calls::{list_active, media_metrics, route_preview, terminate_call as calls_terminate};
use cdr_core::{
    CdrEvent, DashboardStats, DtmfEventRecord, HourlyTrend, PostgresCdrStore, SipGateway,
    SipRegistration, SipRoute, SipUser,
};
use metrics::{MediaMetricsSnapshot, Metrics};
use numbers::{create_number, delete_number, list_numbers, update_number};
use recording::{get_recording_audio, list_recordings};
use report::{export_cdrs_csv, get_report_summary};

#[derive(Clone)]
pub(crate) struct AppState {
    store: Arc<PostgresCdrStore>,
    recording_storage: Arc<dyn storage_core::StorageBackend>,
    sip_manage_base: String,
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
    time_start: Option<String>,
    time_end: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UpdateRouteRequest {
    prefix: String,
    priority: i32,
    gateway_id: String,
    cost: f64,
    time_start: Option<String>,
    time_end: Option<String>,
}

#[derive(Debug, Serialize)]
struct ApiError {
    error: String,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        (StatusCode::INTERNAL_SERVER_ERROR, Json(self)).into_response()
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
    state
        .store
        .insert_user(&req.username, &req.password)
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
    state
        .store
        .insert_user(&username, &req.password)
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
    state
        .store
        .insert_route_with_cost(
            &req.id,
            &req.prefix,
            req.priority,
            &req.gateway_id,
            req.cost,
            req.time_start.as_deref(),
            req.time_end.as_deref(),
        )
        .await
        .map_err(|e| ApiError {
            error: e.to_string(),
        })?;
    Ok(StatusCode::CREATED)
}

async fn update_route(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateRouteRequest>,
) -> Result<StatusCode, ApiError> {
    state
        .store
        .insert_route_with_cost(
            &id,
            &req.prefix,
            req.priority,
            &req.gateway_id,
            req.cost,
            req.time_start.as_deref(),
            req.time_end.as_deref(),
        )
        .await
        .map_err(|e| ApiError {
            error: e.to_string(),
        })?;
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

#[derive(Serialize)]
struct AntiFraudRule {
    id: i32,
    rule_type: String,
    value: String,
    description: String,
    enabled: bool,
}

#[derive(Serialize)]
struct AntiFraudConfigItem {
    key: String,
    value: String,
    description: String,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct CreateAntiFraudRuleRequest {
    rule_type: String,
    value: String,
    description: Option<String>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct UpdateAntiFraudRuleRequest {
    description: Option<String>,
    enabled: Option<bool>,
}

async fn list_anti_fraud_rules() -> Result<Json<Vec<AntiFraudRule>>, ApiError> {
    Ok(Json(Vec::new()))
}

async fn create_anti_fraud_rule(
    Json(_req): Json<CreateAntiFraudRuleRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(serde_json::json!({"ok": true})))
}

async fn update_anti_fraud_rule(
    Path(_id): Path<i32>,
    Json(_req): Json<UpdateAntiFraudRuleRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(serde_json::json!({"ok": true})))
}

async fn delete_anti_fraud_rule(Path(_id): Path<i32>) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(serde_json::json!({"ok": true})))
}

async fn list_anti_fraud_config() -> Result<Json<Vec<AntiFraudConfigItem>>, ApiError> {
    Ok(Json(Vec::new()))
}

async fn prometheus_metrics(State(state): State<AppState>) -> impl IntoResponse {
    let url = format!("{}/manage/media-metrics", state.sip_manage_base);
    match reqwest::get(&url).await {
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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "api_server=debug,tower_http=debug,info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let database_url =
        env::var("DATABASE_URL").unwrap_or_else(|_| "postgres://localhost/vos_rs".to_string());

    let store = PostgresCdrStore::connect(&database_url).await?;
    let storage_config = storage_core::StorageConfig::from_env();
    let recording_storage: Arc<dyn storage_core::StorageBackend> = Arc::new(
        storage_core::local::LocalStorage::new(&storage_config.local_dir)?,
    );
    let sip_manage_base =
        env::var("VOS_RS_MANAGE_BASE").unwrap_or_else(|_| "http://127.0.0.1:8082".to_string());

    let state = AppState {
        store: Arc::new(store),
        recording_storage,
        sip_manage_base,
    };

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = Router::new()
        .route("/health", get(health))
        .route("/metrics", get(prometheus_metrics))
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
