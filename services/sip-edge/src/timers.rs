use crate::sip::transaction::ClientTransactionKey;
use crate::{
    config::EdgeConfig,
    edge_state::{
        extract_uri_from_contact, parse_target_addr_from_route, sip_uri_from_peer, EdgeState,
        PendingDatagram,
    },
};
use call_core::{CallQualityMetrics, GatewayHealthTracker};
use std::{
    net::SocketAddr,
    str::FromStr,
    sync::Arc,
    time::{Duration, Instant, SystemTime},
};
use tokio::net::UdpSocket;
use tracing::{debug, info, warn};

use crate::handle_datagram;
use crate::media;
use crate::sip::outbound;

pub(crate) fn spawn_client_transaction_retransmission(
    edge_state: Arc<EdgeState>,
    socket: Arc<UdpSocket>,
    target: String,
    bytes: Vec<u8>,
    key: ClientTransactionKey,
    edge_config: Arc<EdgeConfig>,
) {
    let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel::<()>();

    let key_clone = key.clone();
    tokio::spawn(async move {
        edge_state.client_transactions.insert(key_clone, cancel_tx);

        let is_invite = key.method == "INVITE";
        // To make unit tests faster, we can scale down the initial T1 timer in tests.
        // But for production, default is 500ms.
        let mut t1 = if cfg!(test) {
            Duration::from_millis(5)
        } else {
            Duration::from_millis(500)
        };
        let max_time = if cfg!(test) {
            Duration::from_millis(50)
        } else {
            Duration::from_secs(32)
        };
        let start_time = Instant::now();

        let mut cancel_rx = cancel_rx;
        let mut completed = false;

        loop {
            tokio::select! {
                _ = &mut cancel_rx => {
                    completed = true;
                    break;
                }
                _ = tokio::time::sleep(t1) => {
                    let elapsed = start_time.elapsed();
                    if elapsed >= max_time {
                        break;
                    }
                    if let Err(error) = socket.send_to(&bytes, &target).await {
                        warn!(%error, ?key, "failed to retransmit client transaction request");
                    } else {
                        debug!(?key, elapsed = ?elapsed, "retransmitted client transaction request");
                    }
                    if is_invite {
                        t1 *= 2;
                    } else {
                        t1 = std::cmp::min(t1 * 2, Duration::from_secs(4));
                    }
                }
            }
        }

        edge_state.client_transactions.remove(&key);

        if !completed {
            warn!(?key, "client transaction timed out without response");
            if key.method == "INVITE" || key.method == "BYE" {
                let local_503 = format!(
                    "SIP/2.0 503 Service Unavailable\r\n\
                     Via: SIP/2.0/UDP {target};branch={branch}\r\n\
                     From: local;tag=timeout\r\n\
                     To: local;tag=timeout\r\n\
                     Call-ID: {call_id}\r\n\
                     CSeq: 1 {method}\r\n\
                     Content-Length: 0\r\n\r\n",
                    target = target,
                    branch = key.branch,
                    call_id = key.call_id,
                    method = key.method
                );

                let target_addr: SocketAddr = target
                    .parse()
                    .unwrap_or_else(|_| "127.0.0.1:5060".parse().unwrap());
                let _ = handle_datagram(
                    local_503.as_bytes(),
                    target_addr,
                    &*edge_state,
                    &*edge_config,
                )
                .await;
            }
        }
    });
}

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
                    let call_id = entry.key().clone();
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

                            let next_cseq = if is_to_gw {
                                let c = tx.last_outbound_cseq.unwrap_or(1) + 1;
                                tx.last_outbound_cseq = Some(c);
                                c
                            } else {
                                let c = tx.last_inbound_cseq.unwrap_or(1) + 1;
                                tx.last_inbound_cseq = Some(c);
                                c
                            };

                            let (target_addr, req_uri, from_hdr, to_hdr, route_set) = if is_to_gw {
                                let target = if !tx.outbound_route_set.is_empty() {
                                    outbound::target_addr_for_str(&tx.outbound_route_set[0])
                                } else {
                                    outbound::target_addr_for(&tx.outbound_uri)
                                };
                                let uri = tx
                                    .callee_contact
                                    .as_ref()
                                    .map(|u| u.to_string())
                                    .unwrap_or_else(|| tx.outbound_uri.to_string());
                                let from = tx
                                    .original_request
                                    .as_ref()
                                    .and_then(|r| r.headers.get("from"))
                                    .map(|v| v.as_str().to_string())
                                    .unwrap_or_default();
                                let to = format!(
                                    "{};tag={}",
                                    tx.original_request
                                        .as_ref()
                                        .and_then(|r| r.headers.get("to"))
                                        .map(|v| v.as_str())
                                        .unwrap_or_default(),
                                    tx.inbound_to_tag.as_deref().unwrap_or("")
                                );
                                (target, uri, from, to, tx.outbound_route_set.clone())
                            } else {
                                let target = if !tx.inbound_route_set.is_empty() {
                                    parse_target_addr_from_route(&tx.inbound_route_set[0])
                                        .unwrap_or_else(|| tx.peer.clone())
                                } else {
                                    tx.peer.clone()
                                };
                                let uri = tx
                                    .caller_contact
                                    .as_ref()
                                    .map(|u| u.to_string())
                                    .unwrap_or_else(|| sip_uri_from_peer(&tx.peer).to_string());
                                let from = format!(
                                    "{};tag={}",
                                    tx.original_request
                                        .as_ref()
                                        .and_then(|r| r.headers.get("to"))
                                        .map(|v| v.as_str())
                                        .unwrap_or_default(),
                                    tx.inbound_to_tag.as_deref().unwrap_or("")
                                );
                                let to = tx
                                    .original_request
                                    .as_ref()
                                    .and_then(|r| r.headers.get("from"))
                                    .map(|v| v.as_str().to_string())
                                    .unwrap_or_default();
                                (target, uri, from, to, tx.inbound_route_set.clone())
                            };

                            let route_headers = route_set
                                .iter()
                                .map(|r| format!("Route: {r}\r\n"))
                                .collect::<Vec<_>>()
                                .join("");
                            let branch = format!("z9hG4bK-refresh-{}-{}", is_to_gw, next_cseq);

                            let update_req = format!(
                                "UPDATE {req_uri} SIP/2.0\r\n\
                                 Via: SIP/2.0/UDP {addr};branch={branch}\r\n\
                                 Max-Forwards: 70\r\n\
                                 From: {from_hdr}\r\n\
                                 To: {to_hdr}\r\n\
                                 Call-ID: {call_id}\r\n\
                                 CSeq: {next_cseq} UPDATE\r\n\
                                 Supported: timer\r\n\
                                 Session-Expires: {expires};refresher={refresher}\r\n\
                                 {route_headers}\
                                 Content-Length: 0\r\n\r\n",
                                req_uri = req_uri,
                                addr = edge_config.advertised_addr,
                                branch = branch,
                                from_hdr = from_hdr,
                                to_hdr = to_hdr,
                                call_id = call_id,
                                next_cseq = next_cseq,
                                expires = expires,
                                refresher = refresher,
                                route_headers = route_headers
                            );

                            tasks.push((target_addr, update_req.into_bytes()));
                        }
                    }
                }
                tasks
            };

            for (target_addr, bytes) in refreshes_to_send {
                let _ = edge_state
                    .send_sip_datagram(
                        PendingDatagram::new(target_addr, bytes),
                        &socket,
                        &*edge_config,
                    )
                    .await;
            }

            // 2. Collect expired calls without holding the lock during async I/O
            let expired: Vec<(String, String, String)> = {
                edge_state
                    .inbound_transactions
                    .iter()
                    .filter_map(|entry| {
                        let call_id = entry.key().clone();
                        let tx = entry.value();
                        let expires = tx.session_expires?;
                        let last_refresh = tx.last_session_refresh?;
                        let elapsed = last_refresh.elapsed().as_secs();
                        if elapsed >= u64::from(expires) {
                            warn!(
                                call_id,
                                elapsed,
                                session_expires = expires,
                                "session timer expired — sending BYE to both legs"
                            );
                            Some((
                                call_id.clone(),
                                tx.peer.clone(),
                                tx.outbound_uri.to_string(),
                            ))
                        } else {
                            None
                        }
                    })
                    .collect()
            };

            for (call_id, caller_peer, gateway_uri) in expired {
                // Build a BYE toward the caller
                let caller_bye = format!(
                    "BYE sip:{caller} SIP/2.0\r\n\
                     Via: SIP/2.0/UDP {addr};branch=z9hG4bK-watchdog-{call_id}\r\n\
                     Max-Forwards: 70\r\n\
                     From: <sip:watchdog@{addr}>;tag=watchdog\r\n\
                     To: <sip:{caller}>\r\n\
                     Call-ID: {call_id}\r\n\
                     CSeq: 9 BYE\r\n\
                     Content-Length: 0\r\n\r\n",
                    caller = caller_peer,
                    addr = edge_config.advertised_addr,
                    call_id = call_id
                );
                let _ = edge_state
                    .send_sip_datagram(
                        PendingDatagram::new(caller_peer, caller_bye.into_bytes()),
                        &socket,
                        &*edge_config,
                    )
                    .await;

                // Build a BYE toward the gateway
                let gw_bye = format!(
                    "BYE {gw_uri} SIP/2.0\r\n\
                     Via: SIP/2.0/UDP {addr};branch=z9hG4bK-watchdog-gw-{call_id}\r\n\
                     Max-Forwards: 70\r\n\
                     From: <sip:watchdog@{addr}>;tag=watchdog\r\n\
                     To: <{gw_uri}>\r\n\
                     Call-ID: {call_id}\r\n\
                     CSeq: 9 BYE\r\n\
                     Content-Length: 0\r\n\r\n",
                    gw_uri = gateway_uri,
                    addr = edge_config.advertised_addr,
                    call_id = call_id
                );
                let _ = edge_state
                    .send_sip_datagram(
                        PendingDatagram::new(
                            outbound::target_addr_for_str(&gateway_uri),
                            gw_bye.into_bytes(),
                        ),
                        &socket,
                        &*edge_config,
                    )
                    .await;

                // Clean up the transaction and call state
                edge_state.inbound_transactions.remove(&call_id);
                edge_state.call_manager.terminate_call(&call_id);
                info!(call_id, "session-expired call terminated by watchdog");
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
                let registrar = edge_state.registrar.lock().await;
                registrar
                    .get_all_active_received_from(SystemTime::now(), edge_state.db_store.as_ref())
                    .await
            };

            for addr in addrs {
                edge_state.send_keepalive_probe(&addr, &socket).await;
            }
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
