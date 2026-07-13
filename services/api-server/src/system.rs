use axum::{
    extract::State,
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::IntoResponse,
};
use std::env;

use crate::metrics::{MediaMetricsSnapshot, Metrics};
use crate::AppState;

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

use axum::Json;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use sqlx::Row;

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
                configs.insert(key, val);
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
    let mut tx = match state.store.pool().begin().await {
        Ok(t) => t,
        Err(error) => {
            tracing::error!(%error, "Failed to start transaction");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Database transaction error").into_response();
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
    let redis_url = env::var("VOS_RS_REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
    if let Ok(client) = redis::Client::open(redis_url) {
        if let Ok(mut con) = client.get_multiplexed_tokio_connection().await {
            for (k, v) in &payload {
                let _: Result<(), redis::RedisError> = redis::cmd("HSET")
                    .arg("vos_rs:system_configs")
                    .arg(k)
                    .arg(v)
                    .query_async(&mut con)
                    .await;
            }
        }
    }

    StatusCode::OK.into_response()
}
