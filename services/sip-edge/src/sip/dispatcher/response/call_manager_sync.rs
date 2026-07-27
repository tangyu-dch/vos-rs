use sip_core::{HeaderName, HeaderValue, SipResponse};
use tracing::{debug, warn};

use crate::edge_state::EdgeState;

/// 将响应同步到 call_manager，返回 `OutboundResponseOutcome`。
///
/// 返回 `Some(outcome)` 表示继续主流程；`None` 表示 call_manager 报错且需早返回 `Vec::new()`。
///
/// The call manager still owns the logical caller-side call state. Give it a private
/// projection of the response; never rewrite the wire B-leg response in place.
pub(crate) async fn sync_to_call_manager(
    sip_response: &SipResponse,
    edge_state: &EdgeState,
    call_id: Option<&str>,
    is_invite: bool,
    is_reinvite_response: bool,
) -> Option<call_core::OutboundResponseOutcome> {
    let mut call_state_response = sip_response.clone();
    if let Some(caller_call_id) = call_id {
        if let Ok(name) = HeaderName::new("call-id") {
            call_state_response
                .headers
                .replace(name, HeaderValue::new_owned(caller_call_id.to_string()));
        }
    }

    if is_invite && !is_reinvite_response {
        match edge_state
            .call_manager
            .handle_outbound_response(&call_state_response)
        {
            Ok(outcome) => Some(outcome),
            Err(error) => {
                if sip_response.status_code >= 180 && sip_response.status_code < 300 {
                    debug!(%error, status = sip_response.status_code, "response arrived when call state machine not in active state, forwarding anyway");
                } else {
                    warn!(%error, "failed to apply outbound SIP response");
                    return None;
                }
                Some(call_core::OutboundResponseOutcome {
                    call_id: call_core::CallId::new(call_id.unwrap_or_default().to_string()),
                    state: call_core::CallState::Established,
                    failover_uri: None,
                    gateway_id: String::new(),
                    failover_gateway_id: None,
                    caller_identity: None,
                })
            }
        }
    } else {
        Some(call_core::OutboundResponseOutcome {
            call_id: call_core::CallId::new(call_id.unwrap_or_default().to_string()),
            state: call_core::CallState::Established,
            failover_uri: None,
            gateway_id: String::new(),
            failover_gateway_id: None,
            caller_identity: None,
        })
    }
}
