pub(crate) mod billing;
pub(crate) mod cdr;
pub(crate) mod cluster;
pub(crate) mod config;
pub(crate) mod edge_state;
mod manage;
pub(crate) mod media;
pub(crate) mod net;
pub(crate) mod resource_lease;
pub(crate) mod routing;
pub(crate) mod runtime;
pub(crate) mod security;
pub(crate) mod sip;
pub(crate) mod startup;
pub(crate) mod tenant;
pub(crate) mod timers;
mod webhooks;

pub(crate) use cdr::cdr_sinks_from_config;
pub(crate) use routing::{
    reload_number_routes, route_table_from_config, spawn_number_route_refresh,
    spawn_periodic_route_refresh, warm_hot_path_redis_cache,
};
pub(crate) use sip::extract_call_id_fast;
pub(crate) use startup::{
    config_logging_filter, init_tracing, seed_database_defaults, validate_bootstrap_config,
    validate_runtime_security,
};

// `EdgeState` 在 main.rs 中通过 `EdgeState::with_media_relay_and_db` 等构造函数直接使用，
// 必须从 `edge_state` 模块重导出。
pub(crate) use edge_state::EdgeState;
// `handle_datagram` / `sbc` 在 main.rs / edge_state 内被非测试代码引用。
pub(crate) use security::sbc;
pub(crate) use sip::handle_datagram;
// 计时器后台任务在 main 中启动。
pub(crate) use timers::{
    spawn_gateway_health_probe_loop, spawn_nat_keepalive_loop, spawn_session_timer_watchdog,
    spawn_subscription_prune_loop,
};

// 以下重导出仅服务于 `#[cfg(test)] mod tests` 中 `super::` 路径引用，
// 非测试构建不会引用，故以 `#[cfg(test)]` 限定避免 unused_imports 告警。
#[cfg(test)]
pub(crate) use edge_state::CdrSinks;
#[cfg(test)]
pub(crate) use sip::client_transaction::spawn_client_transaction_retransmission;
#[cfg(test)]
pub(crate) use sip::{auth, response};
#[cfg(test)]
pub(crate) use sip::{AuthDecision, ClientTransactionKey};
#[cfg(test)]
pub(crate) use timers::calculate_mos_for_legs;

use call_core::CallManager;
use config::EdgeConfig;
use media::MediaRelayState;
use net::{BufferPool, PooledBuffer};
use std::{
    sync::{atomic::Ordering, Arc},
    time::Duration,
};
use tokio::net::UdpSocket;
use tracing::{info, warn};

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

    // TURN 中继预分配（如果配置了 TURN 服务器）
    let turn_client = net::run_turn_bootstrap(&edge_config).await;

    let edge_config = Arc::new(edge_config);
    // 先创建 std UDP socket 设置 DSCP，再转 tokio
    let std_socket = std::net::UdpSocket::bind(&bind_addr)?;
    net::apply_dscp(&std_socket, edge_config.sip_dscp);
    std_socket.set_nonblocking(true)?;
    let socket = Arc::new(UdpSocket::from_std(std_socket)?);
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
    let (cdr_tx, cdr_rx) = tokio::sync::mpsc::channel::<call_core::CallCdr>(cdr_queue_capacity);
    let cdr_spool = cdr::CdrSpool::open(cdr::configured_spool_dir())?;
    let cdr_pipeline_metrics = cdr_spool.metrics();
    let durable_cdr_sink = cdr::DurableCdrSink::new(cdr_tx, cdr_spool.clone());
    let (call_manager, event_receiver) = if edge_config.webhooks.enabled {
        // Webhook 启用：创建 event sink，启动完整 pipeline（内部含 RWI 广播）
        let (event_sender, event_receiver) =
            tokio::sync::mpsc::channel(edge_config.webhooks.queue_capacity);
        (
            CallManager::new_with_event_sink(route_table, durable_cdr_sink, event_sender),
            Some(event_receiver),
        )
    } else if edge_config.nats_url.is_some() {
        // Webhook 未启用但 NATS 可用：创建 event sink，仅启动 RWI 实时事件广播
        let (event_sender, event_receiver) = tokio::sync::mpsc::channel(4096);
        (
            CallManager::new_with_event_sink(route_table, durable_cdr_sink, event_sender),
            Some(event_receiver),
        )
    } else {
        (CallManager::new(route_table, durable_cdr_sink), None)
    };

    if let Some(event_receiver) = event_receiver {
        let nats_url = edge_config
            .nats_url
            .as_deref()
            .ok_or("启用 Webhook 或 RWI 广播时必须在 config.yaml 配置 connections.nats.url")?;
        if edge_config.webhooks.enabled {
            // Webhook 完整流水线（含 JetStream 持久化 + HTTP 投递 + RWI 广播）
            webhooks::start_pipeline(
                edge_config.webhooks.clone(),
                nats_url,
                redis_client.clone(),
                event_receiver,
            )
            .await?;
        } else {
            // 仅 RWI 实时事件广播（无 Webhook 投递）
            webhooks::start_rwi_broadcast(nats_url, event_receiver).await?;
        }
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

    // 注入 TURN 中继客户端（如果启动阶段成功分配了 relayed 地址）
    if let Some(turn) = turn_client {
        edge_state.set_turn_client(turn.clone());
        media_relay.set_turn_client(turn);
        info!("TURN 中继客户端已注入媒体转发路径，对非本地目标将自动走中继");
    }

    if let Some(nats_url) = edge_config.nats_url.as_deref() {
        match async_nats::connect(nats_url).await {
            Ok(nats_client) => {
                edge_state.set_nats(nats_client);
                info!(%nats_url, "NATS 客户端连接成功，注册同步与 VCI 通知通道已就绪");
            }
            Err(e) => {
                warn!(error = %e, %nats_url, "NATS 客户端连接失败，注册同步与 VCI 通知将不可用");
            }
        }
    }

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

        // 多租户注册表初始化：仅在 tenant_enabled=true 时启用
        if edge_config.tenant_enabled {
            let tenant_store = tenant::TenantStore::new(db.pool().clone());
            let tenant_registry = Arc::new(tenant::TenantRegistry::new(tenant_store));
            let loaded = tenant_registry.refresh().await;
            info!(loaded, "tenant registry initialized");
            let refresh_interval = edge_config.tenant_refresh_interval_secs;
            Arc::clone(&tenant_registry).spawn_refresh_loop(refresh_interval);
            edge_state.set_tenant_registry(tenant_registry);
            info!(
                enabled = edge_config.tenant_enabled,
                refresh_secs = refresh_interval,
                "multi-tenant isolation enabled"
            );
        } else {
            info!("multi-tenant isolation disabled (tenant_enabled=false)");
        }
    }

    let sip_flow_tx = sip::sip_flow::SipFlowWriter::start(Arc::clone(&edge_state), 10000);
    edge_state.sip_flow_tx.set(sip_flow_tx).ok();

    let (cdr_shutdown_tx, cdr_worker) = runtime::spawn_cdr_worker(
        cdr_rx,
        Arc::clone(&cdr_sinks),
        cdr_spool.clone(),
        cdr_persistence_enabled,
    );
    cdr::spawn_replay_loop(cdr_spool, Arc::clone(&cdr_sinks));

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

    // 启动 SIP WebSocket (WS) 信令入站监听器（如已配置 ws_bind）
    if let Some(ws_bind) = edge_config.ws_bind_addr.clone() {
        let edge_state_ws = Arc::clone(&edge_state);
        let config_ws = Arc::clone(&edge_config);
        tokio::spawn(async move {
            let on_message =
                move |msg_bytes: Vec<u8>,
                      peer: std::net::SocketAddr,
                      connection_tx: tokio::sync::mpsc::Sender<Vec<u8>>| {
                    let state = Arc::clone(&edge_state_ws);
                    let config = Arc::clone(&config_ws);
                    let fut: std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> =
                        Box::pin(async move {
                            // 注册 WS 入站连接，使后续出站路径可复用此通道
                            state.register_tcp_connection(peer, connection_tx.clone());
                            let datagrams =
                                crate::handle_datagram(&msg_bytes, peer, &state, &config).await;
                            for d in datagrams {
                                let _ = connection_tx.send(d.bytes).await;
                            }
                        });
                    fut
                };
            if let Err(e) = net::serve_ws_listener(ws_bind, on_message).await {
                warn!(error = %e, "SIP WS listener terminated");
            }
        });
    }

    // 启动 SIP WebSocket Secure (WSS) 信令入站监听器（如已配置 wss_bind 与 TLS 证书）
    if let Some(wss_bind) = edge_config.wss_bind_addr.clone() {
        match (&edge_config.tls_cert_path, &edge_config.tls_key_path) {
            (Some(cert_path), Some(key_path)) => {
                let edge_state_wss = Arc::clone(&edge_state);
                let config_wss = Arc::clone(&edge_config);
                let cert = cert_path.clone();
                let key = key_path.clone();
                tokio::spawn(async move {
                    let on_message = move |msg_bytes: Vec<u8>,
                                          peer: std::net::SocketAddr,
                                          connection_tx: tokio::sync::mpsc::Sender<Vec<u8>>| {
                        let state = Arc::clone(&edge_state_wss);
                        let config = Arc::clone(&config_wss);
                        let fut: std::pin::Pin<
                            Box<dyn std::future::Future<Output = ()> + Send>,
                        > = Box::pin(async move {
                            state.register_tcp_connection(peer, connection_tx.clone());
                            let datagrams =
                                crate::handle_datagram(&msg_bytes, peer, &state, &config).await;
                            for d in datagrams {
                                let _ = connection_tx.send(d.bytes).await;
                            }
                        });
                        fut
                    };
                    if let Err(e) = net::serve_wss_listener(wss_bind, cert, key, on_message).await {
                        warn!(error = %e, "SIP WSS listener terminated");
                    }
                });
            }
            _ => {
                warn!(
                    "wss_bind configured but tls_cert_path or tls_key_path missing; skipping WSS listener"
                );
            }
        }
    }

    resource_lease::spawn_renewal_loop(Arc::clone(&edge_state));

    let num_workers = if edge_config.udp_workers_auto {
        num_cpus::get().max(1)
    } else {
        edge_config.udp_workers.max(1)
    };
    let queue_capacity = 10000;
    let worker_txs = runtime::spawn_udp_workers(
        Arc::clone(&edge_state),
        Arc::clone(&socket),
        Arc::clone(&edge_config),
        num_workers,
        queue_capacity,
    );

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
    spawn_session_timer_watchdog(
        Arc::clone(&edge_state),
        Arc::clone(&socket),
        Arc::clone(&edge_config),
    );
    spawn_subscription_prune_loop(
        Arc::clone(&edge_state),
        Arc::clone(&socket),
        Arc::clone(&edge_config),
    );
    if edge_config.dynamic_config_enabled && edge_config.database_routes_enabled {
        if let Some(ref db) = db_store {
            spawn_periodic_route_refresh(Arc::clone(&edge_state), db.clone());
        }
    }

    let pool_capacity = runtime::udp_buffer_pool_capacity(num_workers, queue_capacity);
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

    // 释放 TURN allocation（lifetime=0），避免服务器端资源泄漏
    if let Some(turn) = edge_state.turn_client() {
        info!("releasing TURN allocation during shutdown");
        turn.destroy().await;
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
