//! VCI Hangup 命令处理。
//!
//! 拆分为两个独立场景：
//! - parked 呼叫：直接回 SIP 错误响应并释放媒体端口
//! - 已建立通话：构造 BYE 拆线、扣减网关并发、释放资源租约
//!
//! 两者最终都会通过 [`finalize_vci_hangup`] 写入 CDR 并完成计费结算。

use std::sync::Arc;

use tracing::info;

use crate::config::EdgeConfig;
use crate::edge_state::{EdgeState, PendingDatagram};
use crate::sip::{dialog_request, response};

use super::commands::HangupParams;

/// 触发 VCI Hangup 后的统一收尾：结算或释放资源租约。
///
/// 若 [`call_core::CallManager::try_terminate_call_with_reason`] 返回 true，
/// 表示该 CallId 此前未结算过，将进入计费结算流程；否则视为已结算，仅释放
/// 资源租约避免泄漏。这样可避免 Hangup 重复触发 CDR 与扣减。
pub(crate) fn finalize_vci_hangup(edge_state: &EdgeState, call_id: &str, termination_reason: &str) {
    let call_id_value = call_core::CallId::new(call_id.to_string());
    if edge_state
        .call_manager
        .try_terminate_call_with_reason(call_id, termination_reason)
    {
        crate::billing::settle_completed_call(edge_state, &call_id_value);
    } else {
        crate::resource_lease::release(edge_state, &call_id_value);
    }
}

/// 处理 Hangup 命令，依据呼叫当前所处阶段（parked / 已建立）执行相应拆线逻辑。
pub(super) async fn handle_hangup(
    call_id: &str,
    params: HangupParams,
    edge_state: &Arc<EdgeState>,
    edge_config: &EdgeConfig,
    socket: &Arc<tokio::net::UdpSocket>,
) {
    info!(call_id, "VCI Hangup command execution started");

    let sip_cause = params.sip_cause.unwrap_or(603);
    let termination_reason = format!("VCI Hangup ({sip_cause})");

    if let Some((_, parked)) = edge_state.parked_calls.remove(call_id) {
        let code = sip_cause;
        let reason = match code {
            486 => "Busy Here",
            480 => "Temporarily Unavailable",
            488 => "Not Acceptable Here",
            503 => "Service Unavailable",
            _ => "Decline",
        };
        let resp = response::build_response_with_owned_headers(
            &parked.invite_request,
            code,
            reason,
            &[],
            "",
        );
        let dg = PendingDatagram::new(parked.peer_addr.to_string(), resp);
        let _ = edge_state.send_sip_datagram(dg, socket, edge_config).await;
        edge_state
            .media_relay
            .clear_target(parked.caller_relay_port);
        return;
    }

    if let Some(mut tx) = edge_state.teardown_call_transaction(call_id) {
        let caller_call_id = tx.dialogs.caller.call_id.clone();
        let datagrams = dialog_request::build_session_byes(&mut tx, &edge_config.advertised_addr);
        for datagram in datagrams {
            let _ = edge_state
                .send_sip_datagram(datagram, socket, edge_config)
                .await;
        }

        if let Some(gw_id) = edge_state.call_manager.current_gateway_id(&caller_call_id) {
            edge_state.gateway_health.decrement_active(&gw_id);
            let status = edge_state.gateway_health.get_gateway_status(&gw_id);
            crate::timers::persist_gateway_health(edge_state, gw_id, status);
        }

        if let Some(username) = tx
            .original_request
            .as_ref()
            .and_then(|req| EdgeState::username_from_request(req))
        {
            edge_state.decrement_user_concurrency(&username);
        }
        edge_state.decrement_tenant_concurrency(tx.tenant.as_ref());

        // BLF: 呼叫通过管理 API 终止时，广播 dialog terminated 状态
        if tx.established_at.is_some() {
            let caller_aor = tx.dialogs.caller.remote_uri.to_string();
            let callee_aor = tx.dialogs.caller.local_uri.to_string();
            let blf_datagrams = crate::sip::handlers::subscribe::trigger_dialog_state_change(
                edge_state,
                edge_config,
                &caller_aor,
                &callee_aor,
                &caller_call_id,
                crate::sip::handlers::subscribe::DialogStateChange::Terminated,
            );
            for datagram in blf_datagrams {
                let _ = edge_state
                    .send_sip_datagram(datagram, socket, edge_config)
                    .await;
            }
        }

        finalize_vci_hangup(edge_state, &caller_call_id, &termination_reason);
        return;
    }

    finalize_vci_hangup(edge_state, call_id, &termination_reason);
}
