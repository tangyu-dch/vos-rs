use sip_core::SipResponse;
use tracing::debug;

use crate::edge_state::EdgeState;

/// 清理临时 MESSAGE 事务：当 MESSAGE 请求收到最终响应后，从 inbound_transactions 中移除。
pub(crate) fn cleanup_message_transaction(
    sip_response: &SipResponse,
    edge_state: &EdgeState,
    session_key: Option<&str>,
) {
    let is_message = sip_response
        .headers
        .get("cseq")
        .map(|cseq| cseq.as_str().contains("MESSAGE"))
        .unwrap_or(false);
    if is_message && sip_response.status_code >= 200 {
        if let Some(session_id) = session_key {
            edge_state.inbound_transactions.remove(session_id);
            debug!(session_id, "cleaned up temporary MESSAGE transaction");
        }
    }
}
