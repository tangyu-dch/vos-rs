use crate::{config::EdgeConfig, edge_state::EdgeState};
use sip_core::SipUri;
use std::{
    sync::Arc,
    time::{Duration, SystemTime},
};
use tokio::net::UdpSocket;
use tracing::{info, warn};

use crate::sip::outbound;

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
