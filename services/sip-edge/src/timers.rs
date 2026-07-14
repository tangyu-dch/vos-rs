use crate::sip::transaction::ClientTransactionKey;
use crate::{
    config::EdgeConfig,
    edge_state::{parse_target_addr_from_route, sip_uri_from_peer, EdgeState, PendingDatagram},
};
use call_core::CallQualityMetrics;
use sip_core::SipUri;
use std::{
    net::SocketAddr,
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
    edge_state.client_transactions.insert(key_clone, cancel_tx);
    tokio::spawn(async move {
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
                let _ =
                    handle_datagram(local_503.as_bytes(), target_addr, &edge_state, &edge_config)
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
                        &edge_config,
                    )
                    .await;
            }

            // 2. Collect expired calls without holding the lock during async I/O
            let expired: Vec<(String, String, String, String)> = {
                edge_state
                    .inbound_transactions
                    .iter()
                    .filter_map(|entry| {
                        let call_id = entry.key().clone();
                        let tx = entry.value();

                        // Check 1: Balance exhaustion
                        if let (Some(est), Some(max_dur)) =
                            (tx.established_at, tx.max_duration_secs)
                        {
                            if max_dur > 0 && est.elapsed().as_secs() >= u64::from(max_dur) {
                                warn!(
                                    call_id,
                                    max_duration = max_dur,
                                    "real-time balance exhausted — sending BYE to both legs"
                                );
                                return Some((
                                    call_id.clone(),
                                    tx.peer.clone(),
                                    tx.outbound_uri.to_string(),
                                    "balance exhausted".to_string(),
                                ));
                            }
                        }

                        // Check 2: Session Timer expiration
                        if let (Some(expires), Some(last_refresh)) =
                            (tx.session_expires, tx.last_session_refresh)
                        {
                            let elapsed = last_refresh.elapsed().as_secs();
                            if elapsed >= u64::from(expires) {
                                warn!(
                                    call_id,
                                    elapsed,
                                    session_expires = expires,
                                    "session timer expired — sending BYE to both legs"
                                );
                                return Some((
                                    call_id.clone(),
                                    tx.peer.clone(),
                                    tx.outbound_uri.to_string(),
                                    "session timer expired".to_string(),
                                ));
                            }
                        }

                        None
                    })
                    .collect()
            };

            for (call_id, caller_peer, gateway_uri, reason) in expired {
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
                        &edge_config,
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
                        &edge_config,
                    )
                    .await;

                // 清理事务前先递减用户并发计数
                let username = edge_state
                    .inbound_transactions
                    .get(&call_id)
                    .and_then(|tx| {
                        tx.original_request.as_ref().and_then(|req| {
                            crate::edge_state::EdgeState::username_from_request(req)
                        })
                    });
                if let Some(ref uname) = username {
                    edge_state.decrement_user_concurrency(uname);
                }
                // Clean up the transaction and call state
                edge_state.inbound_transactions.remove(&call_id);
                // Decrement active call count for the gateway before terminating.
                if let Some(gw_id) = edge_state.call_manager.current_gateway_id(&call_id) {
                    edge_state
                        .gateway_health
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .decrement_active(&gw_id);
                }
                edge_state
                    .call_manager
                    .terminate_call_with_reason(&call_id, &reason);

                // Real-time billing: settle the call on timeout.
                if let (true, Some(db)) = (
                    edge_state.billing_settlement_enabled,
                    edge_state.db_store.as_ref(),
                ) {
                    if let Some(call) = edge_state
                        .call_manager
                        .get(&call_core::CallId::new(call_id.clone()))
                    {
                        let caller_user = call.caller.as_deref().and_then(|s| {
                            let idx = s.find("sip:")?;
                            let rest = &s[idx + 4..];
                            let end = rest.find(['@', ';', '>']).unwrap_or(rest.len());
                            if end == 0 {
                                None
                            } else {
                                Some(&rest[..end])
                            }
                        });
                        let callee = call.inbound.remote_uri.user.as_deref().unwrap_or("");
                        let duration_ms = call
                            .ended_at
                            .and_then(|e| e.duration_since(call.started_at).ok())
                            .map(|d| d.as_millis() as i64)
                            .unwrap_or(0);
                        if let Some(user) = caller_user {
                            let db = db.clone();
                            let user = user.to_string();
                            let callee = callee.to_string();
                            let cid = call_id.clone();
                            tokio::spawn(async move {
                                if let Err(e) =
                                    db.settle_call(&cid, &user, &callee, duration_ms).await
                                {
                                    tracing::warn!(call_id = %cid, error = %e, "timeout settlement failed");
                                }
                            });
                        }
                    }
                }

                info!(call_id, "session-expired call terminated by watchdog");
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
                    .get_all_active_received_from(SystemTime::now(), edge_state.db_store.as_ref())
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
            ) in gateways
            {
                if edge_state
                    .gateway_probes
                    .iter()
                    .any(|entry| entry.value() == &gateway_id)
                {
                    continue;
                }

                let can_probe = edge_state
                    .gateway_health
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .try_acquire_probe(&gateway_id);
                if !can_probe {
                    continue;
                }

                let uri = SipUri {
                    secure: false,
                    user: Some("health-check".to_string()),
                    host,
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
    let status = {
        let health = edge_state
            .gateway_health
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        health.get_gateway_status(gateway_id)
    };
    warn!(gateway = gateway_id, %reason, "gateway OPTIONS health probe failed");
    if let Some(status) = status {
        persist_gateway_health(edge_state, gateway_id.to_string(), Some(status));
    }
}

pub(crate) fn record_probe_success(edge_state: &EdgeState, gateway_id: &str) {
    let mut health = edge_state
        .gateway_health
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    health.record_success(gateway_id);
    let status = health.get_gateway_status(gateway_id);
    drop(health);
    info!(
        gateway = gateway_id,
        "gateway OPTIONS health probe succeeded"
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

pub(crate) struct BillingWatchdogContext {
    pub(crate) db: Arc<cdr_core::PostgresCdrStore>,
    pub(crate) socket: Arc<UdpSocket>,
    pub(crate) advertised_addr: String,
    pub(crate) transactions: dashmap::DashMap<String, crate::edge_state::InboundTransaction>,
    pub(crate) call_manager: Arc<call_core::CallManager>,
}

/// 接通后根据账户余额计算允许时长，并在余额耗尽时强制拆线。
pub(crate) fn spawn_billing_watchdog_simple(
    call_id: String,
    caller_user: String,
    callee_num: String,
    context: BillingWatchdogContext,
) {
    let BillingWatchdogContext {
        db,
        socket,
        advertised_addr,
        transactions,
        call_manager,
    } = context;
    tokio::spawn(async move {
        // ── Step 1: 查询余额与费率 ──────────────────────────────────────────
        let (balance, rate) = match db.check_balance(&caller_user, &callee_num).await {
            Ok((_, balance, rate)) => (balance, rate),
            Err(e) => {
                warn!(
                    call_id = %call_id,
                    error = %e,
                    "billing watchdog: DB error, watchdog disabled"
                );
                return;
            }
        };

        // 费率为 0 → 免费呼叫，无需守护
        if rate <= 0.0 {
            return;
        }

        // ── Step 2: 计算最大通话时长并等待 ────────────────────────────────
        let max_duration_ms = ((balance / rate) * 60_000.0) as u64;

        if max_duration_ms >= 1_000 {
            info!(
                call_id = %call_id,
                caller = %caller_user,
                balance,
                rate,
                max_duration_secs = max_duration_ms / 1000,
                "billing watchdog: hard cutoff scheduled"
            );
            tokio::time::sleep(Duration::from_millis(max_duration_ms)).await;

            // 若通话已自然结束则退出
            if !transactions.contains_key(&call_id) {
                return;
            }
        }

        warn!(
            call_id = %call_id,
            caller = %caller_user,
            balance,
            rate,
            "billing watchdog: balance exhausted, terminating call"
        );

        // ── Step 3: 构建并发送 BYE ─────────────────────────────────────────
        let (caller_bye, gateway_bye) = {
            let Some(tx) = transactions.get(&call_id) else {
                return;
            };

            let caller_target = tx.peer.clone();
            let gateway_uri = tx.outbound_uri.to_string();
            let from_hdr = tx
                .original_request
                .as_ref()
                .and_then(|r| r.headers.get("from"))
                .map(|v| v.as_str().to_string())
                .unwrap_or_default();
            let to_hdr = format!(
                "{};tag={}",
                tx.original_request
                    .as_ref()
                    .and_then(|r| r.headers.get("to"))
                    .map(|v| v.as_str())
                    .unwrap_or_default(),
                tx.inbound_to_tag.as_deref().unwrap_or("")
            );

            let caller_bye_bytes = format!(
                "BYE sip:{caller_target} SIP/2.0\r\n\
                 Via: SIP/2.0/UDP {addr};branch=z9hG4bK-billing-caller\r\n\
                 Max-Forwards: 70\r\n\
                 From: {from_hdr}\r\n\
                 To: {to_hdr}\r\n\
                 Call-ID: {call_id}\r\n\
                 CSeq: 9 BYE\r\n\
                 Reason: SIP;cause=402;text=\"Balance Exhausted\"\r\n\
                 Content-Length: 0\r\n\r\n",
                addr = advertised_addr,
            )
            .into_bytes();

            let gw_target = outbound::target_addr_for_str(&gateway_uri);
            let gateway_bye_bytes = format!(
                "BYE {gateway_uri} SIP/2.0\r\n\
                 Via: SIP/2.0/UDP {addr};branch=z9hG4bK-billing-gw\r\n\
                 Max-Forwards: 70\r\n\
                 From: {to_hdr}\r\n\
                 To: {from_hdr}\r\n\
                 Call-ID: {call_id}\r\n\
                 CSeq: 9 BYE\r\n\
                 Reason: SIP;cause=402;text=\"Balance Exhausted\"\r\n\
                 Content-Length: 0\r\n\r\n",
                addr = advertised_addr,
            )
            .into_bytes();

            ((caller_target, caller_bye_bytes), (gw_target, gateway_bye_bytes))
        };

        // 向主叫发 BYE
        let _ = socket
            .send_to(&caller_bye.1, &caller_bye.0)
            .await;

        // 向网关发 BYE
        if let Ok(gw_addr) = gateway_bye.0.parse::<SocketAddr>() {
            let _ = socket.send_to(&gateway_bye.1, gw_addr).await;
        }

        // ── Step 4: 清理事务状态 ───────────────────────────────────────────
        call_manager.terminate_call_with_reason(&call_id, "balance_exhausted");
        transactions.remove(&call_id);

        // ── Step 5: 结算 CDR ───────────────────────────────────────────────
        let cid = call_id.clone();
        let user = caller_user.clone();
        let callee = callee_num.clone();
        let duration_ms = max_duration_ms.max(1) as i64;
        tokio::spawn(async move {
            if let Err(e) = db.settle_call(&cid, &user, &callee, duration_ms).await {
                warn!(call_id = %cid, error = %e, "billing watchdog: CDR settlement failed");
            }
        });
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
