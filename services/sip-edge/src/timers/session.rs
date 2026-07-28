use crate::{config::EdgeConfig, edge_state::EdgeState};
use sip_core::Method;
use std::{sync::Arc, time::Duration};
use tokio::net::UdpSocket;
use tracing::{info, warn};

use crate::sip::dialog_request;

/// Periodically scans all active transactions and sends BYE to both legs
/// of any call that has exceeded its negotiated Session-Expires timeout.
/// This prevents "zombie calls" from accumulating when media or signalling
/// connectivity is silently lost.
pub(crate) fn spawn_session_timer_watchdog(
    edge_state: Arc<EdgeState>,
    socket: Arc<UdpSocket>,
    edge_config: Arc<EdgeConfig>,
) {
    // Scan interval: every 10 seconds in production, 50ms in tests for speed
    let scan_interval = if cfg!(test) {
        Duration::from_millis(50)
    } else {
        Duration::from_secs(10)
    };

    tokio::spawn(async move {
        let mut interval = tokio::time::interval(scan_interval);
        loop {
            interval.tick().await;

            // 1. Send active refreshes if half of Session-Expires has elapsed
            let refreshes_to_send = {
                let mut tasks = Vec::new();
                for mut entry in edge_state.inbound_transactions.iter_mut() {
                    let tx = entry.value_mut();
                    let Some(expires) = tx.session_expires else {
                        continue;
                    };
                    let Some(last_refresh) = tx.last_session_refresh else {
                        continue;
                    };
                    let elapsed = last_refresh.elapsed().as_secs();
                    let Some(refresher) = tx.session_refresher.as_deref() else {
                        continue;
                    };

                    if elapsed >= u64::from(expires) / 2 {
                        let is_to_gw = refresher == "uac";
                        let is_to_caller = refresher == "uas";

                        if is_to_gw || is_to_caller {
                            tx.last_session_refresh = Some(std::time::Instant::now());
                            let Some(template) = tx.original_request.clone() else {
                                continue;
                            };
                            let dialog = if is_to_gw {
                                &mut tx.dialogs.gateway
                            } else {
                                &mut tx.dialogs.caller
                            };
                            if dialog.call_id.is_empty() || dialog.remote_tag.is_none() {
                                continue;
                            }
                            if let Some(datagram) = dialog_request::build_dialog_request(
                                &template,
                                dialog,
                                Method::Update,
                                &edge_config.advertised_addr,
                                &[],
                            ) {
                                tasks.push(datagram);
                            }
                        }
                    }
                }
                tasks
            };

            for datagram in refreshes_to_send {
                let _ = edge_state
                    .send_sip_datagram(datagram, &socket, &edge_config)
                    .await;
            }

            // 2. Collect expired calls without holding the lock during async I/O
            let expired = {
                let mut calls = Vec::new();
                for mut entry in edge_state.inbound_transactions.iter_mut() {
                    let session_id = entry.key().clone();
                    let tx = entry.value_mut();
                    let reason = if let (Some(established), Some(max_duration)) =
                        (tx.established_at, tx.max_duration_secs)
                    {
                        (max_duration > 0
                            && established.elapsed().as_secs() >= u64::from(max_duration))
                        .then_some("balance exhausted")
                    } else {
                        None
                    }
                    .or_else(|| {
                        let expires = tx.session_expires?;
                        let refreshed = tx.last_session_refresh?;
                        (refreshed.elapsed().as_secs() >= u64::from(expires))
                            .then_some("session timer expired")
                    });
                    let Some(reason) = reason else {
                        continue;
                    };

                    let caller_call_id = tx.dialogs.caller.call_id.clone();
                    let username = tx.original_request.as_ref().and_then(|request| {
                        crate::edge_state::EdgeState::username_from_request(request)
                    });
                    let tenant_ctx = tx.tenant.clone();
                    let datagrams =
                        dialog_request::build_session_byes(tx, &edge_config.advertised_addr);
                    warn!(
                        session_id,
                        caller_call_id, reason, "watchdog terminating B2BUA session"
                    );
                    calls.push((
                        session_id,
                        caller_call_id,
                        username,
                        tenant_ctx,
                        reason.to_string(),
                        datagrams,
                    ));
                }
                calls
            };

            for (session_id, caller_call_id, username, tenant_ctx, reason, datagrams) in expired {
                for datagram in datagrams {
                    let _ = edge_state
                        .send_sip_datagram(datagram, &socket, &edge_config)
                        .await;
                }

                if let Some(ref uname) = username {
                    edge_state.decrement_user_concurrency(uname);
                }
                edge_state.decrement_tenant_concurrency(tenant_ctx.as_ref());
                // Clean up the transaction and call state
                edge_state.teardown_call_transaction(&session_id);
                // Decrement active call count for the gateway before terminating.
                if let Some(gw_id) = edge_state.call_manager.current_gateway_id(&caller_call_id) {
                    edge_state.gateway_health.decrement_active(&gw_id);
                }
                edge_state
                    .call_manager
                    .terminate_call_with_reason(&caller_call_id, &reason);

                crate::billing::settle_completed_call(
                    &edge_state,
                    &call_core::CallId::new(caller_call_id.clone()),
                );

                info!(session_id, caller_call_id, "watchdog terminated call");
            }

            // 2. 异步后台清理过期的 nonce 防重放记录，避免影响鉴权热路径性能
            {
                let now_epoch = std::time::SystemTime::now()
                    .duration_since(std::time::SystemTime::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                edge_state
                    .nonce_replay_cache
                    .retain(|_, &mut exp| exp > now_epoch);

                // 如果防重放缓存过大，强制阶段性驱逐
                const MAX_NONCE_CACHE: usize = 100_000;
                if edge_state.nonce_replay_cache.len() > MAX_NONCE_CACHE {
                    let cutoff = now_epoch + 250;
                    edge_state.nonce_replay_cache.retain(|_, exp| *exp > cutoff);
                }
            }
        }
    });
}
