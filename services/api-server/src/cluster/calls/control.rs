use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde_json::Value;

use crate::AppState;

use super::helpers::{err, get_internal_token, urlencoding, E};

/// Returns media and control state for one active call.
pub async fn call_media(
    State(state): State<AppState>,
    Path(call_id): Path<String>,
) -> Result<(StatusCode, Json<Value>), E> {
    let historical = state.store.get_cdr(&call_id).await.map_err(|error| {
        tracing::error!(%error, %call_id, "读取通话媒体详情失败");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "database read failed".to_string(),
        )
    })?;
    match relay_call_get(&state, &call_id, "status").await {
        Ok((status, payload)) if status.is_success() => Ok((status, payload)),
        Ok((StatusCode::NOT_FOUND, _)) if historical.is_some() => Ok((
            StatusCode::OK,
            Json(serde_json::json!({ "runtime_availability": "not_active" })),
        )),
        Err(_) if historical.is_some() => Ok((
            StatusCode::OK,
            Json(serde_json::json!({ "runtime_availability": "unavailable" })),
        )),
        result => result,
    }
}

/// Aggregates persisted CDR data with the active call runtime state.
pub async fn call_detail(
    State(state): State<AppState>,
    Path(call_id): Path<String>,
) -> Result<(StatusCode, Json<Value>), E> {
    let historical = state.store.get_cdr(&call_id).await.map_err(|error| {
        tracing::error!(%error, %call_id, "读取通话详情失败");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "database read failed".to_string(),
        )
    })?;
    let runtime_result = relay_call_get(&state, &call_id, "status").await;
    match runtime_result {
        Ok((status, Json(runtime))) if status.is_success() => Ok((
            StatusCode::OK,
            Json(serde_json::json!({
                "historical": historical,
                "runtime": runtime,
                "runtime_availability": "available",
            })),
        )),
        Ok((StatusCode::NOT_FOUND, _)) if historical.is_some() => Ok((
            StatusCode::OK,
            Json(serde_json::json!({
                "historical": historical,
                "runtime": null,
                "runtime_availability": "not_active",
            })),
        )),
        Ok((StatusCode::NOT_FOUND, payload)) => Ok((StatusCode::NOT_FOUND, payload)),
        Ok((status, payload)) => Ok((status, payload)),
        Err(_error) if historical.is_some() => Ok((
            StatusCode::OK,
            Json(serde_json::json!({
                "historical": historical,
                "runtime": null,
                "runtime_availability": "unavailable",
            })),
        )),
        Err(error) => Err(error),
    }
}

/// Starts audio playback on a call leg through the SIP control plane.
pub async fn play(
    State(state): State<AppState>,
    Path(call_id): Path<String>,
    Json(payload): Json<Value>,
) -> Result<(StatusCode, Json<Value>), E> {
    relay_call_action(&state, &call_id, "play", payload).await
}

/// Stops audio playback on a call leg through the SIP control plane.
pub async fn stop_play(
    State(state): State<AppState>,
    Path(call_id): Path<String>,
    Json(payload): Json<Value>,
) -> Result<(StatusCode, Json<Value>), E> {
    relay_call_action(&state, &call_id, "stop-play", payload).await
}

/// Mutes a call leg through the SIP control plane.
pub async fn mute(
    State(state): State<AppState>,
    Path(call_id): Path<String>,
    Json(payload): Json<Value>,
) -> Result<(StatusCode, Json<Value>), E> {
    relay_call_action(&state, &call_id, "mute", payload).await
}

/// Unmutes a call leg through the SIP control plane.
pub async fn unmute(
    State(state): State<AppState>,
    Path(call_id): Path<String>,
    Json(payload): Json<Value>,
) -> Result<(StatusCode, Json<Value>), E> {
    relay_call_action(&state, &call_id, "unmute", payload).await
}

/// Starts supervisor monitoring through the SIP control plane.
pub async fn monitor(
    State(state): State<AppState>,
    Path(call_id): Path<String>,
    Json(payload): Json<Value>,
) -> Result<(StatusCode, Json<Value>), E> {
    relay_call_action(&state, &call_id, "monitor", payload).await
}

/// Stops supervisor monitoring through the SIP control plane.
pub async fn stop_monitor(
    State(state): State<AppState>,
    Path(call_id): Path<String>,
    Json(payload): Json<Value>,
) -> Result<(StatusCode, Json<Value>), E> {
    relay_call_action(&state, &call_id, "stop-monitor", payload).await
}

async fn relay_call_get(
    state: &AppState,
    call_id: &str,
    action: &str,
) -> Result<(StatusCode, Json<Value>), E> {
    let url = format!(
        "{}/manage/calls/{}/{}",
        state.sip_manage_base,
        urlencoding(call_id),
        action
    );
    let token = get_internal_token(&state.internal_secret)?;
    relay_json(state.internal_client.get(url).header("X-VOS-Token", token)).await
}

async fn relay_call_action(
    state: &AppState,
    call_id: &str,
    action: &str,
    payload: Value,
) -> Result<(StatusCode, Json<Value>), E> {
    let url = format!(
        "{}/manage/calls/{}/{}",
        state.sip_manage_base,
        urlencoding(call_id),
        action
    );
    let token = get_internal_token(&state.internal_secret)?;
    relay_json(
        state
            .internal_client
            .post(url)
            .header("X-VOS-Token", token)
            .json(&payload),
    )
    .await
}

pub(super) async fn relay_json(
    builder: reqwest::RequestBuilder,
) -> Result<(StatusCode, Json<Value>), E> {
    let response = builder.send().await.map_err(err)?;
    let status = response.status();
    let payload = response.json::<Value>().await.map_err(err)?;
    Ok((status, Json(payload)))
}
