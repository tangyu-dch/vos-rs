use sip_core::SipResponse;
use tracing::{debug, warn};

use crate::config::EdgeConfig;
use crate::edge_state::{EdgeState, PendingDatagram};

/// 处理 self-refresh、网关探测、外呼注册三类预过滤响应。
///
/// 返回 `Some(datagrams)` 表示已处理应早返回；`None` 表示继续主流程。
pub(crate) async fn try_handle_prefilter_response(
    sip_response: &SipResponse,
    edge_state: &EdgeState,
    edge_config: &EdgeConfig,
) -> Option<Vec<PendingDatagram>> {
    let is_self_refresh = sip_response
        .headers
        .get_all("via")
        .any(|v| v.as_str().contains("branch=z9hG4bK-refresh-"));

    if is_self_refresh {
        let call_id = sip_response
            .headers
            .get("call-id")
            .map(|v| v.as_str().to_string());
        if sip_response.status_code >= 200 && sip_response.status_code < 300 {
            if let Some(ref cid) = call_id {
                let session_id = edge_state
                    .inbound_transactions
                    .get(cid)
                    .map(|transaction| transaction.session_id.clone());
                if let Some(session_id) = session_id {
                    if let Some(mut transaction) =
                        edge_state.inbound_transactions.get_mut(&session_id)
                    {
                        transaction.last_session_refresh = Some(std::time::Instant::now());
                        debug!(
                            session_id,
                            "received 200 OK for self-generated session refresh"
                        );
                    }
                }
            }
        } else if sip_response.status_code >= 300 {
            warn!(
                call_id = ?call_id,
                status = sip_response.status_code,
                "self-generated session refresh request failed"
            );
        }
        return Some(Vec::new());
    }

    let call_id = sip_response
        .headers
        .get("call-id")
        .map(|call_id| call_id.as_str().to_string());

    if let Some(ref probe_call_id) = call_id {
        if sip_response.status_code >= 200 {
            if let Some((_, gateway_id)) = edge_state.gateway_probes.remove(probe_call_id) {
                if sip_response.status_code < 300 {
                    crate::timers::record_probe_success(edge_state, &gateway_id);
                } else {
                    crate::timers::record_probe_failure(
                        edge_state,
                        &gateway_id,
                        format!("OPTIONS returned {}", sip_response.status_code),
                    );
                }
                return Some(Vec::new());
            }
        }
    }

    if let Some(ref reg_call_id) = call_id {
        let is_outbound_reg = edge_state
            .outbound_registrations
            .iter()
            .any(|entry| entry.value().call_id == *reg_call_id);
        if is_outbound_reg {
            return Some(crate::sip::outbound_reg::handle_outbound_register_response(
                edge_state,
                edge_config,
                sip_response,
                reg_call_id,
            ));
        }
    }

    None
}
