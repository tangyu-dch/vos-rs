use sip_core::SipResponse;

use crate::edge_state::EdgeState;

/// 更新网关健康状态：根据响应状态码记录成功/失败，并异步持久化到数据库。
pub(crate) fn update_gateway_health(
    sip_response: &SipResponse,
    edge_state: &EdgeState,
    outcome: &call_core::OutboundResponseOutcome,
    is_invite: bool,
    is_reinvite_response: bool,
) {
    if is_invite && !is_reinvite_response {
        let gateway_id = outcome.gateway_id.clone();
        if !gateway_id.is_empty() {
            if sip_response.status_code >= 200 && sip_response.status_code <= 299 {
                edge_state.gateway_health.record_success(&gateway_id);
            } else if sip_response.status_code == 408
                || (sip_response.status_code >= 500 && sip_response.status_code <= 599)
            {
                edge_state.gateway_health.record_failure(&gateway_id);
            }

            if let (
                true,
                Some(db),
                Some((
                    open,
                    failures,
                    state_str,
                    last_failure_at,
                    half_open_successes,
                    active_calls,
                )),
            ) = (
                edge_state.gateway_health_persistence_enabled,
                edge_state.db_store.clone(),
                edge_state.gateway_health.get_gateway_status(&gateway_id),
            ) {
                let gw = gateway_id.clone();
                let last_failure_at = last_failure_at.map(|st| {
                    let secs = st
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs() as i64;
                    time::OffsetDateTime::from_unix_timestamp(secs)
                        .unwrap_or(time::OffsetDateTime::UNIX_EPOCH)
                });
                tokio::spawn(async move {
                    if let Err(e) = db
                        .save_gateway_health(
                            &gw,
                            open,
                            failures,
                            &state_str,
                            last_failure_at,
                            half_open_successes,
                            None,
                            active_calls,
                        )
                        .await
                    {
                        tracing::warn!(gateway = %gw, error = %e, "无法异步持久化网关健康状态");
                    }
                });
            }
        }
    }
}
