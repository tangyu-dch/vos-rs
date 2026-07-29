//! 运行时配置查询与安全热更新。

use std::sync::Arc;

use axum::{extract::State, http::StatusCode, Json};
use serde::Serialize;

use crate::{edge_state::runtime_config::RecordingRuntimeConfig, EdgeState};

const HOT_RECORDING_KEYS: [&str; 4] = [
    "recording_enabled",
    "recording_min_free_bytes",
    "recording_max_file_bytes",
    "recording_max_duration_secs",
];

#[derive(Serialize)]
pub(super) struct RecordingConfigResponse {
    apply_scope: &'static str,
    applied: [&'static str; 4],
    effective: RecordingRuntimeConfig,
}

/// 查询当前进程实际使用的录音热配置。
pub(super) async fn recording_config(
    State(state): State<Arc<EdgeState>>,
) -> Json<RecordingConfigResponse> {
    response((*state.recording_runtime_config()).clone())
}

/// 从数据库重新加载录音热配置，并原子替换运行时快照。
pub(super) async fn reload_recording_config(
    State(state): State<Arc<EdgeState>>,
) -> Result<Json<RecordingConfigResponse>, (StatusCode, String)> {
    let store = state.db_store.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "数据库配置源不可用".to_string(),
        )
    })?;
    let current = state.recording_runtime_config();
    let config = RecordingRuntimeConfig {
        enabled: load_bool(store, "recording_enabled", current.enabled).await?,
        min_free_bytes: load_u64(store, "recording_min_free_bytes", current.min_free_bytes).await?,
        max_file_bytes: load_u64(store, "recording_max_file_bytes", current.max_file_bytes).await?,
        max_duration_secs: load_u64(
            store,
            "recording_max_duration_secs",
            current.max_duration_secs,
        )
        .await?,
    };
    state.replace_recording_runtime_config(config.clone());
    tracing::info!(?config, "录音运行时配置已热更新，新录音会话立即生效");
    Ok(response(config))
}

fn response(config: RecordingRuntimeConfig) -> Json<RecordingConfigResponse> {
    Json(RecordingConfigResponse {
        apply_scope: "new_recording_sessions",
        applied: HOT_RECORDING_KEYS,
        effective: config,
    })
}

async fn load_bool(
    store: &cdr_core::PostgresCdrStore,
    key: &str,
    fallback: bool,
) -> Result<bool, (StatusCode, String)> {
    let Some(value) = load_value(store, key).await? else {
        return Ok(fallback);
    };
    match value.as_str() {
        "true" | "1" => Ok(true),
        "false" | "0" => Ok(false),
        _ => Err(invalid_value(key)),
    }
}

async fn load_u64(
    store: &cdr_core::PostgresCdrStore,
    key: &str,
    fallback: u64,
) -> Result<u64, (StatusCode, String)> {
    let Some(value) = load_value(store, key).await? else {
        return Ok(fallback);
    };
    value.parse().map_err(|_| invalid_value(key))
}

async fn load_value(
    store: &cdr_core::PostgresCdrStore,
    key: &str,
) -> Result<Option<String>, (StatusCode, String)> {
    store.get_system_config(key).await.map_err(|error| {
        tracing::error!(%error, key, "读取录音热配置失败");
        (
            StatusCode::SERVICE_UNAVAILABLE,
            format!("读取配置项 {key} 失败"),
        )
    })
}

fn invalid_value(key: &str) -> (StatusCode, String) {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        format!("配置项 {key} 的值无效"),
    )
}
