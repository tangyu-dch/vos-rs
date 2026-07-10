use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use serde_json::Value;

use crate::AppState;

type E = (StatusCode, String);
fn err(e: impl std::fmt::Display) -> E {
    (StatusCode::BAD_GATEWAY, e.to_string())
}

fn get_internal_token() -> Result<String, E> {
    std::env::var("VOS_RS_INTERNAL_SECRET").map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "VOS_RS_INTERNAL_SECRET 未配置".to_string(),
        )
    })
}

/// 活跃呼叫列表（转发到 sip-edge 管理 API）。
pub async fn list_active(State(state): State<AppState>) -> Result<Json<Value>, E> {
    let url = format!("{}/manage/active-calls", state.sip_manage_base);
    let token = get_internal_token()?;
    let v: Value = state
        .internal_client
        .get(&url)
        .header("X-VOS-Token", token)
        .send()
        .await
        .map_err(err)?
        .json()
        .await
        .map_err(err)?;
    Ok(Json(v))
}

/// RTP/录音聚合指标（转发到 sip-edge 管理 API）。
pub async fn media_metrics(State(state): State<AppState>) -> Result<Json<Value>, E> {
    let url = format!("{}/manage/media-metrics", state.sip_manage_base);
    let token = get_internal_token()?;
    let v: Value = state
        .internal_client
        .get(&url)
        .header("X-VOS-Token", token)
        .send()
        .await
        .map_err(err)?
        .json()
        .await
        .map_err(err)?;
    Ok(Json(v))
}

/// 强制拆线（转发到 sip-edge 管理 API）。
pub async fn terminate_call(
    State(state): State<AppState>,
    Path(call_id): Path<String>,
) -> Result<StatusCode, E> {
    let url = format!(
        "{}/manage/calls/{}/terminate",
        state.sip_manage_base, call_id
    );
    let token = get_internal_token()?;
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
) -> Result<Json<Value>, E> {
    let url = format!(
        "{}/manage/route-preview?destination={}",
        state.sip_manage_base,
        urlencoding(&q.destination)
    );
    let token = get_internal_token()?;
    let v: Value = state
        .internal_client
        .get(&url)
        .header("X-VOS-Token", token)
        .send()
        .await
        .map_err(err)?
        .json()
        .await
        .map_err(err)?;
    Ok(Json(v))
}

fn urlencoding(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | '~') {
                c.to_string()
            } else {
                format!("%{:02X}", c as u8)
            }
        })
        .collect()
}
