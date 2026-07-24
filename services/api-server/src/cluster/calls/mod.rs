mod control;
mod helpers;
mod sipflow;

pub use control::{call_detail, call_media, monitor, mute, play, stop_monitor, stop_play, unmute};
pub use sipflow::*;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use serde_json::Value;

use crate::AppState;

use control::relay_json;
use helpers::{err, get_internal_token, urlencoding, E};

#[derive(Debug, Deserialize)]
pub struct ActiveCallsQuery {
    pub export: Option<bool>,
}

/// 活跃呼叫列表（转发到 sip-edge 管理 API）。
pub async fn list_active(
    State(state): State<AppState>,
    Query(q): Query<ActiveCallsQuery>,
) -> Result<axum::response::Response, E> {
    let url = format!("{}/manage/active-calls", state.sip_manage_base);
    let token = get_internal_token(&state.internal_secret)?;
    let response = state
        .internal_client
        .get(&url)
        .header("X-VOS-Token", token)
        .send()
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to fetch active calls: {e}"),
            )
        })?;
    let status = response.status();
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        return Err((status, text));
    }
    let val: Value = response.json().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to parse JSON: {e}"),
        )
    })?;

    if q.export.unwrap_or(false) {
        let headers = vec![
            "通话 ID",
            "主叫号码",
            "被叫号码",
            "状态",
            "开始时间",
            "中继网关",
        ];
        let mut rows = Vec::new();
        if let Some(arr) = val.as_array() {
            for item in arr {
                let call_id = item
                    .get("call_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                let caller = item
                    .get("caller")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                let callee = item
                    .get("callee")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                let state_str = item
                    .get("state")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                let started_at = item
                    .get("started_at_ms")
                    .and_then(|v| v.as_i64())
                    .map(|ms| {
                        time::OffsetDateTime::from_unix_timestamp_nanos((ms as i128) * 1_000_000)
                            .map(|t| t.to_string())
                            .unwrap_or_default()
                    })
                    .unwrap_or_default();
                let gateway = item
                    .get("gateway")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                rows.push(vec![
                    call_id.to_string(),
                    caller.to_string(),
                    callee.to_string(),
                    state_str.to_string(),
                    started_at,
                    gateway.to_string(),
                ]);
            }
        }
        return Ok(crate::system::utils::to_csv_response(
            "active_calls.csv",
            &headers,
            &rows,
        ));
    }

    use axum::response::IntoResponse;
    Ok((status, Json(val)).into_response())
}

/// RTP/录音聚合指标（转发到 sip-edge 管理 API）。
pub async fn media_metrics(State(state): State<AppState>) -> Result<(StatusCode, Json<Value>), E> {
    let url = format!("{}/manage/media-metrics", state.sip_manage_base);
    let token = get_internal_token(&state.internal_secret)?;
    relay_json(state.internal_client.get(&url).header("X-VOS-Token", token)).await
}

/// 强制拆线（转发到 sip-edge 管理 API）。
pub async fn terminate_call(
    State(state): State<AppState>,
    Path(call_id): Path<String>,
) -> Result<StatusCode, E> {
    let url = format!(
        "{}/manage/calls/{}/terminate",
        state.sip_manage_base,
        urlencoding(&call_id)
    );
    let token = get_internal_token(&state.internal_secret)?;
    let status = state
        .internal_client
        .post(&url)
        .header("X-VOS-Token", token)
        .send()
        .await
        .map_err(err)?
        .status();
    Ok(status)
}

#[derive(Deserialize)]
pub struct RoutePreviewQuery {
    pub destination: String,
}

/// 选路试算（转发到 sip-edge 管理 API）。
pub async fn route_preview(
    State(state): State<AppState>,
    Query(q): Query<RoutePreviewQuery>,
) -> Result<(StatusCode, Json<Value>), E> {
    let url = format!(
        "{}/manage/route-preview?destination={}",
        state.sip_manage_base,
        urlencoding(&q.destination)
    );
    let token = get_internal_token(&state.internal_secret)?;
    relay_json(state.internal_client.get(&url).header("X-VOS-Token", token)).await
}
