//! # api-server：REST API 服务
//!
//! 本服务提供 VoIP 软交换平台的 RESTful API。
//!

mod anti_fraud;
mod audit;
mod auth;
mod billing;
mod call_center;
mod calls;
mod cdr;
mod copilot;
mod dashboard;
mod details;
mod gateways;
mod hot_cache;
mod ivr_menus;
mod media_cluster;
mod metrics;
mod numbers;
mod recording;
mod registrations;
mod report;
mod routes;
mod sip_cluster;
mod system;
mod termination;
mod users;
mod v1;

use axum::{
    extract::State,
    http::{HeaderValue, StatusCode},
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
use cdr_core::PostgresCdrStore;
use media_cluster::{get_media_cluster, update_media_cluster};
use numbers::{create_number, delete_number, list_numbers, update_number};
use recording::get_recording_audio;
use report::{export_cdrs_csv, get_report_summary};

use jsonwebtoken::{decode, DecodingKey, Validation};
use uuid::Uuid;

use anti_fraud::{
    create_anti_fraud_rule, delete_anti_fraud_rule, list_anti_fraud_config, list_anti_fraud_rules,
    update_anti_fraud_config, update_anti_fraud_rule,
};
use audit::list_audit_logs;
use auth::{login, role_allows, Claims};
use cdr::{get_cdr, get_dtmf_events, list_cdrs};
use dashboard::{dashboard_events, get_dashboard_stats, get_dashboard_trend};
use gateways::{create_gateway, delete_gateway, list_gateways, update_gateway};
use registrations::list_registrations;
use routes::{create_route, delete_route, list_routes, update_route};
use sip_cluster::{control_sip_cluster_node, get_sip_cluster_status};
use system::{get_system_configs, health, prometheus_metrics, ready, update_system_configs};
use users::{create_user, delete_user, list_users, update_user};

/// 应用状态：所有处理器共享的状态。
#[derive(Clone)]
pub(crate) struct AppState {
    pub(crate) store: Arc<PostgresCdrStore>,
    pub(crate) recording_storage: Arc<dyn storage_core::StorageBackend>,
    pub(crate) recording_local_dir: std::path::PathBuf,
    pub(crate) sip_manage_base: String,
    pub(crate) internal_client: reqwest::Client,
    pub(crate) nats_client: Option<async_nats::Client>,
    pub(crate) jwt_secret: Vec<u8>,
    pub(crate) admin_password: String,
    pub(crate) operator_password: String,
    pub(crate) financier_password: String,
    pub(crate) internal_secret: String,
    pub(crate) redis_client: redis::aio::ConnectionManager,
    pub(crate) sip_node_key_prefix: String,
    pub(crate) sip_auth_realm: String,
}

/// 管理列表统一分页参数；服务端限制单页最大 100 条，避免大响应拖慢 API。
#[derive(Debug, Deserialize)]
pub(crate) struct PageQuery {
    pub page: Option<i64>,
    pub page_size: Option<i64>,
    pub gateway_type: Option<String>,
    pub role: Option<String>,
}

pub(crate) fn normalize_page(query: &PageQuery) -> (i64, i64, i64) {
    let page = query.page.unwrap_or(1).max(1);
    let page_size = query.page_size.unwrap_or(20).clamp(1, 100);
    let offset = (page - 1).saturating_mul(page_size);
    (page, page_size, offset)
}

#[derive(Debug, Serialize)]
pub(crate) struct PaginatedResponse<T> {
    pub(crate) items: Vec<T>,
    pub(crate) total: i64,
    pub(crate) page: i64,
    pub(crate) page_size: i64,
}

#[derive(Debug, Serialize)]
pub(crate) struct ApiError {
    pub(crate) error: String,
}

impl ApiError {
    pub(crate) fn internal(msg: impl Into<String>) -> Self {
        Self { error: msg.into() }
    }

    pub(crate) fn unauthorized(msg: impl Into<String>) -> Self {
        Self { error: msg.into() }
    }

    pub(crate) fn forbidden(msg: impl Into<String>) -> Self {
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
        } else if self.error.contains("参数无效") {
            StatusCode::BAD_REQUEST
        } else if self.error.contains("不存在") {
            StatusCode::NOT_FOUND
        } else {
            StatusCode::INTERNAL_SERVER_ERROR
        };
        (status, Json(self)).into_response()
    }
}

pub(crate) fn parse_dt(s: &str) -> Option<OffsetDateTime> {
    OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339).ok()
}

/// JWT 认证中间件：验证请求中的 Bearer Token 并检查 RBAC 权限。
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

/// 对审计请求体中的敏感字段做递归脱敏。
fn sanitize_audit_json(body: &[u8]) -> String {
    let Ok(mut value) = serde_json::from_slice::<serde_json::Value>(body) else {
        return "[已省略无法解析的 JSON 请求体]".to_string();
    };
    redact_sensitive_json_value(&mut value);
    serde_json::to_string(&value).unwrap_or_else(|_| "[已省略审计请求体]".to_string())
}

/// 递归遍历 JSON 对象和数组，将常见凭据字段替换成固定占位符。
fn redact_sensitive_json_value(value: &mut serde_json::Value) {
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

async fn audit_log(
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
        const MAX_AUDIT_BODY_BYTES: usize = 256 * 1024;
        let content_type = parts
            .headers
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let body_result = axum::body::to_bytes(body, MAX_AUDIT_BODY_BYTES).await;
        let body_bytes = match body_result {
            Ok(bytes) => bytes,
            Err(error) => {
                tracing::warn!(%error, request_id = %request_id, "读取审计请求体失败，跳过请求体记录");
                axum::body::Bytes::new()
            }
        };
        let body_str = if content_type.starts_with("application/json") {
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

fn validate_runtime_secrets(
    production: bool,
    jwt_secret: &str,
    internal_secret: &str,
    admin_password: &str,
    operator_password: &str,
    financier_password: &str,
) -> anyhow::Result<()> {
    validate_runtime_secrets_for_environment(
        production,
        jwt_secret,
        internal_secret,
        admin_password,
        operator_password,
        financier_password,
    )
}

fn validate_runtime_secrets_for_environment(
    production: bool,
    jwt_secret: &str,
    internal_secret: &str,
    admin_password: &str,
    operator_password: &str,
    financier_password: &str,
) -> anyhow::Result<()> {
    if !production {
        return Ok(());
    }
    if jwt_secret.len() < 32 || jwt_secret.contains("change-in-production") {
        anyhow::bail!("生产环境 VOS_RS_API_JWT_SECRET 必须是至少 32 字符的随机密钥");
    }
    if internal_secret.len() < 24
        || matches!(
            internal_secret,
            "internal-dev-secret" | "compose-internal-secret"
        )
    {
        anyhow::bail!("生产环境 VOS_RS_INTERNAL_SECRET 必须是至少 24 字符的随机密钥");
    }
    for (name, value, default) in [
        ("VOS_RS_ADMIN_PASSWORD", admin_password, "admin"),
        ("VOS_RS_OPERATOR_PASSWORD", operator_password, "operator"),
        ("VOS_RS_FINANCIER_PASSWORD", financier_password, "financier"),
    ] {
        if value.len() < 12 || value == default || value.ends_with("-change-me") {
            anyhow::bail!("生产环境 {name} 必须是至少 12 字符的非默认密码");
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let logging_filter = config_logging_filter("api_server=info,tower_http=info");
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(logging_filter))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let config_file_path =
        env::var("VOS_RS_CONFIG_FILE").unwrap_or_else(|_| "config.yaml".to_string());
    let config_content = std::fs::read_to_string(&config_file_path)
        .map_err(|error| anyhow::anyhow!("读取配置文件 {config_file_path} 失败: {error}"))?;

    #[derive(serde::Deserialize, Debug, Default)]
    struct ApiServerConfig {
        connections: Option<ConnectionsSection>,
        api_server: Option<ApiServerSection>,
        sip_edge: Option<SipEdgeConfigSection>,
    }

    #[derive(serde::Deserialize, Debug, Default, Clone)]
    struct ConnectionsSection {
        database: Option<DatabaseSection>,
        redis: Option<RedisSection>,
        nats: Option<NatsSection>,
    }

    #[derive(serde::Deserialize, Debug, Default, Clone)]
    struct RedisSection {
        host: Option<String>,
        port: Option<u16>,
        password: Option<String>,
        database: Option<u16>,
    }

    #[derive(serde::Deserialize, Debug, Default, Clone)]
    struct DatabaseSection {
        host: Option<String>,
        port: Option<u16>,
        username: Option<String>,
        password: Option<String>,
        database: Option<String>,
        max_connections: Option<u32>,
    }

    #[derive(serde::Deserialize, Debug, Default, Clone)]
    struct NatsSection {
        url: Option<String>,
    }

    #[derive(serde::Deserialize, Debug, Default)]
    struct ApiServerSection {
        network: Option<ApiNetworkSection>,
        security: Option<ApiSecuritySection>,
        admin_credentials: Option<AdminCredentialsSection>,
    }

    #[derive(serde::Deserialize, Debug, Default)]
    struct ApiNetworkSection {
        host: Option<String>,
        port: Option<u16>,
        allowed_origins: Option<String>,
    }
    #[derive(serde::Deserialize, Debug, Default)]
    struct ApiSecuritySection {
        jwt_secret: Option<String>,
        internal_secret: Option<String>,
    }
    #[derive(serde::Deserialize, Debug, Default)]
    struct AdminCredentialsSection {
        admin_password: Option<String>,
        operator_password: Option<String>,
        financier_password: Option<String>,
    }

    #[derive(serde::Deserialize, Debug, Default)]
    struct SipEdgeConfigSection {
        network: Option<SipEdgeNetworkSection>,
        cluster: Option<SipEdgeClusterSection>,
        auth: Option<SipEdgeAuthSection>,
    }
    #[derive(serde::Deserialize, Debug, Default)]
    struct SipEdgeNetworkSection {
        manage_bind: Option<String>,
    }
    #[derive(serde::Deserialize, Debug, Default)]
    struct SipEdgeClusterSection {
        node_key_prefix: Option<String>,
        management_url: Option<String>,
    }
    #[derive(serde::Deserialize, Debug, Default)]
    struct SipEdgeAuthSection {
        realm: Option<String>,
    }

    let config: ApiServerConfig = serde_yaml::from_str(&config_content)
        .map_err(|error| anyhow::anyhow!("解析配置文件 {config_file_path} 失败: {error}"))?;
    let conn_section = config.connections.unwrap_or_default();
    let db_section = conn_section.database.unwrap_or_default();
    let api_section = config.api_server.unwrap_or_default();
    let api_network = api_section.network.unwrap_or_default();
    let api_security = api_section.security.unwrap_or_default();
    let admin_credentials = api_section.admin_credentials.unwrap_or_default();

    let database_url = if let (Some(host), Some(port), Some(username), Some(database)) = (
        db_section.host.clone(),
        db_section.port,
        db_section.username.clone(),
        db_section.database.clone(),
    ) {
        let password = db_section.password.clone().unwrap_or_default();
        if password.is_empty() {
            format!("postgres://{}@{}:{}/{}", username, host, port, database)
        } else {
            format!(
                "postgres://{}:{}@{}:{}/{}",
                username, password, host, port, database
            )
        }
    } else {
        return Err(anyhow::anyhow!(
            "配置文件缺少完整的 connections.database 配置"
        ));
    };

    let max_connections = db_section.max_connections.unwrap_or(10);
    let store = match PostgresCdrStore::connect(&database_url, max_connections).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, "PostgreSQL 数据库连接失败，请检查连接配置。VOS-RS 必须有 PostgreSQL 运行！");
            return Err(e.into());
        }
    };

    let redis_section = conn_section.redis.clone().unwrap_or_default();
    let redis_url =
        if let (Some(host), Some(port)) = (redis_section.host.clone(), redis_section.port) {
            let password = redis_section.password.clone().unwrap_or_default();
            let db = redis_section.database.unwrap_or(0);
            if password.is_empty() {
                format!("redis://{}:{}/{}", host, port, db)
            } else {
                format!("redis://:{}@{}:{}/{}", password, host, port, db)
            }
        } else {
            "redis://127.0.0.1:6379".to_string()
        };
    let redis_client = match redis::Client::open(redis_url.clone()) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, "Redis 客户端打开失败。VOS-RS 必须有 Redis 运行！");
            return Err(e.into());
        }
    };
    let redis_client = match redis::aio::ConnectionManager::new(redis_client).await {
        Ok(conn) => conn,
        Err(e) => {
            tracing::error!(error = %e, "Redis 连接失败，请检查服务状态。VOS-RS 必须有 Redis 运行！");
            return Err(e.into());
        }
    };
    tracing::info!("Redis 存储连接成功 (必须要求)");

    // 使用数据库配置直接读取本地录音配置默认值
    let storage_config = storage_core::StorageConfig::from_env();
    let recording_storage: Arc<dyn storage_core::StorageBackend> =
        storage_core::create_storage(&storage_config).await?.into();
    if recording_storage.backend_name() != "local" {
        let storage = Arc::clone(&recording_storage);
        let recording_dir = storage_config.local_dir.clone();
        tokio::spawn(async move {
            let mut uploaded_sizes = std::collections::HashMap::new();
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(15));
            loop {
                interval.tick().await;
                let uploaded = crate::recording::sync_local_recordings(
                    storage.as_ref(),
                    std::path::Path::new(&recording_dir),
                    &mut uploaded_sizes,
                )
                .await;
                if uploaded > 0 {
                    tracing::info!(uploaded, "录音文件已归档到对象存储");
                }
            }
        });
    }

    let nats_url = conn_section
        .nats
        .clone()
        .and_then(|n| n.url)
        .filter(|u| !u.trim().is_empty())
        .unwrap_or_else(|| "nats://127.0.0.1:4222".to_string());
    let nats_client = async_nats::connect(&nats_url).await.ok();

    let sip_edge_section = config.sip_edge.unwrap_or_default();
    let sip_auth_realm = sip_edge_section
        .auth
        .as_ref()
        .and_then(|auth| auth.realm.clone())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "vos-rs".to_string());
    let cluster_section = sip_edge_section.cluster.unwrap_or_default();
    let sip_node_key_prefix = cluster_section
        .node_key_prefix
        .clone()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "vos_rs:cluster:sip_nodes".to_string());
    let sip_manage_base = cluster_section
        .management_url
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| {
            format!(
                "http://{}",
                sip_edge_section
                    .network
                    .unwrap_or_default()
                    .manage_bind
                    .unwrap_or_else(|| "127.0.0.1:8082".to_string())
            )
        });

    let internal_client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(1))
        .timeout(std::time::Duration::from_secs(3))
        .build()?;

    let jwt_secret = env::var("VOS_RS_API_JWT_SECRET")
        .ok()
        .or(api_security.jwt_secret.clone())
        .unwrap_or_else(|| "vos-rs-secret-key-change-in-production".to_string());

    let admin_password = env::var("VOS_RS_ADMIN_PASSWORD")
        .ok()
        .or(admin_credentials.admin_password)
        .unwrap_or_else(|| "admin".to_string());
    let operator_password = env::var("VOS_RS_OPERATOR_PASSWORD")
        .ok()
        .or(admin_credentials.operator_password)
        .unwrap_or_else(|| "operator".to_string());
    let financier_password = env::var("VOS_RS_FINANCIER_PASSWORD")
        .ok()
        .or(admin_credentials.financier_password)
        .unwrap_or_else(|| "financier".to_string());
    let internal_secret = env::var("VOS_RS_INTERNAL_SECRET")
        .ok()
        .or(api_security.internal_secret)
        .unwrap_or_else(|| "internal-dev-secret".to_string());
    let host = env::var("VOS_RS_API_HOST")
        .ok()
        .or_else(|| api_network.host.clone())
        .filter(|h| !h.trim().is_empty())
        .unwrap_or_else(|| "127.0.0.1".to_string());
    let port: u16 = api_network.port.unwrap_or(8080);
    let addr_str = format!("{}:{}", host, port);
    let addr: std::net::SocketAddr = addr_str
        .parse()
        .or_else(|_| {
            use std::net::ToSocketAddrs;
            addr_str.to_socket_addrs()
                .ok()
                .and_then(|mut addrs| addrs.next())
                .ok_or_else(|| anyhow::anyhow!("无法解析 API 服务器绑定地址: {}", addr_str))
        })?;

    let is_public = !addr.ip().is_loopback();
    let production = env::var("VOS_RS_ENV").is_ok_and(|value| value.eq_ignore_ascii_case("production"))
        || is_public;

    validate_runtime_secrets(
        production,
        &jwt_secret,
        &internal_secret,
        &admin_password,
        &operator_password,
        &financier_password,
    )?;

    let state = AppState {
        store: Arc::new(store),
        recording_storage,
        recording_local_dir: storage_config.local_dir.into(),
        sip_manage_base,
        internal_client,
        nats_client,
        jwt_secret: jwt_secret.into_bytes(),
        admin_password,
        operator_password,
        financier_password,
        internal_secret,
        redis_client,
        sip_node_key_prefix,
        sip_auth_realm,
    };

    let cors_origins_raw = api_network.allowed_origins.clone().unwrap_or_default();
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
        tracing::warn!("警告: 未配置 allowed_origins，默认只允许 localhost:3000 和 localhost:8080 开发域名跨域访问");
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
            axum::http::header::RANGE,
            axum::http::HeaderName::from_static("x-request-id"),
        ])
        .expose_headers([
            axum::http::HeaderName::from_static("x-request-id"),
            axum::http::header::ACCEPT_RANGES,
            axum::http::header::CONTENT_RANGE,
            axum::http::header::CONTENT_LENGTH,
        ]);

    let public_routes = Router::new()
        .route("/health", get(health))
        .route("/ready", get(ready))
        .route("/metrics", get(prometheus_metrics))
        .route("/api/auth/login", post(login));

    let v1_public_routes = v1::public_routes();
    let v1_protected_routes = v1::protected_routes(state.clone());

    let protected_routes = Router::new()
        .route(
            "/api/system/configs",
            get(get_system_configs).post(update_system_configs),
        )
        .route(
            "/api/system/media-cluster",
            get(get_media_cluster).put(update_media_cluster),
        )
        .route(
            "/api/system/sip-cluster/status",
            get(get_sip_cluster_status),
        )
        .route(
            "/api/system/sip-cluster/nodes/:node_id/:action",
            post(control_sip_cluster_node),
        )
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
        .route("/api/anti-fraud/config/:key", put(update_anti_fraud_config))
        .route("/api/audit-logs", get(list_audit_logs))
        .route("/api/dashboard/events", get(dashboard_events))
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            audit_log,
        ))
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            jwt_auth,
        ));

    let app = Router::new()
        .merge(public_routes)
        .merge(protected_routes)
        .merge(v1_public_routes)
        .merge(v1_protected_routes)
        .with_state(state.clone())
        .layer(cors)
        .layer(TraceLayer::new_for_http());

    // Spawn background traffic telemetry loop to periodically report node traffic to Redis
    tokio::spawn(crate::dashboard::start_traffic_telemetry_loop(state.clone()));

    tracing::info!("API server listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

fn config_logging_filter(default: &str) -> String {
    let path = env::var("VOS_RS_CONFIG_FILE").unwrap_or_else(|_| "config.yaml".to_string());
    std::fs::read_to_string(path)
        .ok()
        .and_then(|content| serde_yaml::from_str::<serde_yaml::Value>(&content).ok())
        .and_then(|root| {
            root.get("logging")?
                .get("filter")?
                .as_str()
                .map(str::to_owned)
        })
        .filter(|filter| !filter.trim().is_empty())
        .unwrap_or_else(|| default.to_string())
}

#[cfg(test)]
mod tests {
    use super::{
        normalize_page, redact_sensitive_json_value, sanitize_audit_json,
        validate_runtime_secrets_for_environment, PageQuery,
    };
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

    #[test]
    fn management_page_parameters_are_bounded() {
        let query = PageQuery {
            page: Some(0),
            page_size: Some(10_000),
            gateway_type: None,
            role: None,
        };

        assert_eq!(normalize_page(&query), (1, 100, 0));
    }

    #[test]
    fn production_rejects_default_runtime_secrets() {
        let error = validate_runtime_secrets_for_environment(
            true,
            "api-jwt-change-in-production",
            "internal-dev-secret",
            "admin",
            "operator",
            "financier",
        )
        .expect_err("production defaults must be rejected");

        assert!(error.to_string().contains("VOS_RS_API_JWT_SECRET"));
    }

    #[test]
    fn development_allows_local_runtime_defaults() {
        assert!(validate_runtime_secrets_for_environment(
            false,
            "development",
            "internal-dev-secret",
            "admin",
            "operator",
            "financier",
        )
        .is_ok());
    }

    #[test]
    fn public_bind_rejects_default_runtime_secrets() {
        let addr: std::net::SocketAddr = "0.0.0.0:8080".parse().unwrap();
        let is_public = !addr.ip().is_loopback();
        assert!(is_public);
        let production = is_public;
        
        let error = super::validate_runtime_secrets(
            production,
            "vos-rs-secret-key-change-in-production",
            "internal-dev-secret",
            "admin",
            "operator",
            "financier",
        );
        assert!(error.is_err());
    }
}
