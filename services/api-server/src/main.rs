//! # api-server：REST API 服务
//!
//! 本服务提供 VoIP 软交换平台的 RESTful API，包括：
//!
//! - **认证与授权**：JWT Token + RBAC（admin/operator/financier）
//! - **仪表盘**：实时统计、趋势图表
//! - **CDR 管理**：通话详单查询、导出
//! - **用户管理**：SIP 用户 CRUD
//! - **网关管理**：网关配置、健康状态
//! - **路由管理**：路由规则 CRUD、试算
//! - **计费管理**：费率、账户、账本、对账
//! - **录音管理**：录音列表、播放、下载
//! - **号码管理**：号码库存 CRUD
//! - **反欺诈**：规则配置
//! - **Prometheus 指标**：/metrics 端点
//!
//! ## API 端点
//!
//! | 路径 | 方法 | 说明 | 权限 |
//! |------|------|------|------|
//! | `/api/auth/login` | POST | 登录 | 公开 |
//! | `/health` | GET | 健康检查 | 公开 |
//! | `/metrics` | GET | Prometheus 指标 | 公开 |
//! | `/api/dashboard/stats` | GET | 仪表盘统计 | operator/admin |
//! | `/api/cdrs` | GET | CDR 列表 | 所有角色 |
//! | `/api/users` | CRUD | 用户管理 | admin |
//! | `/api/gateways` | CRUD | 网关管理 | admin/operator |
//! | `/api/routes` | CRUD | 路由管理 | admin/operator |
//! | `/api/rates` | CRUD | 费率管理 | admin/financier |
//! | `/api/accounts` | GET | 账户列表 | admin/financier |
//! | `/api/recordings` | GET | 录音列表 | 所有角色 |
//! | `/api/numbers` | CRUD | 号码管理 | admin/operator |
//! | `/api/anti-fraud/rules` | CRUD | 反欺诈规则 | admin/operator |

mod billing;
mod calls;
mod metrics;
mod numbers;
mod recording;
mod report;

use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse,
    },
    routing::{get, post, put},
    Json, Router,
};
use futures::stream::{self, Stream};
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
    AntiFraudRule, AuditLogInput, CdrEvent, DashboardStats, DtmfEventRecord, HourlyTrend,
    PostgresCdrStore, SipGateway, SipRegistration, SipRoute, SipUser,
};
use metrics::{MediaMetricsSnapshot, Metrics};
use numbers::{create_number, delete_number, list_numbers, update_number};
use recording::{get_recording_audio, list_recordings};
use report::{export_cdrs_csv, get_report_summary};

use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use uuid::Uuid;

/// JWT 声明：包含用户身份和权限信息。
///
/// 用于认证中间件验证请求身份。
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Claims {
    /// 用户名（subject）
    pub sub: String,
    /// 角色（admin/operator/financier）
    pub role: String,
    /// 过期时间戳（Unix 秒）
    pub exp: usize,
}

/// 登录请求
#[derive(Debug, Deserialize)]
struct LoginRequest {
    username: String,
    password: String,
}

/// 登录响应
#[derive(Debug, Serialize)]
struct LoginResponse {
    /// JWT Token
    token: String,
    /// 用户名
    username: String,
    /// 角色
    role: String,
}

/// 应用状态：所有处理器共享的状态。
///
/// 包含数据库连接、存储后端、NATS 客户端和 JWT 密钥。
#[derive(Clone)]
pub(crate) struct AppState {
    /// 数据库存储
    store: Arc<PostgresCdrStore>,
    /// 录音存储后端
    recording_storage: Arc<dyn storage_core::StorageBackend>,
    /// sip-edge 管理 API 地址
    sip_manage_base: String,
    /// 调用 sip-edge 管理 API 的共享 HTTP 客户端，统一连接池和超时。
    internal_client: reqwest::Client,
    /// NATS 客户端（用于路由热加载广播）
    nats_client: Option<async_nats::Client>,
    /// JWT 签名密钥
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
        let token = env::var("VOS_RS_INTERNAL_SECRET");
        let request = state.internal_client.get(&url);
        let request = match token {
            Ok(token) if !token.is_empty() => request.header("X-VOS-Token", token),
            _ => {
                return state
                    .store
                    .get_dashboard_stats(0)
                    .await
                    .map(Json)
                    .map_err(|e| ApiError {
                        error: e.to_string(),
                    })
            }
        };
        match request.send().await {
            Ok(resp) => resp
                .json::<Vec<serde_json::Value>>()
                .await
                .map(|calls| calls.len() as i64)
                .unwrap_or(0),
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

#[derive(Debug, Deserialize)]
struct UpdateAntiFraudConfigRequest {
    config_value: String,
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

async fn list_anti_fraud_config(
    State(state): State<AppState>,
) -> Result<Json<Vec<cdr_core::AntiFraudConfigItem>>, ApiError> {
    state
        .store
        .list_anti_fraud_configs()
        .await
        .map(Json)
        .map_err(|e| ApiError {
            error: e.to_string(),
        })
}

#[derive(Debug, Deserialize)]
struct AuditLogQuery {
    page: Option<i64>,
    page_size: Option<i64>,
}

/// 查询管理 API 审计日志，仅管理员可访问。
async fn list_audit_logs(
    State(state): State<AppState>,
    Query(query): Query<AuditLogQuery>,
) -> Result<Json<Vec<cdr_core::AuditLog>>, ApiError> {
    let page = query.page.unwrap_or(1).max(1);
    let page_size = query.page_size.unwrap_or(50).clamp(1, 200);
    state
        .store
        .list_audit_logs(page_size, (page - 1) * page_size)
        .await
        .map(Json)
        .map_err(|e| ApiError {
            error: e.to_string(),
        })
}

async fn update_anti_fraud_config(
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
        Ok(StatusCode::OK)
    } else {
        Err(ApiError::internal("防盗打配置项不存在"))
    }
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
    match state
        .internal_client
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

/// 用户登录：验证凭据并返回 JWT Token。
///
/// 支持三种角色：
/// - `admin`：管理员，可访问所有端点
/// - `operator`：运维，可访问运维和只读端点
/// - `financier`：财务，可访问计费和只读端点
///
/// 密码通过环境变量配置（`VOS_RS_ADMIN_PASSWORD` 等），
/// 未配置时返回错误（生产环境禁止默认密码）。
///
/// 成功返回 24 小时有效期的 JWT Token。
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

/// RBAC 权限检查：根据角色、HTTP 方法和路径判断是否允许访问。
///
/// 角色权限矩阵：
/// - **admin**：所有端点
/// - **operator**：运维端点（网关、路由、号码、反欺诈）+ 只读端点
/// - **financier**：计费端点（费率、账户、账本）+ 只读端点
///
/// 安全规则：
/// - `/api/users` 始终仅限 admin（SIP 用户凭证安全敏感）
/// - 只读端点（CDR、录音、仪表盘）对所有角色开放
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

/// JWT 认证中间件：验证请求中的 Bearer Token 并检查 RBAC 权限。
///
/// 流程：
/// 1. 从 Authorization 头提取 Bearer Token
/// 2. 验证 Token 签名和有效期
/// 3. 检查角色是否有权访问目标端点
/// 4. 将 Claims 注入请求扩展，供后续处理器使用
///
/// 错误响应：
/// - 401：缺少凭证、Token 无效或过期
/// - 403：角色无权访问
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
///
/// 审计日志用于追踪操作，不应成为凭据泄露的新渠道。无法解析为 JSON
/// 时不记录原文，只保留固定提示，避免把表单或未知格式直接落库。
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
                if matches!(
                    key.to_ascii_lowercase().as_str(),
                    "password"
                        | "passwd"
                        | "secret"
                        | "token"
                        | "access_token"
                        | "refresh_token"
                        | "api_key"
                        | "authorization"
                        | "ha1"
                ) {
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
        let input = AuditLogInput {
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

async fn dashboard_events(
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>> {
    let state_clone = state.clone();
    let stream = stream::unfold(
        (
            state_clone,
            tokio::time::interval(std::time::Duration::from_secs(2)),
        ),
        |(state, mut interval)| async move {
            interval.tick().await;

            let token = env::var("VOS_RS_INTERNAL_SECRET").ok();
            let active_calls = match token {
                Some(token) if !token.is_empty() => match state
                    .internal_client
                    .get(format!("{}/manage/active-calls", state.sip_manage_base))
                    .header("X-VOS-Token", token)
                    .send()
                    .await
                {
                    Ok(resp) => resp
                        .json::<Vec<serde_json::Value>>()
                        .await
                        .map(|v| v.len() as u32)
                        .unwrap_or(0),
                    Err(_) => 0,
                },
                _ => 0,
            };

            let trunk_online_count = match state.store.list_gateways_full().await {
                Ok(gateways) => gateways
                    .iter()
                    .filter(|gateway| gateway.enabled != Some(false))
                    .filter(|gateway| gateway.circuit_state.as_deref() != Some("open"))
                    .count() as u32,
                Err(_) => 0,
            };

            let data = serde_json::json!({
                "active_calls": active_calls,
                "trunk_online_count": trunk_online_count,
                "timestamp": time::OffsetDateTime::now_utc().unix_timestamp(),
            });

            let event = Event::default().data(data.to_string());
            Some((Ok(event), (state, interval)))
        },
    );

    Sse::new(stream).keep_alive(KeepAlive::default())
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
    let internal_client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(1))
        .timeout(std::time::Duration::from_secs(3))
        .build()?;

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
        internal_client,
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
    use super::{redact_sensitive_json_value, role_allows, sanitize_audit_json};
    use serde_json::json;

    // ===== Unit tests for role_allows =====

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

    #[test]
    fn operator_can_read_cdrs_and_recordings() {
        assert!(role_allows("operator", "GET", "/api/cdrs"));
        assert!(role_allows("operator", "GET", "/api/recordings"));
        assert!(role_allows("operator", "GET", "/api/dashboard/stats"));
        assert!(role_allows("operator", "GET", "/api/registrations"));
    }

    #[test]
    fn operator_can_access_operations_endpoints() {
        assert!(role_allows("operator", "POST", "/api/routes"));
        assert!(role_allows("operator", "POST", "/api/gateways"));
        assert!(role_allows("operator", "POST", "/api/numbers"));
        assert!(role_allows("operator", "POST", "/api/anti-fraud/rules"));
        assert!(role_allows("operator", "POST", "/api/calls/id/terminate"));
    }

    #[test]
    fn operator_cannot_access_finance_endpoints() {
        assert!(!role_allows("operator", "GET", "/api/accounts"));
        assert!(!role_allows("operator", "POST", "/api/rates"));
        assert!(!role_allows("operator", "POST", "/api/billing/reconcile"));
        assert!(!role_allows("operator", "GET", "/api/ledger"));
    }

    #[test]
    fn operator_cannot_access_user_credentials() {
        assert!(!role_allows("operator", "GET", "/api/users"));
        assert!(!role_allows("operator", "POST", "/api/users"));
        assert!(!role_allows("operator", "PUT", "/api/users/alice"));
        assert!(!role_allows("operator", "DELETE", "/api/users/alice"));
    }

    #[test]
    fn financier_can_access_billing_endpoints() {
        assert!(role_allows("financier", "GET", "/api/cdrs"));
        assert!(role_allows("financier", "GET", "/api/rates"));
        assert!(role_allows("financier", "POST", "/api/rates"));
        assert!(role_allows("financier", "GET", "/api/accounts"));
        assert!(role_allows("financier", "POST", "/api/billing/reconcile"));
        assert!(role_allows("financier", "GET", "/api/ledger"));
    }

    #[test]
    fn financier_cannot_access_operations_endpoints() {
        assert!(!role_allows("financier", "POST", "/api/routes"));
        assert!(!role_allows("financier", "POST", "/api/gateways"));
        assert!(!role_allows("financier", "POST", "/api/numbers"));
        assert!(!role_allows("financier", "POST", "/api/anti-fraud/rules"));
    }

    #[test]
    fn financier_cannot_access_user_credentials() {
        assert!(!role_allows("financier", "GET", "/api/users"));
        assert!(!role_allows("financier", "POST", "/api/users"));
    }

    #[test]
    fn no_role_cannot_access_anything() {
        assert!(!role_allows("unknown", "GET", "/api/cdrs"));
        assert!(!role_allows("unknown", "POST", "/api/routes"));
        assert!(!role_allows("unknown", "GET", "/api/accounts"));
        assert!(!role_allows("", "GET", "/api/cdrs"));
    }

    #[test]
    fn admin_can_access_all_read_only_endpoints() {
        assert!(role_allows("admin", "GET", "/api/cdrs"));
        assert!(role_allows("admin", "GET", "/api/recordings"));
        assert!(role_allows("admin", "GET", "/api/dashboard/stats"));
        assert!(role_allows("admin", "GET", "/api/registrations"));
        assert!(role_allows("admin", "GET", "/api/media/metrics"));
        assert!(role_allows("admin", "GET", "/api/route-preview"));
    }

    #[test]
    fn admin_can_access_all_write_endpoints() {
        assert!(role_allows("admin", "POST", "/api/routes"));
        assert!(role_allows("admin", "POST", "/api/gateways"));
        assert!(role_allows("admin", "POST", "/api/users"));
        assert!(role_allows("admin", "POST", "/api/rates"));
        assert!(role_allows("admin", "POST", "/api/billing/reconcile"));
        assert!(role_allows("admin", "POST", "/api/anti-fraud/rules"));
        assert!(role_allows("admin", "DELETE", "/api/routes/r1"));
        assert!(role_allows("admin", "DELETE", "/api/gateways/gw1"));
        assert!(role_allows("admin", "DELETE", "/api/users/alice"));
    }

    #[test]
    fn operator_can_access_route_preview_and_media_metrics() {
        assert!(role_allows("operator", "GET", "/api/route-preview"));
        assert!(role_allows("operator", "GET", "/api/media/metrics"));
    }

    #[test]
    fn financier_can_read_cdrs_and_registrations() {
        assert!(role_allows("financier", "GET", "/api/cdrs"));
        assert!(role_allows("financier", "GET", "/api/registrations"));
        assert!(role_allows("financier", "GET", "/api/recordings"));
    }

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
}
