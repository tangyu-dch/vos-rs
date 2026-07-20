use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::AppState;

type E = (StatusCode, String);
fn err(e: impl std::fmt::Display) -> E {
    (StatusCode::BAD_GATEWAY, e.to_string())
}

fn get_internal_token(token: &str) -> Result<String, E> {
    if token.is_empty() {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_secret 未配置".to_string(),
        ));
    }
    Ok(token.to_string())
}

/// 活跃呼叫列表（转发到 sip-edge 管理 API）。
pub async fn list_active(State(state): State<AppState>) -> Result<(StatusCode, Json<Value>), E> {
    let url = format!("{}/manage/active-calls", state.sip_manage_base);
    let token = get_internal_token(&state.internal_secret)?;
    relay_json(state.internal_client.get(&url).header("X-VOS-Token", token)).await
}

/// RTP/录音聚合指标（转发到 sip-edge 管理 API）。
pub async fn media_metrics(State(state): State<AppState>) -> Result<(StatusCode, Json<Value>), E> {
    let url = format!("{}/manage/media-metrics", state.sip_manage_base);
    let token = get_internal_token(&state.internal_secret)?;
    relay_json(state.internal_client.get(&url).header("X-VOS-Token", token)).await
}

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

async fn relay_json(builder: reqwest::RequestBuilder) -> Result<(StatusCode, Json<Value>), E> {
    let response = builder.send().await.map_err(err)?;
    let status = response.status();
    let payload = response.json::<Value>().await.map_err(err)?;
    Ok((status, Json(payload)))
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

fn urlencoding(s: &str) -> String {
    s.as_bytes()
        .iter()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
                (*byte as char).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}

/// A single signaling event in the SIP flow timeline.
#[derive(Debug, Serialize)]
pub struct SipFlowEvent {
    /// Offset in milliseconds from the call start.
    pub offset_ms: i64,
    /// SIP method or response (e.g. "INVITE", "100 Trying", "200 OK", "BYE").
    pub message: String,
    /// Direction of the message: "uac_to_b2bua" | "b2bua_to_uac" | "b2bua_to_uas" | "uas_to_b2bua".
    pub direction: String,
    /// Optional description.
    pub note: String,
    /// Complete raw SIP message text (if captured).
    pub raw_message: Option<String>,
}

/// Synthesises a B2BUA SIP flow timeline from the persisted CDR data,
/// or queries the real captured SIP messages if present in the database.
pub async fn call_sipflow(
    State(state): State<AppState>,
    Path(call_id): Path<String>,
) -> Result<(StatusCode, Json<Vec<SipFlowEvent>>), E> {
    let cdr = state
        .store
        .get_cdr(&call_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let Some(cdr) = cdr else {
        return Err((StatusCode::NOT_FOUND, "CDR not found".to_string()));
    };

    let start_ms = cdr.started_at_ms;
    let answered_ms = cdr.answered_at_ms;
    let ended_ms = cdr.ended_at_ms;

    // 1. Try to query the real captured SIP flows from the database
    if let Ok(flows) = state.store.get_sip_flows(&call_id).await {
        if !flows.is_empty() {
            let mut events = Vec::with_capacity(flows.len());
            for flow in flows {
                let flow_ms =
                    flow.timestamp.unix_timestamp() * 1000 + (flow.timestamp.millisecond() as i64);
                let offset_ms = (flow_ms - start_ms).max(0);
                events.push(SipFlowEvent {
                    offset_ms,
                    message: flow.method,
                    direction: flow.direction,
                    note: format!("From: {} → To: {}", flow.from_addr, flow.to_addr),
                    raw_message: Some(flow.raw_message),
                });
            }
            return Ok((StatusCode::OK, Json(events)));
        }
    }

    // 2. Fallback to synthesizing a canonical SIP flow timeline from CDR timestamps
    let mut events: Vec<SipFlowEvent> = Vec::new();

    // Phase 1: Setup – UAC sends INVITE to B2BUA
    events.push(SipFlowEvent {
        offset_ms: 0,
        message: "INVITE".to_string(),
        direction: "uac_to_b2bua".to_string(),
        note: format!(
            "From: {} → To: {}",
            cdr.caller.as_deref().unwrap_or("-"),
            cdr.callee.as_deref().unwrap_or("-")
        ),
        raw_message: None,
    });
    events.push(SipFlowEvent {
        offset_ms: 1,
        message: "100 Trying".to_string(),
        direction: "b2bua_to_uac".to_string(),
        note: String::new(),
        raw_message: None,
    });
    events.push(SipFlowEvent {
        offset_ms: 2,
        message: "INVITE".to_string(),
        direction: "b2bua_to_uas".to_string(),
        note: format!("Forwarded to gateway ({} leg)", &cdr.direction),
        raw_message: None,
    });
    events.push(SipFlowEvent {
        offset_ms: 3,
        message: "100 Trying".to_string(),
        direction: "uas_to_b2bua".to_string(),
        note: String::new(),
        raw_message: None,
    });

    match cdr.status.as_str() {
        "answered" => {
            let ring_ms = answered_ms
                .map(|a| ((a - start_ms) / 2).max(4))
                .unwrap_or(50);
            events.push(SipFlowEvent {
                offset_ms: ring_ms,
                message: "180 Ringing".to_string(),
                direction: "uas_to_b2bua".to_string(),
                note: String::new(),
                raw_message: None,
            });
            events.push(SipFlowEvent {
                offset_ms: ring_ms + 1,
                message: "180 Ringing".to_string(),
                direction: "b2bua_to_uac".to_string(),
                note: String::new(),
                raw_message: None,
            });

            let ans_off = answered_ms.map(|a| a - start_ms).unwrap_or(ring_ms * 2);
            events.push(SipFlowEvent {
                offset_ms: ans_off,
                message: "200 OK".to_string(),
                direction: "uas_to_b2bua".to_string(),
                note: "Call answered".to_string(),
                raw_message: None,
            });
            events.push(SipFlowEvent {
                offset_ms: ans_off + 1,
                message: "200 OK".to_string(),
                direction: "b2bua_to_uac".to_string(),
                note: String::new(),
                raw_message: None,
            });
            events.push(SipFlowEvent {
                offset_ms: ans_off + 2,
                message: "ACK".to_string(),
                direction: "uac_to_b2bua".to_string(),
                note: String::new(),
                raw_message: None,
            });
            events.push(SipFlowEvent {
                offset_ms: ans_off + 3,
                message: "ACK".to_string(),
                direction: "b2bua_to_uas".to_string(),
                note: String::new(),
                raw_message: None,
            });

            let bye_off = ended_ms - start_ms;
            let duration_ms = cdr.duration_ms;
            events.push(SipFlowEvent {
                offset_ms: bye_off,
                message: "BYE".to_string(),
                direction: "uac_to_b2bua".to_string(),
                note: format!("Duration: {} ms", duration_ms),
                raw_message: None,
            });
            events.push(SipFlowEvent {
                offset_ms: bye_off + 1,
                message: "BYE".to_string(),
                direction: "b2bua_to_uas".to_string(),
                note: String::new(),
                raw_message: None,
            });
            events.push(SipFlowEvent {
                offset_ms: bye_off + 2,
                message: "200 OK".to_string(),
                direction: "uas_to_b2bua".to_string(),
                note: String::new(),
                raw_message: None,
            });
            events.push(SipFlowEvent {
                offset_ms: bye_off + 3,
                message: "200 OK".to_string(),
                direction: "b2bua_to_uac".to_string(),
                note: "Call terminated".to_string(),
                raw_message: None,
            });
        }
        "canceled" => {
            let cancel_off = ended_ms - start_ms;
            events.push(SipFlowEvent {
                offset_ms: cancel_off,
                message: "CANCEL".to_string(),
                direction: "uac_to_b2bua".to_string(),
                note: String::new(),
                raw_message: None,
            });
            events.push(SipFlowEvent {
                offset_ms: cancel_off + 1,
                message: "CANCEL".to_string(),
                direction: "b2bua_to_uas".to_string(),
                note: String::new(),
                raw_message: None,
            });
            events.push(SipFlowEvent {
                offset_ms: cancel_off + 2,
                message: "487 Request Terminated".to_string(),
                direction: "uas_to_b2bua".to_string(),
                note: String::new(),
                raw_message: None,
            });
            events.push(SipFlowEvent {
                offset_ms: cancel_off + 3,
                message: "200 OK (CANCEL)".to_string(),
                direction: "b2bua_to_uac".to_string(),
                note: String::new(),
                raw_message: None,
            });
            events.push(SipFlowEvent {
                offset_ms: cancel_off + 4,
                message: "487 Request Terminated".to_string(),
                direction: "b2bua_to_uac".to_string(),
                note: "Call canceled".to_string(),
                raw_message: None,
            });
        }
        _ => {
            // failed
            let fail_code = cdr.failure_status_code.unwrap_or(503);
            let fail_off = ended_ms - start_ms;
            events.push(SipFlowEvent {
                offset_ms: fail_off,
                message: format!(
                    "{} {}",
                    fail_code,
                    cdr.failure_reason
                        .as_deref()
                        .unwrap_or("Service Unavailable")
                ),
                direction: "uas_to_b2bua".to_string(),
                note: String::new(),
                raw_message: None,
            });
            events.push(SipFlowEvent {
                offset_ms: fail_off + 1,
                message: format!(
                    "{} {}",
                    fail_code,
                    cdr.failure_reason
                        .as_deref()
                        .unwrap_or("Service Unavailable")
                ),
                direction: "b2bua_to_uac".to_string(),
                note: "Call failed".to_string(),
                raw_message: None,
            });
            events.push(SipFlowEvent {
                offset_ms: fail_off + 2,
                message: "ACK".to_string(),
                direction: "uac_to_b2bua".to_string(),
                note: String::new(),
                raw_message: None,
            });
        }
    }

    Ok((StatusCode::OK, Json(events)))
}

#[cfg(test)]
mod tests {
    use super::urlencoding;

    #[test]
    fn path_and_query_delimiters_are_percent_encoded() {
        assert_eq!(urlencoding("a/b?c#d"), "a%2Fb%3Fc%23d");
        assert_eq!(urlencoding("通话"), "%E9%80%9A%E8%AF%9D");
    }
}
