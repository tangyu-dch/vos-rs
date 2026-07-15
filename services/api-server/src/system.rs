use crate::metrics::{MediaMetricsSnapshot, Metrics};
use crate::AppState;
use axum::{
    extract::State,
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::IntoResponse,
};

pub async fn health() -> &'static str {
    "OK"
}

/// 就绪探针：进程存活不代表数据库可用，只有依赖检查通过才返回 200。
pub async fn ready(State(state): State<AppState>) -> StatusCode {
    match state.store.ping().await {
        Ok(()) => StatusCode::OK,
        Err(error) => {
            tracing::warn!(%error, "API 就绪检查失败");
            StatusCode::SERVICE_UNAVAILABLE
        }
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

    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; version=0.0.4; charset=utf-8"),
    );
    (headers, Metrics::encode_metrics())
}

use axum::Json;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use std::collections::HashMap;

#[derive(Debug, Serialize, Deserialize)]
pub struct SystemConfigResponse {
    pub configs: HashMap<String, String>,
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
            (StatusCode::OK, Json(SystemConfigResponse { configs })).into_response()
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
    Json(payload): Json<HashMap<String, String>>,
) -> impl IntoResponse {
    if let Err(message) = validate_system_configs(&payload) {
        return (StatusCode::BAD_REQUEST, message).into_response();
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
    if let Ok(mut con) = state.redis_client.get_multiplexed_tokio_connection().await {
        for (k, v) in &payload {
            let _: Result<(), redis::RedisError> = redis::cmd("HSET")
                .arg("vos_rs:system_configs")
                .arg(k)
                .arg(v)
                .query_async(&mut con)
                .await;
        }
    }

    StatusCode::OK.into_response()
}

fn validate_system_configs(configs: &HashMap<String, String>) -> Result<(), &'static str> {
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

fn config_value_kind(key: &str) -> Option<&'static str> {
    match key {
        "rtp_symmetric_learning"
        | "rtp_anti_spoofing"
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
        "recording_dir" | "realm" | "nonce" | "secret_key" | "tls_bind_addr" | "tls_cert_path"
        | "tls_key_path" | "tls_ca_path" | "tls_server_name" => Some("string"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::validate_system_configs;
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
}
