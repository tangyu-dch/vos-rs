use std::net::SocketAddr;

use sip_core::{HeaderName, HeaderValue, SipRequest};
use tracing::{info, warn};

use crate::config::EdgeConfig;
use crate::edge_state::{DialogLeg, EdgeState, InboundTransaction, PendingDatagram};
use crate::sip::response;
use crate::timers::calculate_mos_for_legs;

/// Pre-forward BYE/CANCEL handling: collect media metrics, persist DTMF audit events,
/// invoke `call_manager` termination, perform conference local-end fast path, and
/// clear transfer leg media pairing.
///
/// Appends the local 200 OK response to `datagrams`. Returns `true` when the request
/// is fully terminated locally (conference call fast path) and the orchestrator must
/// return immediately; returns `false` to continue with cross-leg forwarding.
#[allow(clippy::too_many_arguments)]
pub(super) async fn handle_bye_cancel_pre_forward(
    request: &SipRequest,
    peer: SocketAddr,
    edge_state: &EdgeState,
    edge_config: &EdgeConfig,
    transaction: &InboundTransaction,
    transaction_call_id: &str,
    call_id: &str,
    source_leg: DialogLeg,
    datagrams: &mut Vec<PendingDatagram>,
) -> bool {
    let mut caller_rtcp = None;
    let mut gateway_rtcp = None;

    if let Some(endpoint) = &transaction.caller_relay_rtp {
        caller_rtcp = Some(
            edge_state
                .media_relay
                .metrics_for_port(endpoint.port)
                .rtcp_quality,
        );
    }
    if let Some(endpoint) = &transaction.gateway_relay_rtp {
        gateway_rtcp = Some(
            edge_state
                .media_relay
                .metrics_for_port(endpoint.port)
                .rtcp_quality,
        );
    }

    let metrics = if caller_rtcp.is_some() || gateway_rtcp.is_some() {
        Some(calculate_mos_for_legs(
            caller_rtcp.as_ref(),
            gateway_rtcp.as_ref(),
        ))
    } else {
        None
    };

    let media_session_id = transaction.session_id.as_str();
    let dtmf_digits = edge_state.media_relay.get_dtmf_digits(media_session_id);
    if let Some(digits) = &dtmf_digits {
        info!(
            session_id = media_session_id,
            digits = %digits,
            "collected DTMF digits for call"
        );
    }
    edge_state.media_relay.clear_dtmf_digits(media_session_id);

    // Collect DTMF audit events for persistence to the detail table.
    let mut dtmf_events = edge_state.media_relay.take_dtmf_events(media_session_id);
    if !dtmf_events.is_empty() {
        info!(
            session_id = media_session_id,
            count = dtmf_events.len(),
            "collected DTMF audit events for call"
        );
        if let Some(db) = edge_state.db_store.clone() {
            let dtmf_call_id = transaction.dialogs.caller.call_id.clone();
            for event in &mut dtmf_events {
                event.call_id.clone_from(&dtmf_call_id);
            }
            tokio::spawn(async move {
                if let Err(error) = db.insert_dtmf_events_batch(&dtmf_events).await {
                    warn!(%error, %dtmf_call_id, "failed to persist DTMF audit events");
                }
            });
        }
    } else {
        edge_state.media_relay.clear_dtmf_events(media_session_id);
    }

    let mut termination_request = request.clone();
    termination_request.headers.replace(
        HeaderName::new("call-id").unwrap(),
        HeaderValue::new_owned(transaction_call_id.to_string()),
    );
    match edge_state.call_manager.handle_inbound_termination(
        &termination_request,
        metrics,
        dtmf_digits,
    ) {
        Ok(outcome) => {
            // Decrement active call count for the gateway.
            if let Some(gw_id) = edge_state
                .call_manager
                .current_gateway_id(transaction_call_id)
            {
                edge_state.gateway_health.decrement_active(&gw_id);
                let status = edge_state.gateway_health.get_gateway_status(&gw_id);
                crate::timers::persist_gateway_health(edge_state, gw_id.clone(), status);
            }

            crate::billing::settle_completed_call(edge_state, &outcome.call_id);

            // 如果是会议呼叫（单腿 UAS 呼叫），直接在本地终结并返回 200 OK，不转发给其他任何节点
            let out_user = transaction
                .dialogs
                .gateway
                .remote_target
                .user
                .as_deref()
                .unwrap_or("");
            if out_user.starts_with("conf_")
                || out_user.starts_with("room_")
                || out_user == "vosrs-playback"
                || out_user == "vosrs-gather"
                || out_user == "vosrs-stream"
            {
                let username = transaction.original_request.as_ref().and_then(|req| {
                    crate::edge_state::EdgeState::username_from_request(req.as_ref())
                });
                if let Some(ref uname) = username {
                    edge_state.decrement_user_concurrency(uname);
                }
                edge_state.decrement_tenant_concurrency(transaction.tenant.as_ref());
                let duration_secs = transaction
                    .established_at
                    .map(|i| i.elapsed().as_secs())
                    .unwrap_or(0);
                if edge_config.webhooks.control_mode == "http"
                    || edge_config.webhooks.control_mode == "nats"
                {
                    let edge_state_clone = edge_state
                        .self_weak
                        .get()
                        .and_then(|w| w.upgrade())
                        .unwrap();
                    let edge_config_clone = edge_config.clone();
                    let cid_clone = call_id.to_string();
                    tokio::spawn(async move {
                        let event = call_core::WebhookEvent {
                            event_id: uuid::Uuid::new_v4().to_string(),
                            schema_version: "1.0".to_string(),
                            call_id: cid_clone,
                            sequence: 5,
                            occurred_at_ms: std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_millis() as i64,
                            event: call_core::CallEvent::CallFinished {
                                duration_secs,
                                sip_status: Some(200),
                                q850_cause: Some(16),
                                reason: "Normal clearing (local session ended)".to_string(),
                                leg: "a_leg".to_string(),
                            },
                        };
                        let _ = crate::sip::handlers::interactive_control::post_webhook_event(
                            &edge_state_clone,
                            &edge_config_clone,
                            &event,
                        )
                        .await;
                    });
                }
                edge_state.teardown_call_transaction(transaction_call_id);

                datagrams.push(PendingDatagram::new(
                    peer.to_string(),
                    response::ok_for_request(request),
                ));
                return true;
            }

            if let Some(transfer) = &transaction.transfer_dialog {
                let transferee_port = match transfer.transferee_leg {
                    DialogLeg::Caller => transaction.caller_relay_rtp.as_ref().map(|ep| ep.port),
                    DialogLeg::Gateway => transaction.gateway_relay_rtp.as_ref().map(|ep| ep.port),
                    DialogLeg::Transfer => None,
                };
                if let Some(tp) = transferee_port {
                    if let Some(cp) = edge_state.media_relay.peer_port_for(tp) {
                        edge_state.media_relay.clear_target(cp);
                    }
                }
            }

            datagrams.push(PendingDatagram::new(
                peer.to_string(),
                response::ok_for_request(request),
            ));
        }
        Err(error) => {
            // 即使 call_manager 找不到记录（UnknownCall），也必须把 200 OK 发回给发送方
            // 并继续转发 BYE 给对端，否则另一方的呼叫永远无法挂断。
            warn!(
                call_id,
                %error,
                source_leg = ?source_leg,
                "handle_inbound_termination failed; still forwarding BYE to peer leg"
            );
            datagrams.push(PendingDatagram::new(
                peer.to_string(),
                response::ok_for_request(request),
            ));
        }
    }
    false
}

/// Post-forward BYE/CANCEL cleanup: decrement user concurrency, emit CallFinished
/// webhook, tear down the call transaction, and broadcast BLF terminated state.
///
/// Invoked only after the B2BUA request has been built and pushed to the datagrams
/// vector by the orchestrator. Returns BLF NOTIFY datagrams to be sent by the caller.
pub(super) fn forward_bye_cancel_cleanup(
    edge_state: &EdgeState,
    edge_config: &EdgeConfig,
    transaction: &InboundTransaction,
    transaction_call_id: &str,
    call_id: &str,
) -> Vec<PendingDatagram> {
    let username: Option<String> = transaction
        .original_request
        .as_ref()
        .and_then(|req| crate::edge_state::EdgeState::username_from_request(req.as_ref()));
    if let Some(ref uname) = username {
        edge_state.decrement_user_concurrency(uname);
    }
    edge_state.decrement_tenant_concurrency(transaction.tenant.as_ref());
    let duration_secs = transaction
        .established_at
        .map(|i| i.elapsed().as_secs())
        .unwrap_or(0);

    // BLF: 仅在呼叫确实建立过后才广播 terminated 状态，避免对未接通呼叫误发通知
    let blf_datagrams = if transaction.established_at.is_some() {
        let caller_aor = transaction.dialogs.caller.remote_uri.to_string();
        let callee_aor = transaction.dialogs.caller.local_uri.to_string();
        crate::sip::handlers::subscribe::trigger_dialog_state_change(
            edge_state,
            edge_config,
            &caller_aor,
            &callee_aor,
            call_id,
            crate::sip::handlers::subscribe::DialogStateChange::Terminated,
        )
    } else {
        Vec::new()
    };
    if edge_config.webhooks.control_mode == "http" || edge_config.webhooks.control_mode == "nats" {
        let edge_state_clone = edge_state
            .self_weak
            .get()
            .and_then(|w| w.upgrade())
            .unwrap();
        let edge_config_clone = edge_config.clone();
        let cid_clone = call_id.to_string();
        tokio::spawn(async move {
            let event = call_core::WebhookEvent {
                event_id: uuid::Uuid::new_v4().to_string(),
                schema_version: "1.0".to_string(),
                call_id: cid_clone,
                sequence: 5,
                occurred_at_ms: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as i64,
                event: call_core::CallEvent::CallFinished {
                    duration_secs,
                    sip_status: Some(200),
                    q850_cause: Some(16),
                    reason: "Normal clearing".to_string(),
                    leg: "a_leg".to_string(),
                },
            };
            let _ = crate::sip::handlers::interactive_control::post_webhook_event(
                &edge_state_clone,
                &edge_config_clone,
                &event,
            )
            .await;
        });
    }
    edge_state.teardown_call_transaction(transaction_call_id);
    blf_datagrams
}
