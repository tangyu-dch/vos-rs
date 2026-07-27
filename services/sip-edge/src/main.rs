pub(crate) mod billing_settlement;
pub(crate) mod cdr;
pub(crate) mod cdr_spool;
pub(crate) mod cluster;
pub(crate) mod config;
pub(crate) mod edge_state;
mod manage;
pub(crate) mod media;
pub(crate) mod nats_cdr;
pub(crate) mod net;
pub(crate) mod number_routing;
pub(crate) mod resource_lease;
pub(crate) mod routing;
pub(crate) mod security;
pub(crate) mod sip;
pub(crate) mod startup;
pub(crate) mod timers;
mod webhook_delivery;
mod webhooks;

pub(crate) use cdr::{cdr_sinks_from_config, flush_cdr_batch_with_retry_and_spool};
pub(crate) use number_routing::{reload_number_routes, spawn_number_route_refresh};
pub(crate) use routing::{
    route_table_from_config, spawn_periodic_route_refresh, warm_hot_path_redis_cache,
};
pub(crate) use sip::client_transaction::spawn_client_transaction_retransmission;
pub(crate) use sip::extract_call_id_fast;
pub(crate) use startup::{
    config_logging_filter, init_tracing, seed_database_defaults, validate_bootstrap_config,
    validate_runtime_security,
};

// Re-export for backward compatibility with inline module references
#[allow(unused_imports)]
pub(crate) use edge_state::*;
#[allow(unused_imports)]
pub(crate) use net::stun_client;
#[allow(unused_imports)]
pub(crate) use net::transport;
#[allow(unused_imports)]
pub(crate) use net::upnp;
#[allow(unused_imports)]
pub(crate) use security::sbc;
#[allow(unused_imports)]
pub(crate) use sip::auth;
#[allow(unused_imports)]
pub(crate) use sip::dialog;
#[allow(unused_imports)]
pub(crate) use sip::handle_datagram;
#[allow(unused_imports)]
pub(crate) use sip::outbound;
#[allow(unused_imports)]
pub(crate) use sip::registrar::RegisterOutcome;
#[allow(unused_imports)]
pub(crate) use sip::response;
#[allow(unused_imports)]
pub(crate) use sip::transaction;
#[allow(unused_imports)]
pub(crate) use sip::{AuthDecision, ClientTransactionKey, RequestTransactionKey};

#[allow(unused_imports)]
pub(crate) use timers::{
    calculate_mos_for_legs, spawn_gateway_health_probe_loop, spawn_nat_keepalive_loop,
    spawn_session_timer_watchdog,
};

use call_core::CallManager;
use config::EdgeConfig;
use media::MediaRelayState;
use net::{BufferPool, PooledBuffer, Transport};
use sip_core::{parse_message, Method, SipMessageBorrow};
use std::{
    net::SocketAddr,
    sync::{atomic::Ordering, Arc},
    time::Duration,
};
use tokio::net::UdpSocket;
use tracing::{debug, info, warn};

type AnyError = Box<dyn std::error::Error + Send + Sync>;

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), AnyError> {
    validate_bootstrap_config()?;
    let mut edge_config = EdgeConfig::from_env();
    init_tracing(&config_logging_filter("sip_edge=info"));
    edge_config.validate_cluster()?;
    let bind_addr = edge_config.sip_udp_bind.clone();
    let route_table = route_table_from_config(&edge_config)?;
    if route_table.is_empty() {
        warn!("no outbound route configured; INVITE requests will receive 404");
    }

    let cdr_sinks = match cdr_sinks_from_config(&edge_config).await {
        Ok(sinks) => sinks,
        Err(e) => {
            tracing::error!(error = %e, "PostgreSQL 数据库初始化失败，请检查连接参数。VOS-RS 必须有 PostgreSQL 运行！");
            return Err(e);
        }
    };
    let db_store = cdr_sinks.postgres.clone();
    if db_store.is_none() {
        tracing::error!("数据库连接未成功初始化，VOS-RS 需要强制开启数据库连接！");
        return Err(
            std::io::Error::other("数据库连接未成功初始化，VOS-RS 需要强制开启数据库连接").into(),
        );
    }

    let redis_url = edge_config
        .redis_url
        .clone()
        .unwrap_or_else(|| "redis://127.0.0.1:6379".to_string());
    let redis_client = match redis::Client::open(redis_url.clone()) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, "Redis 客户端打开失败。VOS-RS 必须有 Redis 运行！");
            return Err(e.into());
        }
    };
    let redis_conn_for_state = match redis::aio::ConnectionManager::new(redis_client.clone()).await
    {
        Ok(conn) => Some(conn),
        Err(e) => {
            tracing::error!(error = %e, "Redis 连接失败，请检查服务状态。VOS-RS 必须有 Redis 运行！");
            return Err(e.into());
        }
    };
    info!("Redis 存储连接成功 (必须要求)");

    if edge_config.dynamic_config_enabled {
        if let Some(ref db) = db_store {
            edge_config.override_from_db(db).await;
        }
    } else {
        info!("dynamic Redis/PostgreSQL configuration override is disabled");
    }
    if let Ok(secret) = std::env::var("VOS_RS_INTERNAL_SECRET") {
        edge_config.internal_secret = secret;
    }
    if let Ok(secret) = std::env::var("VOS_RS_SIP_AUTH_SECRET") {
        edge_config.auth.secret_key = secret;
    }
    validate_runtime_security(&edge_config)?;
    edge_config.validate_cluster()?;

    if let Some(stun_server) = edge_config.stun_server.clone() {
        if !stun_server.is_empty() {
            net::run_stun_discovery(&stun_server, &mut edge_config).await;
        }
    }

    if edge_config.upnp_enabled {
        net::run_upnp_port_mapping(&bind_addr, &edge_config);
    }

    let edge_config = Arc::new(edge_config);
    let socket = Arc::new(UdpSocket::bind(&bind_addr).await?);
    let socket_addr = socket.local_addr()?;
    info!("SIP Edge UDP Listening on {}", socket_addr);

    let storage_config = storage_core::StorageConfig::from_env();
    let storage = match storage_core::create_storage(&storage_config).await {
        Ok(s) => {
            tracing::info!("Storage backend initialized: {}", s.backend_name());
            Some(Arc::from(s))
        }
        Err(e) => {
            tracing::warn!("Failed to initialize storage backend: {}", e);
            None
        }
    };

    let media_relay = MediaRelayState::with_node_pool(
        &edge_config.media_cluster,
        edge_config.recording_workers,
        edge_config.recording_queue_capacity,
        storage,
    );
    let cdr_sinks = std::sync::Arc::new(cdr_sinks);

    let cdr_queue_capacity = edge_config.cdr_queue_capacity;
    let cdr_persistence_enabled = edge_config.cdr_persistence_enabled;
    let (cdr_tx, mut cdr_rx) = tokio::sync::mpsc::channel::<call_core::CallCdr>(cdr_queue_capacity);
    let cdr_spool = cdr_spool::CdrSpool::open(cdr_spool::configured_spool_dir())?;
    let cdr_pipeline_metrics = cdr_spool.metrics();
    let durable_cdr_sink = cdr_spool::DurableCdrSink::new(cdr_tx, cdr_spool.clone());
    let (call_manager, webhook_receiver) = if edge_config.webhooks.enabled {
        let (event_sender, event_receiver) =
            tokio::sync::mpsc::channel(edge_config.webhooks.queue_capacity);
        (
            CallManager::new_with_event_sink(route_table, durable_cdr_sink, event_sender),
            Some(event_receiver),
        )
    } else {
        (CallManager::new(route_table, durable_cdr_sink), None)
    };

    if let Some(event_receiver) = webhook_receiver {
        let nats_url = edge_config
            .nats_url
            .as_deref()
            .ok_or("启用 Webhook 时必须在 config.yaml 配置 connections.nats.url")?;
        webhooks::start_pipeline(
            edge_config.webhooks.clone(),
            nats_url,
            redis_client.clone(),
            event_receiver,
        )
        .await?;
    }

    let edge_state = Arc::new(EdgeState::with_media_relay_and_db(
        call_manager,
        media_relay.clone(),
        db_store.clone(),
        &edge_config,
    ));
    edge_state.set_socket(Arc::clone(&socket));
    edge_state
        .cdr_pipeline_metrics
        .set(cdr_pipeline_metrics)
        .ok();
    edge_state.self_weak.set(Arc::downgrade(&edge_state)).ok();

    edge_state.set_voice_engine(Arc::new(
        sip::handlers::ivr_topology::VoiceEngineManager::from_env(),
    ));

    if let Some(redis_conn) = redis_conn_for_state {
        edge_state.set_redis(redis_conn.clone());
        edge_state.set_registration_sync(cluster::start_registration_sync(redis_conn));
    }
    let node_heartbeat =
        cluster::spawn_node_heartbeat(&redis_client, &edge_config.cluster, Arc::clone(&edge_state))
            .await?;
    cluster::start_inter_node_egress(Arc::clone(&edge_state), &edge_config).await?;
    warm_hot_path_redis_cache(&edge_state, db_store.as_ref()).await?;
    if let Some(ref db) = db_store {
        reload_number_routes(&edge_state, db).await?;
        spawn_number_route_refresh(
            Arc::clone(&edge_state),
            db.clone(),
            edge_config.nats_url.clone(),
        );
        seed_database_defaults(db, &edge_config).await?;
    }

    let sip_flow_tx = sip::sip_flow::SipFlowWriter::start(Arc::clone(&edge_state), 10000);
    edge_state.sip_flow_tx.set(sip_flow_tx).ok();

    let cdr_sinks_bg = Arc::clone(&cdr_sinks);
    let cdr_spool_bg = cdr_spool.clone();
    let (cdr_shutdown_tx, mut cdr_shutdown_rx) = tokio::sync::oneshot::channel();
    let cdr_worker = tokio::spawn(async move {
        let mut batch = Vec::new();
        let mut interval = tokio::time::interval(Duration::from_millis(100));
        loop {
            tokio::select! {
                Some(cdr) = cdr_rx.recv() => {
                    batch.push(cdr);
                    if batch.len() >= 100 && cdr_persistence_enabled {
                        flush_cdr_batch_with_retry_and_spool(&cdr_sinks_bg, &cdr_spool_bg, &batch).await;
                        batch.clear();
                    } else if batch.len() >= 100 {
                        batch.clear();
                    }
                }
                _ = interval.tick() => {
                    if !batch.is_empty() && cdr_persistence_enabled {
                        flush_cdr_batch_with_retry_and_spool(&cdr_sinks_bg, &cdr_spool_bg, &batch).await;
                        batch.clear();
                    } else if !batch.is_empty() {
                        batch.clear();
                    }
                }
                _ = &mut cdr_shutdown_rx => {
                    while let Ok(cdr) = cdr_rx.try_recv() {
                        batch.push(cdr);
                    }
                    if !batch.is_empty() && cdr_persistence_enabled {
                        flush_cdr_batch_with_retry_and_spool(
                            &cdr_sinks_bg,
                            &cdr_spool_bg,
                            &batch,
                        ).await;
                    }
                    break;
                }
            }
        }
    });
    cdr_spool::spawn_replay_loop(cdr_spool, Arc::clone(&cdr_sinks));

    let manage_addr = edge_config.manage_bind.clone();
    {
        let manage_state = Arc::clone(&edge_state);
        let addr = manage_addr.clone();
        let internal_secret = edge_config.internal_secret.clone();
        let advertised_addr = edge_config.advertised_addr.clone();
        tokio::spawn(async move {
            manage::serve(addr, manage_state, internal_secret, advertised_addr).await;
        });
    }

    resource_lease::spawn_renewal_loop(Arc::clone(&edge_state));

    let num_workers = if edge_config.udp_workers_auto {
        num_cpus::get().max(1)
    } else {
        edge_config.udp_workers.max(1)
    };
    let queue_capacity = 10000;
    let mut worker_txs = Vec::new();

    for worker_id in 0..num_workers {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<(PooledBuffer, SocketAddr)>(queue_capacity);
        worker_txs.push(tx);

        let state = Arc::clone(&edge_state);
        let sock = Arc::clone(&socket);
        let cfg = edge_config.clone();

        tokio::spawn(async move {
            debug!("UDP Worker {} started", worker_id);
            while let Some((packet, peer)) = rx.recv().await {
                let datagrams = sip::handle_datagram(&packet, peer, &state, &cfg).await;
                if datagrams.is_empty() {
                    debug!(%peer, "received datagram without response");
                }

                for datagram in datagrams {
                    let transport = if let Ok(msg) = parse_message(&datagram.bytes) {
                        if let Some(via) = msg.headers().get("via") {
                            let via_str = via.as_str().to_uppercase();
                            if via_str.contains("SIP/2.0/TLS") {
                                Transport::Tls
                            } else if via_str.contains("SIP/2.0/TCP") {
                                Transport::Tcp
                            } else {
                                Transport::Udp
                            }
                        } else {
                            Transport::Udp
                        }
                    } else {
                        Transport::Udp
                    };

                    let client_transaction_key =
                        if transport == Transport::Udp && datagram.is_request() {
                            parse_message(&datagram.bytes)
                                .ok()
                                .and_then(|message| match message {
                                    SipMessageBorrow::Request(request)
                                        if !matches!(&request.method, Method::Ack) =>
                                    {
                                        sip::ClientTransactionKey::from_request(&request)
                                    }
                                    _ => None,
                                })
                        } else {
                            None
                        };
                    let registered_transaction = client_transaction_key.clone().and_then(|key| {
                        spawn_client_transaction_retransmission(
                            Arc::clone(&state),
                            Arc::clone(&sock),
                            datagram.target.clone(),
                            datagram.bytes.clone(),
                            key.clone(),
                            cfg.clone(),
                        )
                        .then_some(key)
                    });
                    if client_transaction_key.is_some() && registered_transaction.is_none() {
                        continue;
                    }

                    if let Err(error) = state.send_sip_datagram(datagram.clone(), &sock, &cfg).await
                    {
                        if let Some(key) = registered_transaction.as_ref() {
                            state.client_transactions.cancel(key);
                        }
                        warn!(target = %datagram.target, error = %error, "failed to send SIP message");
                    } else if datagram.bytes.starts_with(b"INVITE ") {
                        let msg_head = String::from_utf8_lossy(
                            &datagram.bytes[..datagram.bytes.len().min(300)],
                        );
                        debug!(target = %datagram.target, head = %msg_head, "sending outbound INVITE datagram");
                    } else {
                        debug!(
                            peer = %datagram.target,
                            bytes = datagram.bytes.len(),
                            "sent SIP datagram"
                        );
                    }
                }
            }
        });
    }

    spawn_nat_keepalive_loop(Arc::clone(&edge_state), Arc::clone(&socket));
    crate::sip::outbound_reg::spawn_outbound_registration_loop(
        Arc::clone(&edge_state),
        Arc::clone(&edge_config),
    );
    if edge_config.gateway_health_checks_enabled {
        spawn_gateway_health_probe_loop(
            Arc::clone(&edge_state),
            Arc::clone(&socket),
            Arc::clone(&edge_config),
        );
    }
    if edge_config.dynamic_config_enabled && edge_config.database_routes_enabled {
        if let Some(ref db) = db_store {
            spawn_periodic_route_refresh(Arc::clone(&edge_state), db.clone());
        }
    }

    let pool_capacity = (num_workers * queue_capacity).min(4096) + 256;
    let buffer_pool = Arc::new(BufferPool::new(pool_capacity, 65535));
    let mut shutdown_check_interval = tokio::time::interval(Duration::from_millis(500));
    let mut is_draining = false;
    let shutdown_timeout = tokio::time::sleep(Duration::from_secs(999999));
    tokio::pin!(shutdown_timeout);

    loop {
        let mut raw_buf = buffer_pool.acquire();
        tokio::select! {
            result = socket.recv_from(&mut raw_buf) => {
                let (size, peer) = result?;
                raw_buf.truncate(size);
                let packet = PooledBuffer::new(raw_buf, Arc::clone(&buffer_pool));

                // SIP client transactions belong to the transport layer. Apply responses
                // before worker queueing so 100/180/183 can stop Timer A without waiting
                // for routing, media, CDR, or other application response processing.
                edge_state.client_transactions.observe_packet(&packet);

                use std::hash::{Hash, Hasher};
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                match extract_call_id_fast(&packet) {
                    Some(call_id) => call_id.hash(&mut hasher),
                    None => peer.hash(&mut hasher),
                }
                let worker_idx = (hasher.finish() as usize) % num_workers;

                if worker_txs[worker_idx].try_send((packet, peer)).is_err() {
                    static DROP_COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
                    let cnt = DROP_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if cnt % 1000 == 0 {
                        tracing::warn!("UDP Worker {} 队列满，丢弃入站数据包 (当前累计丢包数: {})", worker_idx, cnt);
                    }
                }

                if is_draining {
                    let active_count = edge_state.call_manager.active_calls_count();
                    if active_count == 0 {
                        info!("All active calls ended. Exiting gracefully.");
                        break;
                    }
                }
            }
            _ = tokio::signal::ctrl_c(), if !is_draining => {
                info!("Shutdown signal received. Entering graceful drain mode...");
                edge_state.draining.store(true, Ordering::Release);
                if let Some(heartbeat) = &node_heartbeat {
                    if let Err(error) = heartbeat.refresh().await {
                        warn!(%error, "failed to publish draining state immediately");
                    }
                }
                is_draining = true;
                shutdown_timeout.as_mut().reset(tokio::time::Instant::now() + Duration::from_secs(30));
            }
            _ = shutdown_check_interval.tick(), if is_draining => {
                let active_count = edge_state.call_manager.active_calls_count();
                info!(active_calls = active_count, "Draining: waiting for active calls to terminate...");
                if active_count == 0 {
                    info!("All active calls ended. Exiting gracefully.");
                    break;
                }
            }
            _ = &mut shutdown_timeout, if is_draining => {
                warn!("Graceful shutdown timeout reached. Exiting immediately.");
                break;
            }
        }
    }

    if let Some(heartbeat) = &node_heartbeat {
        if let Err(error) = heartbeat.unregister().await {
            warn!(%error, "failed to unregister SIP cluster node during shutdown");
        }
    }

    let _ = cdr_shutdown_tx.send(());
    match tokio::time::timeout(Duration::from_secs(10), cdr_worker).await {
        Ok(Ok(())) => info!("CDR writer drained during shutdown"),
        Ok(Err(error)) => warn!(%error, "CDR writer task failed during shutdown"),
        Err(_) => warn!("timed out waiting for CDR writer to drain; durable spool may replay overflow records on restart"),
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::auth::{digest_response, AuthConfig};
    use super::{
        handle_datagram, media, response, spawn_client_transaction_retransmission,
        spawn_nat_keepalive_loop, spawn_session_timer_watchdog, CdrSinks, ClientTransactionKey,
        EdgeConfig, EdgeState,
    };
    use crate::cdr::flush_cdr_batch;
    use crate::edge_state::PendingDatagram;
    use crate::net::handle_ws_connection;
    use crate::startup::validate_runtime_security_for_environment;
    use call_core::{CallId, CallManager, CallState, Route, RouteTable, RouteTarget};
    use sdp_core::RtpEndpoint;
    use sip_core::{parse_message, SipMessage, SipUri};
    use std::{
        collections::HashMap,
        net::SocketAddr,
        str::FromStr,
        sync::Arc,
        time::{Duration, SystemTime},
    };
    use tokio::net::UdpSocket;

    #[test]
    fn production_rejects_default_edge_secrets() {
        let config = EdgeConfig::default();
        let error = validate_runtime_security_for_environment(&config, true)
            .expect_err("production defaults must be rejected");

        assert!(error.to_string().contains("VOS_RS_INTERNAL_SECRET"));
    }

    include!("tests/unified_tests.rs");
}
