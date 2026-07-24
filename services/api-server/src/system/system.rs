use super::metrics::{CdrMetricsSnapshot, MediaMetricsSnapshot, Metrics};
use crate::AppState;
use axum::{
    extract::State,
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::IntoResponse,
};

pub async fn health() -> &'static str {
    "OK"
}

/// 就绪探针：进程存活不代表数据库可用，只有底层依赖检查全部通过才返回 200。
pub async fn ready(State(state): State<AppState>) -> impl IntoResponse {
    let db_status = match state.store.ping().await {
        Ok(()) => "ok",
        Err(error) => {
            tracing::warn!(%error, "API 就绪检查: 数据库未就绪");
            "error"
        }
    };

    let is_ready = db_status == "ok";
    let body = serde_json::json!({
        "status": if is_ready { "ok" } else { "degraded" },
        "components": {
            "database": db_status,
        }
    });

    if is_ready {
        (StatusCode::OK, axum::Json(body))
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, axum::Json(body))
    }
}

pub async fn prometheus_metrics(State(state): State<AppState>) -> impl IntoResponse {
    let url = format!("{}/manage/media-metrics", state.sip_manage_base);
    let secret = state.internal_secret.clone();
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

    let cdr_url = format!("{}/manage/cdr-metrics", state.sip_manage_base);
    match state
        .internal_client
        .get(&cdr_url)
        .header("X-VOS-Token", &state.internal_secret)
        .send()
        .await
    {
        Ok(response) => match response.json::<CdrMetricsSnapshot>().await {
            Ok(snapshot) => Metrics::update_cdr_metrics(&snapshot),
            Err(error) => tracing::debug!(%error, "failed to decode sip-edge CDR metrics"),
        },
        Err(error) => tracing::debug!(%error, "failed to fetch sip-edge CDR metrics"),
    }

    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; version=0.0.4; charset=utf-8"),
    );
    (headers, Metrics::encode_metrics())
}

use axum::Json;
use serde::Serialize;
use serde_json::json;
use sqlx::Row;
use std::collections::HashMap;

#[derive(Debug, Serialize)]
pub struct SystemConfigResponse {
    pub configs: HashMap<String, String>,
    pub metadata: HashMap<String, SystemConfigMetadata>,
}

#[derive(Debug, Serialize)]
pub struct SystemConfigMetadata {
    pub category: &'static str,
    pub value_type: &'static str,
    pub apply_mode: &'static str,
    pub sensitive: bool,
}

/// 获取全局系统配置
pub async fn get_system_configs(State(state): State<AppState>) -> impl IntoResponse {
    let rows = sqlx::query("SELECT config_key, config_value FROM system_configs")
        .fetch_all(state.store.pool())
        .await;

    match rows {
        Ok(items) => {
            let mut configs = HashMap::new();
            for item in items {
                let key: String = item.get("config_key");
                let val: String = item.get("config_value");
                if key == "media_cluster_json" {
                    continue;
                }
                configs.insert(
                    key.clone(),
                    if key == "secret_key" {
                        String::new()
                    } else {
                        val
                    },
                );
            }
            let metadata = configs
                .keys()
                .filter_map(|key| config_metadata(key).map(|value| (key.clone(), value)))
                .collect();
            (
                StatusCode::OK,
                Json(SystemConfigResponse { configs, metadata }),
            )
                .into_response()
        }
        Err(error) => {
            tracing::error!(%error, "Failed to get system configs");
            (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response()
        }
    }
}

/// 修改并更新全局系统配置
pub async fn update_system_configs(
    State(state): State<AppState>,
    Json(mut payload): Json<HashMap<String, String>>,
) -> impl IntoResponse {
    // GET masks this value as empty; posting an unchanged form must not erase the secret.
    if payload
        .get("secret_key")
        .is_some_and(|value| value.is_empty())
    {
        payload.remove("secret_key");
    }
    if let Err(message) = validate_system_configs(&payload) {
        return (StatusCode::BAD_REQUEST, message).into_response();
    }
    if let Err(message) = validate_realm_change(&state, &payload).await {
        return (StatusCode::CONFLICT, message).into_response();
    }
    let mut tx = match state.store.pool().begin().await {
        Ok(t) => t,
        Err(error) => {
            tracing::error!(%error, "Failed to start transaction");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Database transaction error",
            )
                .into_response();
        }
    };

    for (k, v) in &payload {
        let res = sqlx::query(
            "INSERT INTO system_configs (config_key, config_value) VALUES ($1, $2) ON CONFLICT (config_key) DO UPDATE SET config_value = $2"
        )
        .bind(k)
        .bind(v)
        .execute(&mut *tx)
        .await;

        if let Err(error) = res {
            tracing::error!(%error, "Failed to update config");
            let _ = tx.rollback().await;
            return (StatusCode::INTERNAL_SERVER_ERROR, "Database write error").into_response();
        }
    }

    if let Err(error) = tx.commit().await {
        tracing::error!(%error, "Failed to commit transaction");
        return (StatusCode::INTERNAL_SERVER_ERROR, "Database commit error").into_response();
    }

    // 双写写入 Redis
    let mut redis = state.redis_client.clone();
    let mut pipeline = redis::pipe();
    for (key, value) in &payload {
        pipeline.hset("vos_rs:system_configs", key, value).ignore();
    }
    if let Err(error) = pipeline.query_async::<()>(&mut redis).await {
        tracing::error!(%error, "Redis 系统配置批量更新失败");
    }

    let metadata: HashMap<_, _> = payload
        .keys()
        .filter_map(|key| config_metadata(key).map(|value| (key.clone(), value)))
        .collect();
    (
        StatusCode::OK,
        Json(json!({
            "updated": payload.keys().collect::<Vec<_>>(),
            "metadata": metadata,
            "apply_mode": "restart_required",
            "restart_required": true,
        })),
    )
        .into_response()
}

async fn validate_realm_change(
    state: &AppState,
    configs: &HashMap<String, String>,
) -> Result<(), &'static str> {
    let Some(new_realm) = configs.get("realm") else {
        return Ok(());
    };
    if new_realm.trim().is_empty() {
        return Err("SIP realm 不能为空");
    }
    let current = sqlx::query_scalar::<_, String>(
        "SELECT config_value FROM system_configs WHERE config_key = 'realm'",
    )
    .fetch_optional(state.store.pool())
    .await
    .map_err(|_| "读取当前 SIP realm 失败")?
    .unwrap_or_else(|| "vos-rs".to_string());
    if current == *new_realm {
        return Ok(());
    }
    let user_count = state
        .store
        .count_users()
        .await
        .map_err(|_| "检查 SIP 用户失败")?;
    let trunk_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sip_gateways WHERE role='access' AND access_auth_mode IN ('digest_register','ip_and_digest')",
    )
    .fetch_one(state.store.pool())
    .await
    .map_err(|_| "检查接入中继凭据失败")?;
    validate_realm_transition(&current, new_realm, user_count + trunk_count)
}

fn validate_realm_transition(
    current: &str,
    requested: &str,
    user_count: i64,
) -> Result<(), &'static str> {
    if current != requested && user_count > 0 {
        Err("存在 SIP 用户或注册认证中继时禁止修改 realm；修改会使现有 Digest HA1 凭据全部失效")
    } else {
        Ok(())
    }
}

fn validate_system_configs(configs: &HashMap<String, String>) -> Result<(), &'static str> {
    validate_config_types(configs)?;
    validate_positive_config_values(configs)?;
    validate_config_relationships(configs)
}

fn validate_config_types(configs: &HashMap<String, String>) -> Result<(), &'static str> {
    for (key, value) in configs {
        let Some(kind) = config_value_kind(key) else {
            return Err("包含不支持的系统配置项");
        };
        match kind {
            "bool" if !matches!(value.as_str(), "true" | "false" | "1" | "0") => {
                return Err("布尔配置值无效");
            }
            "integer" if value.parse::<u64>().is_err() => return Err("整数配置值无效"),
            "number" if value.parse::<f64>().is_err() => return Err("数值配置值无效"),
            _ => {}
        }
    }
    Ok(())
}

fn validate_positive_config_values(configs: &HashMap<String, String>) -> Result<(), &'static str> {
    for key in [
        "session_expires_gateway",
        "session_expires_caller",
        "sbc_max_concurrency",
        "cdr_queue_capacity",
        "recording_workers",
        "recording_queue_capacity",
        "cluster_heartbeat_interval_secs",
        "cluster_node_timeout_secs",
        "cluster_dialog_ttl_secs",
    ] {
        if configs
            .get(key)
            .is_some_and(|value| value.parse::<u64>().ok() == Some(0))
        {
            return Err("时长、并发与队列配置必须大于零");
        }
    }
    for key in ["sbc_rate_limit_capacity", "sbc_rate_limit_fill_rate"] {
        if configs.get(key).is_some_and(|value| {
            value
                .parse::<f64>()
                .map_or(true, |number| !number.is_finite() || number <= 0.0)
        }) {
            return Err("SBC 限速配置必须是大于零的有限数值");
        }
    }
    Ok(())
}

fn validate_config_relationships(configs: &HashMap<String, String>) -> Result<(), &'static str> {
    if configs
        .get("recording_dir")
        .is_some_and(|value| value.trim().is_empty())
    {
        return Err("录音目录不能为空");
    }
    if configs
        .get("secret_key")
        .is_some_and(|value| value.len() < 16)
    {
        return Err("SIP 鉴权密钥长度不能少于 16 个字符");
    }
    if let (Some(heartbeat), Some(timeout)) = (
        configs
            .get("cluster_heartbeat_interval_secs")
            .and_then(|value| value.parse::<u64>().ok()),
        configs
            .get("cluster_node_timeout_secs")
            .and_then(|value| value.parse::<u64>().ok()),
    ) {
        if timeout <= heartbeat {
            return Err("集群节点超时必须大于心跳间隔");
        }
    }
    Ok(())
}

fn config_value_kind(key: &str) -> Option<&'static str> {
    match key {
        "rtp_symmetric_learning"
        | "rtp_anti_spoofing"
        | "database_routes_enabled"
        | "sbc_rate_limit_enabled"
        | "cluster_enabled"
        | "recording_enabled"
        | "balance_enforcement_enabled"
        | "billing_settlement_enabled"
        | "cdr_persistence_enabled"
        | "gateway_health_checks_enabled"
        | "udp_workers_auto"
        | "media_metrics_log"
        | "tls_allow_test_certificate"
        | "tls_insecure_skip_verify" => Some("bool"),
        "sbc_rate_limit_capacity" | "sbc_rate_limit_fill_rate" => Some("number"),
        "session_expires_gateway"
        | "session_expires_caller"
        | "sbc_max_concurrency"
        | "udp_workers"
        | "udp_receive_buffer_bytes"
        | "udp_send_buffer_bytes"
        | "cdr_queue_capacity"
        | "rtp_source_relearn_secs"
        | "recording_workers"
        | "recording_queue_capacity"
        | "recording_retention_secs"
        | "recording_min_free_bytes"
        | "recording_max_file_bytes"
        | "recording_max_duration_secs" => Some("integer"),
        "cluster_heartbeat_interval_secs"
        | "cluster_node_timeout_secs"
        | "cluster_dialog_ttl_secs" => Some("integer"),
        "recording_dir" | "default_gateway" | "realm" | "nonce" | "secret_key"
        | "tls_bind_addr" | "tls_cert_path" | "tls_key_path" | "tls_ca_path"
        | "tls_server_name" => Some("string"),
        _ => None,
    }
}

fn config_metadata(key: &str) -> Option<SystemConfigMetadata> {
    let value_type = config_value_kind(key)?;
    let category = if key.starts_with("recording_") || key == "storage_backend" {
        "recording"
    } else if key.starts_with("rtp_") || key.starts_with("media_") {
        "media"
    } else if key.starts_with("sbc_") || key.starts_with("tls_") {
        "security"
    } else if key.starts_with("cluster_") {
        "cluster"
    } else if key.starts_with("billing_") || key == "balance_enforcement_enabled" {
        "billing"
    } else if key.starts_with("cdr_") || key.starts_with("udp_") {
        "performance"
    } else if matches!(
        key,
        "database_routes_enabled" | "default_gateway" | "gateway_health_checks_enabled"
    ) {
        "routing"
    } else {
        "sip"
    };
    Some(SystemConfigMetadata {
        category,
        value_type,
        apply_mode: "restart_required",
        sensitive: matches!(key, "secret_key" | "tls_key_path"),
    })
}

#[cfg(test)]
mod tests {
    use super::{config_metadata, validate_realm_transition, validate_system_configs};
    use std::collections::HashMap;

    #[test]
    fn rejects_unknown_system_config_key() {
        let configs = HashMap::from([("unknown".to_string(), "1".to_string())]);
        assert!(validate_system_configs(&configs).is_err());
    }

    #[test]
    fn rejects_invalid_boolean_value() {
        let configs = HashMap::from([("recording_enabled".to_string(), "yes".to_string())]);
        assert!(validate_system_configs(&configs).is_err());
    }

    #[test]
    fn rejects_unsafe_runtime_values() {
        assert!(validate_system_configs(&HashMap::from([(
            "cluster_node_timeout_secs".to_string(),
            "0".to_string(),
        )]))
        .is_err());
        assert!(validate_system_configs(&HashMap::from([(
            "sbc_rate_limit_fill_rate".to_string(),
            "NaN".to_string(),
        )]))
        .is_err());
        assert!(validate_system_configs(&HashMap::from([(
            "recording_dir".to_string(),
            "  ".to_string(),
        )]))
        .is_err());
        assert!(validate_system_configs(&HashMap::from([
            (
                "cluster_heartbeat_interval_secs".to_string(),
                "5".to_string()
            ),
            ("cluster_node_timeout_secs".to_string(), "5".to_string()),
        ]))
        .is_err());
    }

    #[test]
    fn accepts_supported_system_configs() {
        let configs = HashMap::from([
            ("recording_enabled".to_string(), "true".to_string()),
            (
                "balance_enforcement_enabled".to_string(),
                "false".to_string(),
            ),
            (
                "billing_settlement_enabled".to_string(),
                "false".to_string(),
            ),
            ("cdr_persistence_enabled".to_string(), "false".to_string()),
            (
                "gateway_health_checks_enabled".to_string(),
                "false".to_string(),
            ),
        ]);
        assert!(validate_system_configs(&configs).is_ok());
    }

    #[test]
    fn realm_change_is_blocked_when_credentials_exist() {
        assert!(validate_realm_transition("vos-rs", "new-realm", 1).is_err());
        assert!(validate_realm_transition("vos-rs", "vos-rs", 1).is_ok());
        assert!(validate_realm_transition("vos-rs", "new-realm", 0).is_ok());
    }

    #[test]
    fn exposes_high_frequency_config_categories_and_apply_mode() {
        let keys = [
            "session_expires_gateway",
            "database_routes_enabled",
            "rtp_symmetric_learning",
            "recording_enabled",
            "balance_enforcement_enabled",
            "sbc_rate_limit_enabled",
            "cluster_node_timeout_secs",
        ];
        for key in keys {
            let metadata = config_metadata(key).expect("config key should be supported");
            assert_eq!(metadata.apply_mode, "restart_required");
        }
        assert_eq!(
            config_metadata("cluster_node_timeout_secs")
                .expect("cluster metadata")
                .category,
            "cluster"
        );
    }
}
