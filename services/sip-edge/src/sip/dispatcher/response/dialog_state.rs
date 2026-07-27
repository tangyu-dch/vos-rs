use std::net::SocketAddr;

use sip_core::SipResponse;

use crate::config::EdgeConfig;
use crate::edge_state::EdgeState;

/// 更新网关侧 dialog 状态：remote tag、peer、route set、local/remote URI、Contact、CSeq。
/// 同时在 1xx 响应时触发 CallRinging webhook。
pub(crate) fn update_gateway_dialog_state(
    sip_response: &SipResponse,
    peer: SocketAddr,
    edge_state: &EdgeState,
    edge_config: &EdgeConfig,
    session_key: Option<&str>,
    call_id: Option<&str>,
    is_invite: bool,
) {
    if let Some(call_id) = call_id {
        if is_invite && session_key.is_some() {
            edge_state.remember_gateway_remote_tag(session_key.unwrap_or(call_id), sip_response);
        }
        if let Some(mut t_mut) = edge_state
            .inbound_transactions
            .get_mut(session_key.unwrap_or(call_id))
        {
            t_mut.dialogs.gateway.peer = Some(peer.to_string());
        }
        if is_invite && sip_response.status_code >= 180 && sip_response.status_code < 300 {
            if let Some(mut t_mut) = edge_state
                .inbound_transactions
                .get_mut(session_key.unwrap_or(call_id))
            {
                let mut route_set = sip_response
                    .headers
                    .get_all("record-route")
                    .map(|value| value.as_str().to_string())
                    .collect::<Vec<_>>();
                route_set.reverse();
                t_mut.dialogs.gateway.route_set = route_set;
                if let Some(local_uri) = sip_response
                    .headers
                    .get("from")
                    .and_then(|value| crate::edge_state::extract_uri_from_contact(value.as_str()))
                {
                    t_mut.dialogs.gateway.local_uri = local_uri;
                }
                if let Some(remote_uri) = sip_response
                    .headers
                    .get("to")
                    .and_then(|value| crate::edge_state::extract_uri_from_contact(value.as_str()))
                {
                    t_mut.dialogs.gateway.remote_uri = remote_uri;
                }
                if let Some(invite_cseq) = sip_response
                    .headers
                    .get("cseq")
                    .and_then(|value| crate::sip::dialog::cseq_number(value.as_str()))
                {
                    t_mut.dialogs.gateway.local_cseq = invite_cseq;
                }
                if let Some(contact_val) = sip_response.headers.get("contact") {
                    if let Some(mut uri) =
                        crate::edge_state::extract_uri_from_contact(contact_val.as_str())
                    {
                        if uri.port.is_none() {
                            uri.port = t_mut.dialogs.gateway.remote_uri.port;
                        }
                        t_mut.dialogs.gateway.remote_target = uri;
                    }
                }
            }
            if sip_response.status_code < 200
                && (edge_config.webhooks.control_mode == "http"
                    || edge_config.webhooks.control_mode == "nats")
            {
                let edge_state_clone = edge_state
                    .self_weak
                    .get()
                    .and_then(|w| w.upgrade())
                    .unwrap();
                let edge_config_clone = edge_config.clone();
                let cid_clone = call_id.to_string();
                let status = sip_response.status_code;
                tokio::spawn(async move {
                    let event = call_core::WebhookEvent {
                        event_id: uuid::Uuid::new_v4().to_string(),
                        schema_version: "1.0".to_string(),
                        call_id: cid_clone,
                        sequence: 2,
                        occurred_at_ms: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as i64,
                        event: call_core::CallEvent::CallRinging {
                            sip_status: status,
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
        }
    }
}
