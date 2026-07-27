use sip_core::SipResponse;

use crate::config::EdgeConfig;
use crate::edge_state::{EdgeState, InboundTransaction};

/// 处理呼叫失败状态：清理媒体目标、释放并发资源、触发 CallFinished webhook、移除事务。
#[allow(clippy::too_many_arguments)]
pub(crate) fn handle_failed_state(
    sip_response: &SipResponse,
    edge_state: &EdgeState,
    edge_config: &EdgeConfig,
    transaction: Option<&InboundTransaction>,
    call_id: Option<&str>,
    session_key: Option<&str>,
    outcome: &call_core::OutboundResponseOutcome,
    is_reinvite_response: bool,
) {
    if matches!(outcome.state, call_core::CallState::Failed) {
        if let Some(transaction) = transaction {
            edge_state.clear_media_targets(transaction);
        }
        if !is_reinvite_response {
            if let Some(cid) = call_id {
                let username = edge_state
                    .inbound_transactions
                    .get(session_key.unwrap_or(cid))
                    .and_then(|tx| {
                        tx.original_request.as_ref().and_then(|req| {
                            crate::edge_state::EdgeState::username_from_request(req)
                        })
                    });
                if let Some(ref uname) = username {
                    edge_state.decrement_user_concurrency(uname);
                }
                if !outcome.gateway_id.is_empty() {
                    edge_state
                        .gateway_health
                        .decrement_active(&outcome.gateway_id);
                }
                crate::resource_lease::release(
                    edge_state,
                    &call_core::CallId::new(cid.to_string()),
                );
                if edge_config.webhooks.control_mode == "http"
                    || edge_config.webhooks.control_mode == "nats"
                {
                    let edge_state_clone = edge_state
                        .self_weak
                        .get()
                        .and_then(|w| w.upgrade())
                        .unwrap();
                    let edge_config_clone = edge_config.clone();
                    let cid_clone = cid.to_string();
                    let status = sip_response.status_code;
                    tokio::spawn(async move {
                        let event = call_core::WebhookEvent {
                            event_id: uuid::Uuid::new_v4().to_string(),
                            schema_version: "1.0".to_string(),
                            call_id: cid_clone,
                            sequence: 4,
                            occurred_at_ms: std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_millis() as i64,
                            event: call_core::CallEvent::CallFinished {
                                duration_secs: 0,
                                sip_status: Some(status),
                                q850_cause: Some(16),
                                reason: "Call setup failed".to_string(),
                                leg: "b_leg".to_string(),
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
                if let Some(session_id) = session_key {
                    edge_state.inbound_transactions.remove(session_id);
                }
            }
        }
    }
}
