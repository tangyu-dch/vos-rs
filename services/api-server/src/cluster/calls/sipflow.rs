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
    /// Absolute timestamp string (HH:MM:SS.ffffff)
    pub timestamp_str: String,
    /// Delta in seconds/millis from previous message
    pub diff_ms: f64,
    /// Offset in milliseconds from the call start.
    pub offset_ms: i64,
    /// SIP method or response (e.g. "INVITE", "100 Trying", "200 OK", "BYE").
    pub message: String,
    /// Direction of the message: "uac_to_b2bua" | "b2bua_to_uac" | "b2bua_to_uas" | "uas_to_b2bua".
    pub direction: String,
    /// Source IP:Port
    pub from_addr: String,
    /// Destination IP:Port
    pub to_addr: String,
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

    // 1. Try to query the real captured SIP flows from the database using call_id or A-leg call_id from CDR
    let mut target_call_ids = vec![call_id.clone()];
    if let Some(ref cdr_data) = cdr {
        if let Some(ref orig) = cdr_data.audit.original_caller {
            if !target_call_ids.contains(orig) {
                target_call_ids.push(orig.clone());
            }
        }
    }

    let mut flows = Vec::new();
    for id in &target_call_ids {
        if let Ok(mut f) = state.store.get_sip_flows(id).await {
            flows.append(&mut f);
        }
    }
    flows.sort_by_key(|f| f.timestamp);

    if !flows.is_empty() {
        let first_flow_ms = flows
            .first()
            .map(|flow| {
                flow.timestamp.unix_timestamp() * 1000 + i64::from(flow.timestamp.millisecond())
            })
            .unwrap_or_default();
        let start_ms = cdr
            .as_ref()
            .map_or(first_flow_ms, |value| value.started_at_ms);
        let mut events: Vec<SipFlowEvent> = Vec::with_capacity(flows.len());
        let mut prev_ts_nanos: Option<i128> = None;
        for flow in flows {
            let flow_ms =
                flow.timestamp.unix_timestamp() * 1000 + i64::from(flow.timestamp.millisecond());
            let current_nanos = flow.timestamp.unix_timestamp_nanos();
            let diff_sec = match prev_ts_nanos {
                Some(prev) => (current_nanos - prev) as f64 / 1_000_000_000.0,
                None => 0.0,
            };
            prev_ts_nanos = Some(current_nanos);

            let (h, m, s) = flow.timestamp.time().as_hms();
            let micro = flow.timestamp.microsecond();
            let timestamp_str = format!("{:02}:{:02}:{:02}.{:06}", h, m, s, micro);

            let mut msg_display = flow.method.clone();
            if (flow.method.contains("INVITE") || flow.method.contains("200 OK"))
                && flow.raw_message.contains("v=0")
            {
                msg_display.push_str(" (SDP)");
            }

            // Detect UDP Retransmissions (same method & direction within 2000ms)
            let mut is_retransmission = false;
            if let Some(prev) = events.last() {
                if prev.message.replace(" (SDP)", "") == flow.method
                    && prev.direction == flow.direction
                    && (flow_ms - (start_ms + prev.offset_ms)).abs() < 2000
                {
                    is_retransmission = true;
                }
            }
            if is_retransmission {
                msg_display.push_str(" [重传]");
            }

            // Auto-recorrect B-leg direction for legacy DB records
            let mut final_direction = flow.direction.clone();
            let is_b_leg_call_id = flow.call_id != call_id;
            if is_b_leg_call_id {
                if final_direction == "uac_to_b2bua" || final_direction == "uas_to_b2bua" {
                    if flow.from_addr == "0.0.0.0:5060" || flow.from_addr.contains("5060") {
                        final_direction = "b2bua_to_uas".to_string();
                    } else {
                        final_direction = "uas_to_b2bua".to_string();
                    }
                } else if final_direction == "b2bua_to_uac" || final_direction == "b2bua_to_uas" {
                    if flow.to_addr == "0.0.0.0:5060" || flow.to_addr.contains("5060") {
                        final_direction = "uas_to_b2bua".to_string();
                    } else {
                        final_direction = "b2bua_to_uas".to_string();
                    }
                }
            }

            events.push(SipFlowEvent {
                timestamp_str,
                diff_ms: diff_sec,
                offset_ms: (flow_ms - start_ms).max(0),
                message: msg_display,
                direction: final_direction,
                from_addr: flow.from_addr.clone(),
                to_addr: flow.to_addr.clone(),
                note: format!("From: {} → To: {}", flow.from_addr, flow.to_addr),
                raw_message: Some(flow.raw_message),
            });
        }

        // Synthetic completion for BYE closing sequence if truncated in real capture
        let is_bye_truncated = events
            .last()
            .is_some_and(|ev| ev.message.contains("BYE") && ev.direction == "uac_to_b2bua");

        if is_bye_truncated {
            let last_ts = events.last().unwrap().timestamp_str.clone();
            let last_off = events.last().unwrap().offset_ms;
            let last_addr = events.last().unwrap().from_addr.clone();

            events.push(SipFlowEvent {
                timestamp_str: last_ts.clone(),
                diff_ms: 0.001,
                offset_ms: last_off + 1,
                message: "BYE".to_string(),
                direction: "b2bua_to_uas".to_string(),
                from_addr: "0.0.0.0:5060".to_string(),
                to_addr: last_addr.clone(),
                note: String::new(),
                raw_message: None,
            });
            events.push(SipFlowEvent {
                timestamp_str: last_ts.clone(),
                diff_ms: 0.001,
                offset_ms: last_off + 2,
                message: "200 OK".to_string(),
                direction: "uas_to_b2bua".to_string(),
                from_addr: last_addr.clone(),
                to_addr: "0.0.0.0:5060".to_string(),
                note: String::new(),
                raw_message: None,
            });
            events.push(SipFlowEvent {
                timestamp_str: last_ts,
                diff_ms: 0.001,
                offset_ms: last_off + 3,
                message: "200 OK".to_string(),
                direction: "b2bua_to_uac".to_string(),
                from_addr: "0.0.0.0:5060".to_string(),
                to_addr: last_addr,
                note: "Call terminated".to_string(),
                raw_message: None,
            });
        }

        // If the flow ends with non-2xx failure response (e.g. 603/486/404) sent from B2BUA to UAC,
        // check if ACK from B2BUA to UAS was truncated and synthesize it.
        let ends_with_failure_to_uac = events.last().is_some_and(|ev| {
            ev.direction == "b2bua_to_uac"
                && (ev.message.starts_with('3')
                    || ev.message.starts_with('4')
                    || ev.message.starts_with('5')
                    || ev.message.starts_with('6'))
        });
        let ends_with_uac_ack = events
            .last()
            .is_some_and(|ev| ev.direction == "uac_to_b2bua" && ev.message.starts_with("ACK"));

        if ends_with_failure_to_uac {
            // UAC hasn't sent ACK yet, or B2BUA ACK to UAS wasn't logged
            let last_ts = events.last().unwrap().timestamp_str.clone();
            let last_off = events.last().unwrap().offset_ms;
            let uas_addr = events
                .iter()
                .find(|e| e.direction == "b2bua_to_uas" || e.direction == "uas_to_b2bua")
                .map_or("uas".to_string(), |e| e.to_addr.clone());

            events.push(SipFlowEvent {
                timestamp_str: last_ts.clone(),
                diff_ms: 0.0005,
                offset_ms: last_off + 1,
                message: "ACK".to_string(),
                direction: "b2bua_to_uas".to_string(),
                from_addr: "0.0.0.0:5060".to_string(),
                to_addr: uas_addr,
                note: "Auto ACK to UAS".to_string(),
                raw_message: None,
            });
            events.push(SipFlowEvent {
                timestamp_str: last_ts,
                diff_ms: 0.0005,
                offset_ms: last_off + 2,
                message: "ACK".to_string(),
                direction: "uac_to_b2bua".to_string(),
                from_addr: "uac".to_string(),
                to_addr: "0.0.0.0:5060".to_string(),
                note: "Auto ACK from UAC".to_string(),
                raw_message: None,
            });
        } else if ends_with_uac_ack {
            // UAC ACK received, append missing B2BUA -> UAS ACK
            let uas_has_ack = events
                .iter()
                .any(|e| e.direction == "b2bua_to_uas" && e.message.starts_with("ACK"));
            if !uas_has_ack {
                let last_ts = events.last().unwrap().timestamp_str.clone();
                let last_off = events.last().unwrap().offset_ms;
                let uas_addr = events
                    .iter()
                    .find(|e| e.direction == "b2bua_to_uas" || e.direction == "uas_to_b2bua")
                    .map_or("uas".to_string(), |e| e.to_addr.clone());

                events.push(SipFlowEvent {
                    timestamp_str: last_ts,
                    diff_ms: 0.0005,
                    offset_ms: last_off + 1,
                    message: "ACK".to_string(),
                    direction: "b2bua_to_uas".to_string(),
                    from_addr: "0.0.0.0:5060".to_string(),
                    to_addr: uas_addr,
                    note: "Auto ACK to UAS".to_string(),
                    raw_message: None,
                });
            }
        }

        // If the flow ends with 200 OK from UAC -> B2BUA (callee initiated BYE flow), append final 200 OK to UAS
        let is_callee_bye_200_truncated = events
            .last()
            .is_some_and(|ev| ev.message.contains("200 OK") && ev.direction == "uac_to_b2bua");
        if is_callee_bye_200_truncated {
            let last_ts = events.last().unwrap().timestamp_str.clone();
            let last_off = events.last().unwrap().offset_ms;
            events.push(SipFlowEvent {
                timestamp_str: last_ts,
                diff_ms: 0.001,
                offset_ms: last_off + 1,
                message: "200 OK".to_string(),
                direction: "b2bua_to_uas".to_string(),
                from_addr: "0.0.0.0:5060".to_string(),
                to_addr: "uas".to_string(),
                note: "Call terminated".to_string(),
                raw_message: None,
            });
        }
        return Ok((StatusCode::OK, Json(events)));
    }

    // 活跃通话尚未生成 CDR，也可能暂未捕获到报文；返回空列表供前端持续重试。
    let Some(cdr) = cdr else {
        return Ok((StatusCode::OK, Json(Vec::new())));
    };

    let start_ms = cdr.started_at_ms;
    let answered_ms = cdr.answered_at_ms;
    let ended_ms = cdr.ended_at_ms;

    // 2. Fallback to synthesizing a canonical SIP flow timeline from CDR timestamps
    let mut events: Vec<SipFlowEvent> = Vec::new();
    let make_evt = |offset_ms: i64, msg: &str, dir: &str, note: String| SipFlowEvent {
        timestamp_str: format!("+{}ms", offset_ms),
        diff_ms: 0.001,
        offset_ms,
        message: msg.to_string(),
        direction: dir.to_string(),
        from_addr: String::new(),
        to_addr: String::new(),
        note,
        raw_message: None,
    };

    // Phase 1: Setup – UAC sends INVITE to B2BUA
    events.push(make_evt(
        0,
        "INVITE (SDP)",
        "uac_to_b2bua",
        format!(
            "From: {} → To: {}",
            cdr.caller.as_deref().unwrap_or("-"),
            cdr.callee.as_deref().unwrap_or("-")
        ),
    ));
    events.push(make_evt(1, "100 Trying", "b2bua_to_uac", String::new()));
    events.push(make_evt(
        2,
        "INVITE (SDP)",
        "b2bua_to_uas",
        format!("Forwarded to gateway ({} leg)", cdr.direction),
    ));
    events.push(make_evt(3, "100 Trying", "uas_to_b2bua", String::new()));

    match cdr.status.as_str() {
        "answered" => {
            let ring_ms = answered_ms
                .map(|a| ((a - start_ms) / 2).max(4))
                .unwrap_or(50);
            events.push(make_evt(
                ring_ms,
                "180 Ringing",
                "uas_to_b2bua",
                String::new(),
            ));
            events.push(make_evt(
                ring_ms + 1,
                "180 Ringing",
                "b2bua_to_uac",
                String::new(),
            ));

            let ans_off = answered_ms.map(|a| a - start_ms).unwrap_or(ring_ms * 2);
            events.push(make_evt(
                ans_off,
                "200 OK (SDP)",
                "uas_to_b2bua",
                "Call answered".to_string(),
            ));
            events.push(make_evt(
                ans_off + 1,
                "200 OK (SDP)",
                "b2bua_to_uac",
                String::new(),
            ));
            events.push(make_evt(ans_off + 2, "ACK", "uac_to_b2bua", String::new()));
            events.push(make_evt(ans_off + 3, "ACK", "b2bua_to_uas", String::new()));

            let bye_off = ended_ms - start_ms;
            let duration_ms = cdr.duration_ms;
            events.push(make_evt(
                bye_off,
                "BYE",
                "uac_to_b2bua",
                format!("Duration: {} ms", duration_ms),
            ));
            events.push(make_evt(bye_off + 1, "BYE", "b2bua_to_uas", String::new()));
            events.push(make_evt(
                bye_off + 2,
                "200 OK",
                "uas_to_b2bua",
                String::new(),
            ));
            events.push(make_evt(
                bye_off + 3,
                "200 OK",
                "b2bua_to_uac",
                "Call terminated".to_string(),
            ));
        }
        "canceled" => {
            let cancel_off = ended_ms - start_ms;
            events.push(make_evt(
                cancel_off,
                "CANCEL",
                "uac_to_b2bua",
                String::new(),
            ));
            events.push(make_evt(
                cancel_off + 1,
                "CANCEL",
                "b2bua_to_uas",
                String::new(),
            ));
            events.push(make_evt(
                cancel_off + 2,
                "487 Request Terminated",
                "uas_to_b2bua",
                String::new(),
            ));
            events.push(make_evt(
                cancel_off + 3,
                "200 OK (CANCEL)",
                "b2bua_to_uac",
                String::new(),
            ));
            events.push(make_evt(
                cancel_off + 4,
                "487 Request Terminated",
                "b2bua_to_uac",
                "Call canceled".to_string(),
            ));
        }
        _ => {
            // failed
            let fail_code = cdr.failure_status_code.unwrap_or(503);
            let fail_off = ended_ms - start_ms;
            let reason = format!(
                "{} {}",
                fail_code,
                cdr.failure_reason
                    .as_deref()
                    .unwrap_or("Service Unavailable")
            );
            events.push(make_evt(fail_off, &reason, "uas_to_b2bua", String::new()));
            events.push(make_evt(
                fail_off + 1,
                &reason,
                "b2bua_to_uac",
                "Call failed".to_string(),
            ));
            events.push(make_evt(fail_off + 2, "ACK", "uac_to_b2bua", String::new()));
        }
    }

    Ok((StatusCode::OK, Json(events)))
}
