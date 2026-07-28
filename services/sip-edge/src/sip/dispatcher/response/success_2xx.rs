use std::time::Instant;

use sip_core::SipResponse;
use tracing::debug;

use crate::config::EdgeConfig;
use crate::edge_state::{EdgeState, PendingDatagram};

/// 处理 2xx 成功响应：触发 CallAnswered webhook、记录 established_at、解析 Session-Expires。
///
/// 返回 BLF NOTIFY 数据报（呼叫建立时向订阅者广播 dialog 状态）。
/// 注意：原代码中 `established_at` 在 L783-787 与 L816-823 有重复设置，本函数保留原样不修改。
pub(crate) fn handle_2xx_success(
    sip_response: &SipResponse,
    edge_state: &EdgeState,
    edge_config: &EdgeConfig,
    session_key: Option<&str>,
    call_id: Option<&str>,
) -> Vec<PendingDatagram> {
    if sip_response.status_code >= 200 && sip_response.status_code < 300 {
        if let Some(cid) = call_id {
            let is_invite_local = sip_response
                .headers
                .get("cseq")
                .map(|c| c.as_str().contains("INVITE"))
                .unwrap_or(false);
            if is_invite_local
                && (edge_config.webhooks.control_mode == "http"
                    || edge_config.webhooks.control_mode == "nats")
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
                        call_id: cid_clone.clone(),
                        sequence: 3,
                        occurred_at_ms: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as i64,
                        event: call_core::CallEvent::CallAnswered {
                            sip_status: status,
                            leg: "b_leg".to_string(),
                        },
                    };
                    if let Some(next_inst) =
                        crate::sip::handlers::interactive_control::post_webhook_event(
                            &edge_state_clone,
                            &edge_config_clone,
                            &event,
                        )
                        .await
                    {
                        crate::sip::handlers::interactive_control::execute_instruction(
                            next_inst,
                            cid_clone,
                            edge_state_clone,
                            edge_config_clone,
                        )
                        .await;
                    }
                });
            }
            // BLF: 首次建立呼叫时触发 dialog 状态广播（confirmed）
            let mut blf_datagrams = Vec::new();
            if is_invite_local {
                let was_established = edge_state
                    .inbound_transactions
                    .get(session_key.unwrap_or(cid))
                    .map(|t| t.established_at.is_some())
                    .unwrap_or(false);
                if !was_established {
                    if let Some(transaction) = edge_state
                        .inbound_transactions
                        .get(session_key.unwrap_or(cid))
                    {
                        let caller_aor = transaction.dialogs.caller.remote_uri.to_string();
                        let callee_aor = transaction.dialogs.caller.local_uri.to_string();
                        blf_datagrams =
                            crate::sip::handlers::subscribe::trigger_dialog_state_change(
                                edge_state,
                                edge_config,
                                &caller_aor,
                                &callee_aor,
                                cid,
                                crate::sip::handlers::subscribe::DialogStateChange::Established,
                            );
                    }
                }
            }
            if let Some(mut t_mut) = edge_state
                .inbound_transactions
                .get_mut(session_key.unwrap_or(cid))
            {
                if t_mut.established_at.is_none() {
                    t_mut.established_at = Some(std::time::Instant::now());
                }
            }
            let se_header = sip_response
                .headers
                .get("session-expires")
                .or_else(|| sip_response.headers.get("x"))
                .map(|v| v.as_str().to_string());
            if let Some(se_val) = se_header {
                let mut parts = se_val.splitn(2, ';');
                let secs: Option<u32> = parts.next().and_then(|s| s.trim().parse().ok());
                let refresher = parts
                    .next()
                    .and_then(|p| p.split('=').nth(1).map(|r| r.trim().to_string()))
                    .unwrap_or_else(|| "uac".to_string());
                if let Some(secs) = secs {
                    if let Some(mut t_mut) = edge_state
                        .inbound_transactions
                        .get_mut(session_key.unwrap_or(cid))
                    {
                        t_mut.session_expires = Some(secs);
                        t_mut.session_refresher = Some(refresher);
                        t_mut.last_session_refresh = Some(Instant::now());
                        debug!(
                            call_id = cid,
                            session_expires = secs,
                            "stored Session-Expires from 200 OK"
                        );
                    }
                }
            }
            if let Some(mut t_mut) = edge_state
                .inbound_transactions
                .get_mut(session_key.unwrap_or(cid))
            {
                if t_mut.established_at.is_none() {
                    t_mut.established_at = Some(Instant::now());
                }
            }
            return blf_datagrams;
        }
    }
    Vec::new()
}
