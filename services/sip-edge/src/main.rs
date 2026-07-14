pub(crate) mod config;
pub(crate) mod edge_state;
mod manage;
pub(crate) mod media;
pub(crate) mod net;
pub(crate) mod security;
pub(crate) mod sip;
pub(crate) mod timers;

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

use call_core::{CallManager, Route, RouteTable, RouteTarget};
use cdr_core::PostgresCdrStore;
use config::EdgeConfig;
use futures::StreamExt;
use media::MediaRelayState;
use net::{create_tls_acceptor, Transport};
use sip_core::{parse_message, Method, SipMessage, SipUri};
use std::{
    collections::HashMap,
    io,
    net::SocketAddr,
    str::FromStr,
    sync::{atomic::Ordering, Arc},
    time::Duration,
};
use tokio::net::UdpSocket;
use tracing::{debug, info, warn};
use tracing_subscriber::EnvFilter;

type AnyError = Box<dyn std::error::Error + Send + Sync>;

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), AnyError> {
    init_tracing();

    let mut edge_config = EdgeConfig::from_env();
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
        return Err(std::io::Error::other(
            "数据库连接未成功初始化，VOS-RS 需要强制开启数据库连接",
        )
        .into());
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
    let redis_conn_for_state = match redis_client.get_multiplexed_tokio_connection().await {
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

    let media_relay = MediaRelayState::with_recording_pool(
        edge_config.recording_workers,
        edge_config.recording_queue_capacity,
    );
    let cdr_sinks = std::sync::Arc::new(cdr_sinks);

    // 使用有界队列防止数据库/NATS 故障时 CDR 无限堆积。
    let cdr_queue_capacity = edge_config.cdr_queue_capacity;
    let cdr_persistence_enabled = edge_config.cdr_persistence_enabled;
    let (cdr_tx, mut cdr_rx) = tokio::sync::mpsc::channel::<call_core::CallCdr>(cdr_queue_capacity);
    let call_manager = CallManager::new(route_table, cdr_tx);

    let edge_state = Arc::new(EdgeState::with_media_relay_and_db(
        call_manager,
        media_relay.clone(),
        db_store.clone(),
        &edge_config,
    ));

    // 将 Redis 连接注入 EdgeState（用于集群注册状态共享）
    if let Some(redis_conn) = redis_conn_for_state {
        edge_state.set_redis(redis_conn);
    }

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

        if let Some(ref db) = db_store {
            match reload_routes_from_database(&edge_state, db).await {
                Ok(()) => info!("loaded routes from database"),
                Err(e) => warn!(%e, "failed to load routes from database"),
            }
        }

        if let Some(ref nats_url) = edge_config.nats_url {
            spawn_route_reload_listener(
                nats_url.clone(),
                Arc::clone(&edge_state),
                db_store.clone(),
            );
        }

        if edge_config.gateway_health_checks_enabled {
            if let Some(ref db) = db_store {
                match db.load_gateway_health_list().await {
                    Ok(health_list) => {
                        let mut health = edge_state
                            .gateway_health
                            .lock()
                            .unwrap_or_else(|e| e.into_inner());
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

    match create_tls_acceptor(
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
                            acceptor,
                            Arc::clone(&edge_state),
                            Arc::clone(&edge_config),
                        );
                    }
                    Err(e) => {
                        warn!(%tls_addr, error = %e, "failed to start TLS listener");
                    }
                }
            }
        }
        Ok(None) => {
            info!("SIP TLS listener disabled; configure TLS cert/key paths in config.yaml");
        }
        Err(e) => {
            warn!(error = %e, "failed to create TLS acceptor; TLS listener disabled");
        }
    }

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

    // Start NAT keepalive background loop — sends keepalive probes to active registrations
    let num_workers = if edge_config.udp_workers_auto {
        num_cpus::get().max(1)
    } else {
        edge_config.udp_workers.max(1)
    };
    let queue_capacity = 10000;
    let mut worker_txs = Vec::new();

    for worker_id in 0..num_workers {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<(Vec<u8>, SocketAddr)>(queue_capacity);
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
                                SipMessage::Request(request)
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
    if edge_config.gateway_health_checks_enabled {
        spawn_gateway_health_probe_loop(
            Arc::clone(&edge_state),
            Arc::clone(&socket),
            Arc::clone(&edge_config),
        );
    }
    if let Some(ref db) = db_store {
        spawn_periodic_route_refresh(Arc::clone(&edge_state), db.clone());
    }

    let mut buffer = [0u8; 65_535];
    let mut shutdown_check_interval = tokio::time::interval(Duration::from_millis(500));
    let mut is_draining = false;
    let shutdown_timeout = tokio::time::sleep(Duration::from_secs(999999));
    tokio::pin!(shutdown_timeout);

    loop {
        tokio::select! {
            result = socket.recv_from(&mut buffer) => {
                let (size, peer) = result?;
                let packet = buffer[..size].to_vec();

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

                if worker_txs[worker_idx].try_send((packet.clone(), peer)).is_err() {
                    let _ = worker_txs[worker_idx].send((packet, peer)).await;
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
                edge_state.draining.store(true, Ordering::Relaxed);
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

    Ok(())
}

fn init_tracing() {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("sip_edge=info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

async fn cdr_sinks_from_config(config: &EdgeConfig) -> Result<CdrSinks, AnyError> {
    let postgres = match &config.database_url {
        Some(database_url) if !database_url.trim().is_empty() => {
            let store =
                PostgresCdrStore::connect(database_url, config.database_max_connections).await?;
            info!("PostgreSQL CDR persistence enabled");
            Some(store)
        }
        _ => {
            return Err("PostgreSQL 数据库连接未配置，数据库为系统运行的必须依赖项".into());
        }
    };

    Ok(CdrSinks { postgres })
}

async fn flush_cdr_batch(
    cdr_sinks: &CdrSinks,
    cdrs: &[call_core::CallCdr],
) -> Result<(), AnyError> {
    if cdrs.is_empty() {
        return Ok(());
    }

    if let Some(cdr_store) = &cdr_sinks.postgres {
        for cdr in cdrs {
            cdr_store.insert_call_cdr(cdr).await?;
        }
        debug!(count = cdrs.len(), "persisted batch CDRs to PostgreSQL");
        return Ok(());
    }

    debug!(count = cdrs.len(), "discarded batch CDRs without CDR sink");
    Ok(())
}

async fn flush_cdr_batch_with_retry_and_wal(cdr_sinks: &CdrSinks, batch: &[call_core::CallCdr]) {
    if batch.is_empty() {
        return;
    }

    let mut success = false;
    for attempt in 1..=3 {
        match flush_cdr_batch(cdr_sinks, batch).await {
            Ok(_) => {
                success = true;
                break;
            }
            Err(e) => {
                warn!(attempt, error = %e, "批量发送 CDR 失败，正在重试...");
                tokio::time::sleep(Duration::from_millis(1000)).await;
            }
        }
    }

    if !success {
        tracing::error!("致命错误: 连续 3 次批量刷新 CDR 失败！为防止数据丢失，正将 CDR 数据追加写入本地 logs/cdr_dlq.jsonl 死信归档...");

        let _ = tokio::fs::create_dir_all("logs").await;
        if let Ok(mut file) = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("logs/cdr_dlq.jsonl")
            .await
        {
            use tokio::io::AsyncWriteExt;
            for cdr in batch {
                if let Ok(json_str) = serde_json::to_string(cdr) {
                    let _ = file.write_all(format!("{}\n", json_str).as_bytes()).await;
                }
            }
            let _ = file.flush().await;
            info!(
                count = batch.len(),
                "已成功将未送达的批量 CDR 追加归档至本地 logs/cdr_dlq.jsonl 中"
            );
        } else {
            tracing::error!(
                count = batch.len(),
                "极其严重: 写入本地 logs/cdr_dlq.jsonl 文件失败！CDR 数据将丢弃！"
            );
        }
    }
}

async fn refresh_anti_fraud_rules(edge_state: &EdgeState) {
    if let Some(ref db) = edge_state.db_store {
        match db.list_anti_fraud_rules().await {
            Ok(rules) => {
                let enabled_rules: Vec<_> = rules.into_iter().filter(|r| r.enabled).collect();
                let count = enabled_rules.len();
                let mut guard = edge_state
                    .anti_fraud_rules
                    .write()
                    .unwrap_or_else(|e| e.into_inner());
                *guard = enabled_rules;
                info!(count, "已成功刷新防盗打控制规则缓存");
            }
            Err(e) => warn!("无法从数据库加载防盗打规则: {}", e),
        }
    }
}

/// Reload routes from database, applying time-window filtering.
/// Used by both initial startup and NATS hot-reload.
async fn reload_routes_from_database(
    edge_state: &EdgeState,
    db: &PostgresCdrStore,
) -> Result<(), AnyError> {
    let db_routes = db.load_routes().await?;
    let db_gateways = db.load_gateways().await?;
    let gateway_map: HashMap<String, (String, Option<u16>, String, Option<u32>)> = db_gateways
        .into_iter()
        .map(|(id, host, port, transport, cap, _, _, _)| (id, (host, port, transport, cap)))
        .collect();

    let mut routes = Vec::new();
    let now_hhmm = cdr_core::current_hhmm();
    for (id, prefix, priority, gateway_id, cost, weight, time_start, time_end) in db_routes {
        if let (Some(start), Some(end)) = (time_start.as_ref(), time_end.as_ref()) {
            if let Some(now) = now_hhmm.as_deref() {
                if now < start.as_str() || now > end.as_str() {
                    continue;
                }
            }
        }
        if let Some((host, port, _transport, max_capacity)) = gateway_map.get(&gateway_id) {
            let mut target = RouteTarget::new(&gateway_id, host.clone(), *port);
            target.max_capacity = *max_capacity;
            routes.push(Route::with_cost_and_weight(
                id,
                prefix,
                priority as u16,
                cost,
                weight.max(0) as u32,
                target,
            ));
        }
    }
    if !routes.is_empty() {
        edge_state
            .call_manager
            .update_routes(RouteTable::new(routes));
    }
    Ok(())
}

fn spawn_route_reload_listener(
    nats_url: String,
    edge_state: Arc<EdgeState>,
    db_store: Option<PostgresCdrStore>,
) {
    tokio::spawn(async move {
        let Ok(client) = async_nats::connect(&nats_url).await else {
            warn!("路由重载器无法连接到 NATS");
            return;
        };

        let Ok(mut subscriber) = client.subscribe("vos_rs.routing.reload").await else {
            warn!("路由重载器无法订阅 NATS 主题");
            return;
        };

        info!("已成功启动动态路由热加载监听协程");
        while let Some(_msg) = subscriber.next().await {
            info!("收到路由热加载 NATS 广播通知，正在从数据库刷新路由...");
            if let Some(ref db) = db_store {
                match reload_routes_from_database(&edge_state, db).await {
                    Ok(()) => {
                        refresh_anti_fraud_rules(&edge_state).await;
                        info!("动态路由热重载成功，已加载最新路由表数据！");
                    }
                    Err(e) => warn!("热加载路由失败: {}", e),
                }
            }
        }
    });
}

/// Periodically reload routes from database to support time-based routing.
/// Routes with time_start/time_end are filtered at load time, so this ensures
/// they are automatically enabled/disabled as time windows pass.
fn spawn_periodic_route_refresh(edge_state: Arc<EdgeState>, db: PostgresCdrStore) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        interval.tick().await; // skip first immediate tick
        loop {
            interval.tick().await;
            if let Err(e) = reload_routes_from_database(&edge_state, &db).await {
                warn!(%e, "periodic route refresh failed");
            }
        }
    });
}

fn route_table_from_config(config: &EdgeConfig) -> Result<RouteTable, AnyError> {
    if config.default_gateway.is_empty() {
        return Ok(RouteTable::default());
    }

    let target = parse_gateway_target("default", &config.default_gateway)?;
    Ok(RouteTable::new(vec![Route::new(
        "default", "", 100, target,
    )]))
}

fn parse_gateway_target(gateway_id: &str, raw: &str) -> Result<RouteTarget, AnyError> {
    let value = raw.trim();
    if value.is_empty() {
        return Err(Box::new(io::Error::new(
            io::ErrorKind::InvalidInput,
            "sip_edge.routing.default_gateway must not be empty",
        )));
    }

    let uri = if value.starts_with("sip:") || value.starts_with("sips:") {
        SipUri::from_str(value)
    } else {
        SipUri::from_str(&format!("sip:{value}"))
    }
    .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error.to_string()))?;

    Ok(RouteTarget::new(gateway_id, uri.host, uri.port))
}

/// 快速从原始 SIP 字节中提取 Call-ID 值（无需完整解析）。
///
/// 按行扫描报文头部，匹配 `Call-ID:` 或紧凑形式 `i:`，
/// 提取其值用于 Worker 路由哈希，确保同一 Dialog 的所有消息
/// 始终路由到同一个处理 Worker，避免并发竞态。
///
/// # 返回
/// - `Some(call_id)` — 成功提取
/// - `None` — 报文格式异常或不含 Call-ID（fallback 到 peer-IP 哈希）
fn extract_call_id_fast(packet: &[u8]) -> Option<&[u8]> {
    // SIP 消息头部为 ASCII，每行以 CRLF 或 LF 结尾
    let text = std::str::from_utf8(packet).ok()?;

    // 跳过请求行或状态行（第一行）
    let headers_start = text.find('\n').map(|i| i + 1)?;
    let headers = &text[headers_start..];

    // 遍历每一行，匹配 Call-ID 头（大小写不敏感）
    for line in headers.lines() {
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            // 空行：头部结束
            break;
        }
        // 匹配 "Call-ID:" 及紧凑形式 "i:"
        let value = if trimmed.len() > 8 && trimmed[..8].eq_ignore_ascii_case("call-id:") {
            trimmed[8..].trim()
        } else if trimmed.len() > 2 && trimmed[..2].eq_ignore_ascii_case("i:") {
            trimmed[2..].trim()
        } else {
            continue;
        };

        if value.is_empty() {
            return None;
        }
        // 在原始 packet 中找到 value 的位置并返回字节切片（零拷贝）
        if let Some(pos) = packet
            .windows(value.len())
            .position(|w| w == value.as_bytes())
        {
            return Some(&packet[pos..pos + value.len()]);
        }
        return None;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::auth::{digest_response, AuthConfig};
    use super::{
        flush_cdr_batch, handle_datagram, media, response, spawn_client_transaction_retransmission,
        spawn_nat_keepalive_loop, spawn_session_timer_watchdog, CdrSinks, ClientTransactionKey,
        EdgeConfig, EdgeState,
    };
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
