use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::Serialize;

use crate::AppState;

use super::helpers::E;

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
