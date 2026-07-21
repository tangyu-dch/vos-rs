pub(crate) mod billing_settlement;
pub(crate) mod cdr;
pub(crate) mod cluster;
pub(crate) mod config;
pub(crate) mod edge_state;
mod manage;
pub(crate) mod media;
pub(crate) mod net;
pub(crate) mod number_routing;
pub(crate) mod resource_lease;
pub(crate) mod routing;
pub(crate) mod security;
pub(crate) mod sip;
pub(crate) mod timers;
mod webhook_delivery;
mod webhooks;

pub(crate) use cdr::{cdr_sinks_from_config, flush_cdr_batch_with_retry_and_wal};
pub(crate) use number_routing::{reload_number_routes, spawn_number_route_refresh};
pub(crate) use routing::{
    parse_gateway_target, reload_routes_from_database, route_table_from_config,
    spawn_periodic_route_refresh, spawn_route_reload_listener, warm_hot_path_redis_cache,
};
pub(crate) use security::rules::refresh_anti_fraud_rules;
pub(crate) use sip::extract_call_id_fast;

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
    calculate_mos_for_legs, spawn_client_transaction_retransmission,
    spawn_gateway_health_probe_loop, spawn_nat_keepalive_loop, spawn_session_timer_watchdog,
};

use call_core::CallManager;
use config::EdgeConfig;
use media::MediaRelayState;
use net::{create_tls_acceptor, BufferPool, PooledBuffer, Transport};
use sip_core::{parse_message, Method, SipMessageBorrow};
use std::{
    net::SocketAddr,
    sync::{atomic::Ordering, Arc},
    time::Duration,
};
use tokio::net::UdpSocket;
use tracing::{debug, info, warn};
use tracing_subscriber::EnvFilter;

type AnyError = Box<dyn std::error::Error + Send + Sync>;

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), AnyError> {
    let mut edge_config = EdgeConfig::from_env();
    init_tracing(&config_logging_filter("sip_edge=info"));
    edge_config.validate_cluster()?;
    let bind_addr = edge_config.sip_udp_bind.clone();
    let route_table = route_table_from_config(&edge_config)?;
    if route_table.is_empty() {
        warn!("no outbound route configured; INVITE requests will receive 404");
    }

    // 先用 bootstrap 的 db url 连接数据库以运行 migration 结构
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

    // 检查并强制校验 Redis 连接
    let redis_url = edge_config
        .redis_url
        .clone()
        .unwrap_or_else(|| "redis://127.0.0.1:6379".to_string());
    let redis_client = match redis::Client::open(redis_url.clone()) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(redis_url, error = %e, "Redis 客户端打开失败。VOS-RS 必须有 Redis 运行！");
            return Err(e.into());
        }
    };
    let redis_conn_for_state = match redis::aio::ConnectionManager::new(redis_client.clone()).await
    {
        Ok(conn) => Some(conn),
        Err(e) => {
            tracing::error!(redis_url, error = %e, "Redis 连接失败，请检查服务状态。VOS-RS 必须有 Redis 运行！");
            return Err(e.into());
        }
    };
    info!("Redis 存储连接成功 (必须要求)");

    // 运行数据库配置覆盖：系统配置加载
    if edge_config.dynamic_config_enabled {
        if let Some(ref db) = db_store {
            edge_config.override_from_db(db).await;
        }
    } else {
        info!("dynamic Redis/PostgreSQL configuration override is disabled");
    }
    // 动态配置可能替换媒体节点池，必须在创建监听和媒体状态前再次校验。
    edge_config.validate_cluster()?;

    // STUN: discover public address for media relay if configured
    if let Some(stun_server) = edge_config.stun_server.clone() {
        if !stun_server.is_empty() {
            net::run_stun_discovery(&stun_server, &mut edge_config).await;
        }
    }

    // UPnP: auto-discover router and add port mappings if enabled
    if edge_config.upnp_enabled {
        net::run_upnp_port_mapping(&bind_addr, &edge_config);
    }

    // Wrap EdgeConfig in Arc — all mutations done, now read-only shared access
    let edge_config = Arc::new(edge_config);

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

    // 使用有界队列防止数据库/NATS 故障时 CDR 无限堆积。
    let cdr_queue_capacity = edge_config.cdr_queue_capacity;
    let cdr_persistence_enabled = edge_config.cdr_persistence_enabled;
    let (cdr_tx, mut cdr_rx) = tokio::sync::mpsc::channel::<call_core::CallCdr>(cdr_queue_capacity);
    let (call_manager, webhook_receiver) = if edge_config.webhooks.enabled {
        let (event_sender, event_receiver) =
            tokio::sync::mpsc::channel(edge_config.webhooks.queue_capacity);
        (
            CallManager::new_with_event_sink(route_table, cdr_tx, event_sender),
            Some(event_receiver),
        )
    } else {
        (CallManager::new(route_table, cdr_tx), None)
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
    edge_state.self_weak.set(Arc::downgrade(&edge_state)).ok();

    // 将 Redis 连接注入 EdgeState（用于集群注册状态共享）
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
    }

    // Start background task to capture SIP packet trace flow (SipFlow)
    let sip_flow_tx = sip::sip_flow::SipFlowWriter::start(
        Arc::clone(&edge_state),
        10000, // queue capacity
    );
    edge_state.sip_flow_tx.set(sip_flow_tx).ok();

    // Start background task to flush CDRs in batches (every 100ms or 100 entries)
    let cdr_sinks_bg = Arc::clone(&cdr_sinks);
    tokio::spawn(async move {
        let mut batch = Vec::new();
        let mut interval = tokio::time::interval(Duration::from_millis(100));
        loop {
            tokio::select! {
                Some(cdr) = cdr_rx.recv() => {
                    batch.push(cdr);
                    if batch.len() >= 100 && cdr_persistence_enabled {
                        flush_cdr_batch_with_retry_and_wal(&cdr_sinks_bg, &batch).await;
                        batch.clear();
                    } else if batch.len() >= 100 {
                        batch.clear();
                    }
                }
                _ = interval.tick() => {
                    if !batch.is_empty() && cdr_persistence_enabled {
                        flush_cdr_batch_with_retry_and_wal(&cdr_sinks_bg, &batch).await;
                        batch.clear();
                    } else if !batch.is_empty() {
                        batch.clear();
                    }
                }
            }
        }
    });

    // 启动管理 API（活跃呼叫查询 / 强制拆线）
    let manage_addr = edge_config.manage_bind.clone();
    {
        let manage_state = Arc::clone(&edge_state);
        let addr = manage_addr.clone();
        let internal_secret = edge_config.internal_secret.clone();
        tokio::spawn(async move {
            manage::serve(addr, manage_state, internal_secret).await;
        });
    }

    if let Some(db) = &db_store {
        let has_users = sqlx::query("SELECT 1 FROM sip_users LIMIT 1")
            .fetch_optional(db.pool())
            .await?
            .is_some();
        if !has_users {
            if let Some(raw_users) = edge_config.bootstrap_auth_users.as_deref() {
                for entry in raw_users.split(',') {
                    let entry = entry.trim();
                    if let Some((username, password)) =
                        entry.split_once(':').or_else(|| entry.split_once('='))
                    {
                        let username = username.trim();
                        let password = password.trim();
                        if !username.is_empty() {
                            db.insert_user(username, password).await?;
                            info!(username, "seeded SIP user into database");
                        }
                    }
                }
            }
        }

        if edge_config.database_routes_enabled {
            let has_gateways = sqlx::query("SELECT 1 FROM sip_gateways LIMIT 1")
                .fetch_optional(db.pool())
                .await?
                .is_some();
            let has_routes = sqlx::query("SELECT 1 FROM sip_routes LIMIT 1")
                .fetch_optional(db.pool())
                .await?
                .is_some();
            if !has_gateways {
                if !edge_config.default_gateway.trim().is_empty() {
                    let raw_gateway = &edge_config.default_gateway;
                    let raw_gateway = raw_gateway.trim();
                    if !raw_gateway.is_empty() {
                        if let Ok(target) = parse_gateway_target("default", raw_gateway) {
                            db.insert_gateway("default", &target.host, target.port, "udp")
                                .await?;
                            db.insert_route("default", "", 100, "default").await?;
                            info!(
                                gateway = raw_gateway,
                                "seeded default gateway and route into database"
                            );
                        }
                    }
                }
            } else if !has_routes && !edge_config.default_gateway.trim().is_empty() {
                db.insert_route("default", "", 100, "default").await?;
                info!("seeded default route into database (gateway already exists)");
            }

            match reload_routes_from_database(&edge_state, db).await {
                Ok(()) => info!("loaded routes from database"),
                Err(e) => warn!(%e, "failed to load routes from database"),
            }

            if let Some(ref nats_url) = edge_config.nats_url {
                spawn_route_reload_listener(
                    nats_url.clone(),
                    Arc::clone(&edge_state),
                    db_store.clone(),
                );
            }

            if edge_config.gateway_health_checks_enabled {
                match db.load_gateway_health_list().await {
                    Ok(health_list) => {
                        let health = &edge_state.gateway_health;
                        for (
                            gw_id,
                            open,
                            failures,
                            _state,
                            last_failure_at,
                            half_open_successes,
                            _last_probe_at,
                            active_calls,
                        ) in health_list
                        {
                            let last_failure_sys = last_failure_at.map(|dt| {
                                std::time::UNIX_EPOCH
                                    + std::time::Duration::from_secs(dt.unix_timestamp() as u64)
                            });
                            health.restore_state(
                                &gw_id,
                                open,
                                failures,
                                last_failure_sys,
                                half_open_successes,
                                active_calls,
                            );
                        }
                        info!("loaded and restored gateway health states from database");
                    }
                    Err(error) => {
                        warn!(%error, "failed to load gateway health states from database")
                    }
                }
            }
        } else {
            info!("database route loading disabled; using config.yaml routing table");
        }

        refresh_anti_fraud_rules(&edge_state).await;
    }

    let socket: Arc<UdpSocket> = Arc::new(UdpSocket::bind(&bind_addr).await?);

    // Increase UDP buffers for high CPS; the kernel may cap them via rmem_max/wmem_max.
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        let fd = socket.as_raw_fd();
        let receive_buffer = edge_config.udp_receive_buffer_bytes.min(i32::MAX as usize) as i32;
        let send_buffer = edge_config.udp_send_buffer_bytes.min(i32::MAX as usize) as i32;
        unsafe {
            libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_RCVBUF,
                &receive_buffer as *const i32 as *const libc::c_void,
                std::mem::size_of::<i32>() as libc::socklen_t,
            );
            libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_SNDBUF,
                &send_buffer as *const i32 as *const libc::c_void,
                std::mem::size_of::<i32>() as libc::socklen_t,
            );
        }
    }

    edge_state.set_socket(Arc::clone(&socket));
    info!(%bind_addr, "sip-edge UDP listener started");

    if let Some(ref nats_url) = edge_config.nats_url {
        let nats_url_clone = nats_url.clone();
        let edge_state_clone = Arc::clone(&edge_state);
        let edge_config_clone = Arc::clone(&edge_config);
        tokio::spawn(async move {
            match async_nats::connect(&nats_url_clone).await {
                Ok(client) => {
                    info!("NATS call control client successfully connected");
                    edge_state_clone.set_nats(client.clone());
                    if edge_config_clone.webhooks.control_mode == "nats" {
                        if let Err(e) =
                            crate::sip::handlers::command_listener::start_command_listener(
                                edge_state_clone.clone(),
                                edge_config_clone,
                                client.clone(),
                            )
                            .await
                        {
                            tracing::error!("NATS VCI command listener failed: {:?}", e);
                        }
                    }

                    let sub_client = client.clone();
                    let sub_state = edge_state_clone.clone();
                    tokio::spawn(async move {
                        use futures::StreamExt;
                        if let Ok(mut subscriber) = sub_client
                            .subscribe("vos_rs.cluster.registration.invalidate")
                            .await
                        {
                            tracing::info!("Subscribed to vos_rs.cluster.registration.invalidate");
                            while let Some(message) = subscriber.next().await {
                                if let Ok(payload) = std::str::from_utf8(&message.payload) {
                                    if let Ok(msg) = serde_json::from_str::<
                                        crate::sip::registrar::RegistrationInvalidateMsg,
                                    >(payload)
                                    {
                                        sub_state
                                            .registrar
                                            .write()
                                            .await
                                            .invalidate_cache(&msg.aor);
                                        tracing::debug!(aor = %msg.aor, "invalidated registration cache from cluster broadcast");
                                    }
                                }
                            }
                        }
                    });
                }
                Err(e) => {
                    tracing::error!("failed to connect to NATS for call control: {:?}", e);
                }
            }
        });
    }

    // Start TCP listener
    let tcp_listener = match tokio::net::TcpListener::bind(&bind_addr).await {
        Ok(l) => {
            info!(%bind_addr, "sip-edge TCP listener started");
            Some(Arc::new(l) as Arc<tokio::net::TcpListener>)
        }
        Err(e) => {
            warn!(%bind_addr, error = %e, "failed to start TCP listener");
            None
        }
    };

    if let Some(l) = tcp_listener {
        net::start_tcp_listener(l, Arc::clone(&edge_state), Arc::clone(&edge_config));
    }

    // Start TLS listener (default derived port 5061) only when TLS material is configured.
    let tls_bind_addr = edge_config
        .tls_bind_addr
        .clone()
        .or_else(|| {
            bind_addr.parse::<SocketAddr>().ok().map(|addr| {
                let mut tls_addr = addr;
                tls_addr.set_port(5061);
                tls_addr.to_string()
            })
        })
        .and_then(|addr| match addr.parse::<SocketAddr>() {
            Ok(addr) => Some(addr),
            Err(e) => {
                warn!(addr, error = %e, "invalid TLS bind address; TLS listener disabled");
                None
            }
        });

    let tls_acceptor = match create_tls_acceptor(
        edge_config.tls_cert_path.as_deref(),
        edge_config.tls_key_path.as_deref(),
        edge_config.tls_allow_test_certificate,
    ) {
        Ok(Some(acceptor)) => {
            if let Some(tls_addr) = tls_bind_addr {
                match tokio::net::TcpListener::bind(&tls_addr).await {
                    Ok(l) => {
                        info!(%tls_addr, "sip-edge TLS listener started");
                        net::start_tls_listener(
                            Arc::new(l),
                            acceptor.clone(),
                            Arc::clone(&edge_state),
                            Arc::clone(&edge_config),
                        );
                    }
                    Err(e) => {
                        warn!(%tls_addr, error = %e, "failed to start TLS listener");
                    }
                }
            }
            Some(acceptor)
        }
        Ok(None) => {
            info!("SIP TLS listener disabled; configure TLS cert/key paths in config.yaml");
            None
        }
        Err(e) => {
            warn!(error = %e, "failed to create TLS acceptor; TLS listener disabled");
            None
        }
    };

    // Start WebSocket listener
    let ws_bind_addr = edge_config.ws_bind_addr.clone().unwrap_or_else(|| {
        if let Ok(addr) = bind_addr.parse::<SocketAddr>() {
            let mut ws_addr = addr;
            ws_addr.set_port(5062);
            ws_addr.to_string()
        } else {
            "0.0.0.0:5062".to_string()
        }
    });

    if let Ok(ws_listener) = tokio::net::TcpListener::bind(&ws_bind_addr).await {
        info!(%ws_bind_addr, "sip-edge WebSocket listener started");
        net::start_ws_listener(
            ws_listener,
            tls_acceptor,
            Arc::clone(&edge_state),
            Arc::clone(&edge_config),
        );
    } else {
        warn!(%ws_bind_addr, "failed to start WebSocket listener");
    }

    // Start session timer watchdog — sends BYE to zombie calls that exceed Session-Expires
    spawn_session_timer_watchdog(
        Arc::clone(&edge_state),
        Arc::clone(&socket),
        edge_config.clone(),
    );
    resource_lease::spawn_renewal_loop(Arc::clone(&edge_state));

    // Start NAT keepalive background loop — sends keepalive probes to active registrations
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

                    let client_transaction_key = if transport == Transport::Udp {
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
                    let registered_transaction = client_transaction_key.and_then(|key| {
                        if state.client_transactions.contains_key(&key) {
                            None
                        } else {
                            spawn_client_transaction_retransmission(
                                Arc::clone(&state),
                                Arc::clone(&sock),
                                datagram.target.clone(),
                                datagram.bytes.clone(),
                                key.clone(),
                                cfg.clone(),
                            );
                            Some(key)
                        }
                    });

                    if let Err(error) = state.send_sip_datagram(datagram.clone(), &sock, &cfg).await
                    {
                        if let Some(key) = registered_transaction.as_ref() {
                            state.cancel_client_transaction(key);
                        }
                        warn!(target = %datagram.target, error = %error, "failed to send SIP message");
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

                // 使用 Call-ID 哈希进行 worker 路由：确保同一 Dialog 的所有消息
                // (INVITE/ACK/BYE/re-INVITE) 由同一 Worker 处理，消除跨 worker 竞态。
                // 若解析失败则 fallback 到 peer-IP 哈希保证负载均衡。
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

    Ok(())
}

fn init_tracing(filter: &str) {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new(filter))
        .init();
}

fn config_logging_filter(default: &str) -> String {
    let path = std::env::var("VOS_RS_CONFIG_FILE").unwrap_or_else(|_| "config.yaml".to_string());
    std::fs::read_to_string(path)
        .ok()
        .and_then(|content| serde_yaml::from_str::<serde_yaml::Value>(&content).ok())
        .and_then(|root| {
            root.get("logging")?
                .get("filter")?
                .as_str()
                .map(str::to_owned)
        })
        .filter(|filter| !filter.trim().is_empty())
        .unwrap_or_else(|| default.to_string())
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

    include!("tests/unified_tests.rs");
}
