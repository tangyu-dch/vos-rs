use crate::{config::EdgeConfig, edge_state::EdgeState};
use call_core::CallQualityMetrics;
use sip_core::{Method, SipUri};
use std::{
    sync::Arc,
    time::{Duration, SystemTime},
};
use tokio::net::UdpSocket;
use tracing::{info, warn};

use crate::media;
use crate::sip::{dialog_request, outbound};

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
                        reason.to_string(),
                        datagrams,
                    ));
                }
                calls
            };

            for (session_id, caller_call_id, username, reason, datagrams) in expired {
                for datagram in datagrams {
                    let _ = edge_state
                        .send_sip_datagram(datagram, &socket, &edge_config)
                        .await;
                }

                if let Some(ref uname) = username {
                    edge_state.decrement_user_concurrency(uname);
                }
                // Clean up the transaction and call state
                edge_state.teardown_call_transaction(&session_id);
                // Decrement active call count for the gateway before terminating.
                if let Some(gw_id) = edge_state.call_manager.current_gateway_id(&caller_call_id) {
                    edge_state.gateway_health.decrement_active(&gw_id);
                }
                edge_state
                    .call_manager
                    .terminate_call_with_reason(&caller_call_id, &reason);

                crate::billing_settlement::settle_completed_call(
                    &edge_state,
                    &call_core::CallId::new(caller_call_id.clone()),
                );

                info!(session_id, caller_call_id, "watchdog terminated call");
            }

            // 2. 异步后台清理过期的 nonce 防重放记录，避免影响鉴权热路径性能
            {
                let now_epoch = SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
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

pub(crate) fn spawn_nat_keepalive_loop(edge_state: Arc<EdgeState>, socket: Arc<UdpSocket>) {
    let scan_interval = if cfg!(test) {
        Duration::from_millis(50)
    } else {
        Duration::from_secs(30)
    };

    tokio::spawn(async move {
        let mut interval = tokio::time::interval(scan_interval);
        interval.tick().await;

        loop {
            interval.tick().await;

            let addrs = {
                let registrar = edge_state.registrar.read().await;
                registrar
                    .get_all_active_received_from(SystemTime::now(), None)
                    .await
            };

            for addr in addrs {
                edge_state.send_keepalive_probe(&addr, &socket).await;
            }
        }
    });
}

/// Periodically probes configured gateways with SIP OPTIONS.
pub(crate) fn spawn_gateway_health_probe_loop(
    edge_state: Arc<EdgeState>,
    socket: Arc<UdpSocket>,
    edge_config: Arc<EdgeConfig>,
) {
    let interval_duration = if cfg!(test) {
        Duration::from_millis(100)
    } else {
        Duration::from_secs(10)
    };

    tokio::spawn(async move {
        let mut interval = tokio::time::interval(interval_duration);
        interval.tick().await;

        loop {
            interval.tick().await;
            if !edge_config.gateway_health_checks_enabled {
                continue;
            }
            let Some(db) = edge_state.db_store.clone() else {
                continue;
            };

            let gateways = match db.load_gateways().await {
                Ok(gateways) => gateways,
                Err(error) => {
                    warn!(%error, "failed to load gateways for OPTIONS health probing");
                    continue;
                }
            };

            for (
                gateway_id,
                host,
                port,
                _transport,
                _capacity,
                _caller_mode,
                _virtual_caller,
                _prefix_rules,
                _max_concurrent,
                _role,
            ) in gateways
            {
                if edge_state
                    .gateway_probes
                    .iter()
                    .any(|entry| entry.value() == &gateway_id)
                {
                    continue;
                }

                let can_probe = edge_state.gateway_health.try_acquire_probe(&gateway_id);
                if !can_probe {
                    continue;
                }

                let uri = SipUri {
                    secure: false,
                    user: Some("health-check".to_string().into()),
                    host: host.into(),
                    port,
                    params: Vec::new(),
                };
                let target = outbound::target_addr_for(&uri);
                let call_id = format!("health-probe-{gateway_id}-{}", chrono_like_epoch_millis());
                let bytes = outbound::build_gateway_options(
                    &uri,
                    &edge_config.advertised_addr,
                    &call_id,
                    1,
                );

                edge_state
                    .gateway_probes
                    .insert(call_id.clone(), gateway_id.clone());
                if let Err(error) = socket.send_to(&bytes, &target).await {
                    edge_state.gateway_probes.remove(&call_id);
                    record_probe_failure(&edge_state, &gateway_id, error.to_string());
                    continue;
                }

                let state = Arc::clone(&edge_state);
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_secs(3)).await;
                    if state.gateway_probes.remove(&call_id).is_some() {
                        record_probe_failure(
                            &state,
                            &gateway_id,
                            "OPTIONS probe timeout".to_string(),
                        );
                    }
                });
            }
        }
    });
}

fn chrono_like_epoch_millis() -> u128 {
    SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis())
}

pub(crate) fn record_probe_failure(edge_state: &EdgeState, gateway_id: &str, reason: String) {
    let prev_state = edge_state.gateway_health.circuit_state(gateway_id);
    edge_state.gateway_health.record_failure(gateway_id);
    let status = edge_state.gateway_health.get_gateway_status(gateway_id);
    let consecutive_failures = status.as_ref().map(|s| s.1).unwrap_or(0);
    let current_state_str = status.as_ref().map(|s| s.2.as_str()).unwrap_or("unknown");
    warn!(
        gateway = gateway_id,
        %reason,
        prev_state = ?prev_state,
        current_state = current_state_str,
        consecutive_failures,
        "gateway OPTIONS health probe failed"
    );
    if let Some(status) = status {
        persist_gateway_health(edge_state, gateway_id.to_string(), Some(status));
    }
}

pub(crate) fn record_probe_success(edge_state: &EdgeState, gateway_id: &str) {
    let prev_state = edge_state.gateway_health.circuit_state(gateway_id);
    edge_state.gateway_health.record_probe_success(gateway_id);
    let status = edge_state.gateway_health.get_gateway_status(gateway_id);
    let current_state_str = status.as_ref().map(|s| s.2.as_str()).unwrap_or("unknown");
    info!(
        gateway = gateway_id,
        prev_state = ?prev_state,
        current_state = current_state_str,
        "gateway OPTIONS health probe succeeded — circuit state reset to Closed and failure count cleared"
    );
    persist_gateway_health(edge_state, gateway_id.to_string(), status);
}

pub(crate) fn persist_gateway_health(
    edge_state: &EdgeState,
    gateway_id: String,
    status: Option<(bool, i32, String, Option<std::time::SystemTime>, i32, i32)>,
) {
    if !edge_state.gateway_health_persistence_enabled {
        return;
    }
    let Some((
        circuit_open,
        failures,
        state_str,
        last_failure_sys,
        half_open_successes,
        active_calls,
    )) = status
    else {
        return;
    };
    let Some(db) = edge_state.db_store.clone() else {
        return;
    };
    let last_failure_at = last_failure_sys.map(|st| {
        let secs = st
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        time::OffsetDateTime::from_unix_timestamp(secs).unwrap_or(time::OffsetDateTime::UNIX_EPOCH)
    });
    tokio::spawn(async move {
        if let Err(error) = db
            .save_gateway_health(
                &gateway_id,
                circuit_open,
                failures,
                &state_str,
                last_failure_at,
                half_open_successes,
                None,
                active_calls,
            )
            .await
        {
            warn!(gateway = %gateway_id, %error, "failed to persist gateway probe health");
        }
    });
}

pub(crate) fn calculate_mos_for_legs(
    caller_rtcp: Option<&media::RtcpQualitySnapshot>,
    gateway_rtcp: Option<&media::RtcpQualitySnapshot>,
) -> CallQualityMetrics {
    let mut metrics = CallQualityMetrics::default();

    let (caller_rtt, caller_loss, _caller_jitter) = if let Some(rtcp) = caller_rtcp {
        let rtt = rtcp.max_rtt_ms.or(rtcp.last_rtt_ms);
        let loss = rtcp
            .max_fraction_lost
            .or(rtcp.last_fraction_lost)
            .map(|f| (f64::from(f)) / 256.0 * 100.0);
        let jitter = rtcp
            .max_jitter
            .or(rtcp.last_jitter)
            .map(|j| (f64::from(j)) / 8.0);

        metrics.caller_rtt_ms = rtt;
        metrics.caller_loss_rate = loss;
        metrics.caller_jitter_ms = jitter;

        (rtt.unwrap_or(0), loss.unwrap_or(0.0), jitter.unwrap_or(0.0))
    } else {
        (0, 0.0, 0.0)
    };

    let (gateway_rtt, gateway_loss, _gateway_jitter) = if let Some(rtcp) = gateway_rtcp {
        let rtt = rtcp.max_rtt_ms.or(rtcp.last_rtt_ms);
        let loss = rtcp
            .max_fraction_lost
            .or(rtcp.last_fraction_lost)
            .map(|f| (f64::from(f)) / 256.0 * 100.0);
        let jitter = rtcp
            .max_jitter
            .or(rtcp.last_jitter)
            .map(|j| (f64::from(j)) / 8.0);

        metrics.gateway_rtt_ms = rtt;
        metrics.gateway_loss_rate = loss;
        metrics.gateway_jitter_ms = jitter;

        (rtt.unwrap_or(0), loss.unwrap_or(0.0), jitter.unwrap_or(0.0))
    } else {
        (0, 0.0, 0.0)
    };

    if caller_rtcp.is_none() && gateway_rtcp.is_none() {
        return metrics;
    }

    let d_caller = (f64::from(caller_rtt)) / 2.0;
    let d_gateway = (f64::from(gateway_rtt)) / 2.0;
    let d_total = d_caller + d_gateway;

    let i_d = if d_total < 177.3 {
        0.024 * d_total
    } else {
        0.024 * d_total + 0.11 * (d_total - 177.3)
    };

    let i_e_caller = 95.0 * (caller_loss / (caller_loss + 4.3));
    let i_e_gateway = 95.0 * (gateway_loss / (gateway_loss + 4.3));
    let i_e = i_e_caller + i_e_gateway;

    let r_factor = 93.2 - i_d - i_e;
    let r_factor = r_factor.clamp(0.0, 93.2);

    let mos = 1.0 + 0.035 * r_factor + 0.000007 * r_factor * (r_factor - 60.0) * (100.0 - r_factor);
    let mos = mos.clamp(1.0, 4.5);

    metrics.mos = Some(mos);
    metrics
}
