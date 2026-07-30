//! # api-server：REST API 服务
//!
//! 本服务提供 VoIP 软交换平台的 RESTful API。
//!

// Copilot 工具 schema 使用 json! 宏嵌套较深，需提升递归上限以支持 40+ 工具定义
#![recursion_limit = "512"]

// 子目录模块
mod access_control;
mod announcements;
mod billing;
mod cluster;
mod copilot;
mod resources;
mod system;
mod termination;

// 顶层模块
mod config;
mod dashboard;
mod details;
mod error;
mod helpers;
mod import;
mod llm_configs;
mod middleware;
mod notifications;
mod recording;
mod rwi_ws;
mod v1;

// 重导出迁移后的公共 API，保持 `crate::ApiError` 等旧路径继续可用。
pub(crate) use error::ApiError;
pub(crate) use helpers::{
    config_logging_filter, normalize_page, parse_dt, validate_runtime_secrets,
};
pub(crate) use middleware::{audit_log, jwt_auth};

use axum::{
    http::HeaderValue,
    routing::{get, post, put},
    Router,
};
use serde::{Deserialize, Serialize};
use std::env;
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use billing::operations::list_ledger;
use billing::report::{export_cdrs_csv, get_report_summary};
use cdr_core::PostgresCdrStore;
use cluster::calls::{
    list_active, media_metrics, route_preview, terminate_call as calls_terminate,
};
use cluster::media_cluster::{get_media_cluster, update_media_cluster};
use recording::get_recording_audio;
use resources::numbers::{create_number, delete_number, list_numbers, update_number};

use billing::anti_fraud::{
    create_anti_fraud_rule, delete_anti_fraud_rule, list_anti_fraud_config, list_anti_fraud_rules,
    update_anti_fraud_config, update_anti_fraud_rule,
};
use billing::cdr::{get_cdr, get_dtmf_events, list_cdrs};
use cluster::sip_cluster::{control_sip_cluster_node, get_sip_cluster_status};
use dashboard::{dashboard_events, get_dashboard_stats, get_dashboard_trend};
use resources::gateways::{create_gateway, delete_gateway, list_gateways, update_gateway};
use resources::registrations::list_registrations;
use resources::routes::{create_route, delete_route, list_routes, update_route};
use resources::users::{create_user, delete_user, list_users, update_user};
use system::audit::list_audit_logs;
use system::auth::login;
use system::system::{
    get_system_configs, health, openapi_spec, prometheus_metrics, ready, update_system_configs,
};

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
    pub(crate) access_snapshot: Arc<tokio::sync::RwLock<cdr_core::AccessSnapshot>>,
    pub(crate) internal_secret: String,
    pub(crate) redis_client: redis::aio::ConnectionManager,
    pub(crate) sip_node_key_prefix: String,
    pub(crate) sip_auth_realm: String,
    /// 活跃通话列表缓存（2s TTL），消除 summary/extras/telemetry 重复 HTTP 请求
    pub(crate) active_calls_cache: crate::dashboard::ActiveCallsCache,
    /// LLM 专用 HTTP 客户端（长超时，与 sip-edge 内部管理调用的 internal_client 分离）。
    /// LLM 配置（api_key/base_url/model）运行时从数据库 llm_configs 表读取，无需重启即可切换。
    pub(crate) llm_client: reqwest::Client,
}

/// 管理列表统一分页参数；服务端限制单页最大 100 条，避免大响应拖慢 API。
///
/// `page` / `page_size` / `export` 使用自定义反序列化器以兼容
/// `#[serde(flatten)]` 场景下 `serde_urlencoded` 传入的字符串值——
/// 否则会出现 `invalid type: string "1", expected i64` 错误。
#[derive(Debug, Deserialize)]
pub(crate) struct PageQuery {
    #[serde(default, deserialize_with = "deserialize_optional_i64_from_str")]
    pub page: Option<i64>,
    #[serde(default, deserialize_with = "deserialize_optional_i64_from_str")]
    pub page_size: Option<i64>,
    #[serde(default)]
    pub gateway_type: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_bool_from_str")]
    pub export: Option<bool>,
}

/// 从查询字符串中反序列化 `Option<i64>`，兼容字符串和数字两种形式。
///
/// `serde_urlencoded` 在 `#[serde(flatten)]` 场景下会以字符串形式
/// 传递查询参数，默认 `i64` 反序列化器无法接受字符串，因此需要
/// 自定义反序列化器先尝试字符串再尝试数字。
pub(crate) fn deserialize_optional_i64_from_str<'de, D>(
    deserializer: D,
) -> Result<Option<i64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct OptionalI64Visitor;

    impl<'de> serde::de::Visitor<'de> for OptionalI64Visitor {
        type Value = Option<i64>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("可选的整数或整数字符串")
        }

        fn visit_unit<E>(self) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Ok(None)
        }

        fn visit_none<E>(self) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Ok(None)
        }

        fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            struct I64FromString;

            impl<'de> serde::de::Visitor<'de> for I64FromString {
                type Value = i64;

                fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                    formatter.write_str("整数或整数字符串")
                }

                fn visit_str<E>(self, s: &str) -> Result<Self::Value, E>
                where
                    E: serde::de::Error,
                {
                    s.parse::<i64>()
                        .map_err(|_| E::custom(format!("无法将 '{s}' 解析为整数")))
                }

                fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E>
                where
                    E: serde::de::Error,
                {
                    Ok(v)
                }

                fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E>
                where
                    E: serde::de::Error,
                {
                    i64::try_from(v).map_err(|_| E::custom("数字超出 i64 范围"))
                }
            }

            let value = deserializer.deserialize_any(I64FromString)?;
            Ok(Some(value))
        }
    }

    deserializer.deserialize_option(OptionalI64Visitor)
}

/// 从查询字符串中反序列化 `Option<bool>`，兼容 "true"/"false" 字符串。
pub(crate) fn deserialize_optional_bool_from_str<'de, D>(
    deserializer: D,
) -> Result<Option<bool>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct OptionalBoolVisitor;

    impl<'de> serde::de::Visitor<'de> for OptionalBoolVisitor {
        type Value = Option<bool>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("可选的布尔值或布尔字符串")
        }

        fn visit_unit<E>(self) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Ok(None)
        }

        fn visit_none<E>(self) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Ok(None)
        }

        fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            struct BoolFromString;

            impl<'de> serde::de::Visitor<'de> for BoolFromString {
                type Value = bool;

                fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                    formatter.write_str("布尔值或布尔字符串")
                }

                fn visit_str<E>(self, s: &str) -> Result<Self::Value, E>
                where
                    E: serde::de::Error,
                {
                    match s.to_ascii_lowercase().as_str() {
                        "true" | "1" => Ok(true),
                        "false" | "0" | "" => Ok(false),
                        _ => Err(E::custom(format!("无法将 '{s}' 解析为布尔值"))),
                    }
                }

                fn visit_bool<E>(self, v: bool) -> Result<Self::Value, E>
                where
                    E: serde::de::Error,
                {
                    Ok(v)
                }
            }

            let value = deserializer.deserialize_any(BoolFromString)?;
            Ok(Some(value))
        }
    }

    deserializer.deserialize_option(OptionalBoolVisitor)
}

#[derive(Debug, Serialize)]
pub(crate) struct PaginatedResponse<T> {
    pub(crate) items: Vec<T>,
    pub(crate) total: i64,
    pub(crate) page: i64,
    pub(crate) page_size: i64,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let logging_filter = config_logging_filter("api_server=info,tower_http=info");
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(logging_filter))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let config = config::load_config()?;
    let conn_section = config.connections.unwrap_or_default();
    let db_section = conn_section.database.unwrap_or_default();
    let api_section = config.api_server.unwrap_or_default();
    let api_network = api_section.network.unwrap_or_default();
    let api_security = api_section.security.unwrap_or_default();

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
    let mut storage_config = storage_core::StorageConfig::from_env();
    if let Some(backend) = store
        .get_system_config("storage_backend")
        .await?
        .filter(|value| !value.trim().is_empty())
    {
        storage_config.backend = backend
            .parse()
            .map_err(|error: String| anyhow::anyhow!(error))?;
    }
    let recording_local_dir = store
        .get_system_config("recording_dir")
        .await?
        .filter(|value| !value.trim().is_empty())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| storage_config.local_dir.clone().into());
    storage_config.local_dir = recording_local_dir.to_string_lossy().into_owned();
    let recording_storage: Arc<dyn storage_core::StorageBackend> =
        storage_core::create_storage(&storage_config).await?.into();
    if recording_storage.backend_name() != "local" {
        let storage = Arc::clone(&recording_storage);
        let recording_dir = recording_local_dir.clone();
        tokio::spawn(async move {
            let mut uploaded_sizes = std::collections::HashMap::new();
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(15));
            loop {
                interval.tick().await;
                let uploaded = crate::recording::sync_local_recordings(
                    storage.as_ref(),
                    &recording_dir,
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
    // 仅在集群模式启用且 management_url 非空时使用集群管理地址；
    // 单节点模式（enabled=false）应回退到 sip_edge.network.manage_bind，
    // 避免 management_url 误配死端口导致 /calls/active 等转发接口 502。
    let cluster_enabled = cluster_section.enabled.unwrap_or(false);
    let sip_manage_base = if cluster_enabled {
        cluster_section
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
            })
    } else {
        format!(
            "http://{}",
            sip_edge_section
                .network
                .unwrap_or_default()
                .manage_bind
                .unwrap_or_else(|| "127.0.0.1:8082".to_string())
        )
    };

    let internal_client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(2))
        .timeout(std::time::Duration::from_secs(5))
        .build()?;

    // LLM 专用 HTTP 客户端：大模型推理响应较慢，超时设为 90s，支持环境变量/系统代理与自适应 DNS。
    let llm_client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(15))
        .timeout(std::time::Duration::from_secs(90))
        .user_agent("vos-rs/1.0")
        .no_gzip()
        .no_deflate()
        .no_brotli()
        .build()?;

    let jwt_secret = env::var("VOS_RS_API_JWT_SECRET")
        .ok()
        .or(api_security.jwt_secret.clone())
        .unwrap_or_else(|| "vos-rs-secret-key-change-in-production".to_string());

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
    let addr: std::net::SocketAddr = addr_str.parse().or_else(|_| {
        use std::net::ToSocketAddrs;
        addr_str
            .to_socket_addrs()
            .ok()
            .and_then(|mut addrs| addrs.next())
            .ok_or_else(|| anyhow::anyhow!("无法解析 API 服务器绑定地址: {}", addr_str))
    })?;

    let is_public = !addr.ip().is_loopback();
    let production = env::var("VOS_RS_ENV")
        .is_ok_and(|value| value.eq_ignore_ascii_case("production"))
        || is_public;

    validate_runtime_secrets(production, &jwt_secret, &internal_secret)?;
    let bootstrap_password = env::var("VOS_RS_BOOTSTRAP_ADMIN_PASSWORD").ok();
    let access_snapshot = access_control::initialize_access_control(
        &store,
        bootstrap_password.as_deref(),
        production,
    )
    .await?;

    let state = AppState {
        store: Arc::new(store),
        recording_storage,
        recording_local_dir,
        sip_manage_base,
        internal_client,
        nats_client,
        jwt_secret: jwt_secret.into_bytes(),
        access_snapshot: Arc::new(tokio::sync::RwLock::new(access_snapshot)),
        internal_secret,
        redis_client,
        sip_node_key_prefix,
        sip_auth_realm,
        active_calls_cache: crate::dashboard::ActiveCallsCache::new(
            std::time::Duration::from_secs(2),
        ),
        llm_client,
    };

    // 启动时从数据库重建 Redis 缓存中的所有 LLM 配置
    if let Err(e) = crate::llm_configs::rebuild_llm_configs_in_redis(&state).await {
        tracing::error!("启动时重建 Redis LLM 配置缓存失败: {:?}", e);
    }

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
        .route("/api/v1/openapi.json", get(openapi_spec))
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
        .route("/api/users/import", post(import::import_users))
        .route(
            "/api/users/import-template",
            get(import::import_users_template),
        )
        .route("/api/gateways", get(list_gateways).post(create_gateway))
        .route(
            "/api/gateways/:id",
            put(update_gateway).delete(delete_gateway),
        )
        .route("/api/routes", get(list_routes).post(create_route))
        .route("/api/routes/:id", put(update_route).delete(delete_route))
        .route("/api/routes/import", post(import::import_routes))
        .route(
            "/api/routes/import-template",
            get(import::import_routes_template),
        )
        .route("/api/registrations", get(list_registrations))
        .route("/api/recordings/:call_id/audio", get(get_recording_audio))
        .route("/api/reports/summary", get(get_report_summary))
        .route("/api/reports/export", get(export_cdrs_csv))
        .route("/api/ledger", get(list_ledger))
        .route("/api/calls/active", get(list_active))
        .route("/api/calls/:call_id/terminate", post(calls_terminate))
        .route("/api/route-preview", get(route_preview))
        .route("/api/media/metrics", get(media_metrics))
        .route("/api/numbers", get(list_numbers).post(create_number))
        .route(
            "/api/numbers/:number",
            put(update_number).delete(delete_number),
        )
        .route("/api/numbers/import", post(import::import_numbers))
        .route(
            "/api/numbers/import-template",
            get(import::import_numbers_template),
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
        .route("/rwi/v1/ws", get(rwi_ws::rwi_ws_handler))
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            audit_log,
        ))
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            jwt_auth,
        ))
        .route_layer(axum::middleware::from_fn(v1::response_contract));

    let app = Router::new()
        .merge(public_routes)
        .merge(protected_routes)
        .merge(v1_public_routes)
        .merge(v1_protected_routes)
        .with_state(state.clone())
        .layer(cors)
        .layer(TraceLayer::new_for_http());

    // Spawn background traffic telemetry loop to periodically report node traffic to Redis
    tokio::spawn(crate::dashboard::start_traffic_telemetry_loop(
        state.clone(),
    ));
    tokio::spawn(crate::notifications::start_notification_scan_loop(
        state.clone(),
    ));

    tracing::info!("API server listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
