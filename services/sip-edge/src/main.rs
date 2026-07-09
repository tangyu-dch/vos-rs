pub(crate) mod config;
pub(crate) mod edge_state;
pub(crate) mod handlers;
mod manage;
pub(crate) mod media;
pub(crate) mod nats_cdr;
pub(crate) mod net;
pub(crate) mod security;
pub(crate) mod sip;
pub(crate) mod timers;

// Re-export for backward compatibility with inline module references
pub(crate) use edge_state::*;
pub(crate) use net::stun_client;
pub(crate) use net::transport;
pub(crate) use net::upnp;
pub(crate) use security::sbc;
pub(crate) use sip::auth;
pub(crate) use sip::dialog;
pub(crate) use sip::outbound;
pub(crate) use sip::registrar::{self, RegisterOutcome};
pub(crate) use sip::response;
pub(crate) use sip::transaction;

// Constants re-exported for tests
pub(crate) use config::{
    DATABASE_URL_ENV, DEFAULT_GATEWAY_ENV, NATS_CDR_STREAM_ENV, NATS_CDR_SUBJECT_ENV,
    NATS_URL_ENV, TLS_BIND_ENV, TLS_CERT_PATH_ENV, TLS_KEY_PATH_ENV,
};
pub(crate) use timers::{
    calculate_mos_for_legs, spawn_client_transaction_retransmission, spawn_nat_keepalive_loop,
    spawn_session_timer_watchdog,
};

use call_core::{CallError, CallManager, Route, RouteTable, RouteTarget};
use cdr_core::{PostgresCdrStore, DEFAULT_CDR_STREAM, DEFAULT_CDR_SUBJECT};
use config::EdgeConfig;
use media::{MediaConfig, MediaRelayState};
use nats_cdr::NatsCdrPublisher;
use net::{
    create_tls_acceptor, handle_stream_connection, handle_ws_connection, SipStream, Transport,
};
use sdp_core::RtpEndpoint;
use sip::{
    AuthConfig, AuthDecision, ClientTransactionKey, DialogValidationError, RequestTransactionKey,
};
use sip_core::{
    parse_message, HeaderMap, HeaderName, HeaderValue, Method, SipMessage, SipRequest, SipUri,
};
use std::{
    collections::HashMap,
    env, io,
    net::SocketAddr,
    str::FromStr,
    sync::{atomic::Ordering, Arc},
    time::{Duration, Instant, SystemTime},
};
use tokio::net::UdpSocket;
use tracing::{debug, error, info, warn};
use tracing_subscriber::EnvFilter;


type AnyError = Box<dyn std::error::Error + Send + Sync>;

fn current_hhmm() -> Option<String> {
    let fmt = time::format_description::parse("[hour]:[minute]").ok()?;
    time::OffsetDateTime::now_utc().format(&fmt).ok()
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), AnyError> {
    init_tracing();

    let bind_addr =
        env::var("VOS_RS_SIP_UDP_BIND").unwrap_or_else(|_| "0.0.0.0:5060".to_string());
    let route_table = route_table_from_env()?;
    if route_table.is_empty() {
        warn!(
            "no outbound route configured; INVITE requests will receive 404"
        );
    }
    let mut edge_config = EdgeConfig::from_env();

    // STUN: discover public address for media relay if configured
    if let Ok(stun_server) = env::var("VOS_RS_STUN_SERVER") {
        if !stun_server.is_empty() {
            info!(server = %stun_server, "STUN discovery enabled");
            let fallback = edge_config.media.advertised_addr.clone();
            let public_ip = stun_client::discover_stun_addr(Some(&stun_server), &fallback).await;
            edge_config.media.set_advertised_addr(public_ip);

            // Background STUN keepalive: reuse one socket for consistent NAT mapping
            let stun_server_clone = stun_server.clone();
            tokio::spawn(async move {
                let server_addr = match tokio::net::lookup_host(&stun_server_clone).await {
                    Ok(mut addrs) => match addrs.next() {
                        Some(a) => a,
                        None => {
                            warn!("STUN keepalive: DNS lookup failed, stopping");
                            return;
                        }
                    },
                    Err(e) => {
                        warn!(error = %e, "STUN keepalive: DNS lookup failed, stopping");
                        return;
                    }
                };
                let sock = match tokio::net::UdpSocket::bind("0.0.0.0:0").await {
                    Ok(s) => s,
                    Err(e) => {
                        warn!(error = %e, "STUN keepalive: bind failed, stopping");
                        return;
                    }
                };
                let _ = sock.connect(server_addr).await;
                let mut interval = tokio::time::interval(Duration::from_secs(30));
                interval.tick().await;
                loop {
                    interval.tick().await;
                    // Minimal STUN Binding Request: 20 bytes
                    let mut req = [0u8; 20];
                    req[0] = 0x00;
                    req[1] = 0x01; // BINDING
                    req[2] = 0x00;
                    req[3] = 0x08; // length = 8
                    req[4] = 0x21;
                    req[5] = 0x12;
                    req[6] = 0xa4;
                    req[7] = 0x42; // magic cookie
                    let _ = sock.send(&req).await;
                    let mut buf = [0u8; 1500];
                    let _ = tokio::time::timeout(Duration::from_secs(3), sock.recv(&mut buf)).await;
                    debug!("STUN keepalive sent");
                }
            });
        }
    }

    // UPnP: auto-discover router and add port mappings if enabled
    let upnp_enabled = env::var("VOS_RS_UPNP_ENABLED")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false);

    if upnp_enabled {
        info!("UPnP port mapping enabled, discovering gateway...");
        if let Some(gw) = upnp::discover_gateway() {
            if let Some(ext_ip) = upnp::get_external_ip(&gw) {
                info!(external_ip = %ext_ip, "UPnP: router external IP");

                // Map SIP UDP port (5060)
                let sip_port: u16 = bind_addr
                    .parse::<SocketAddr>()
                    .map(|a| a.port())
                    .unwrap_or(5060);
                upnp::add_port_mapping(&gw, sip_port, sip_port, "UDP", "sip-edge SIP UDP", 3600);
                upnp::add_port_mapping(&gw, sip_port, sip_port, "TCP", "sip-edge SIP TCP", 3600);

                // Map RTP port range
                let rtp_min = edge_config.media.port_min;
                let rtp_max = edge_config.media.port_max;
                for port in (rtp_min..=rtp_max).step_by(2) {
                    upnp::add_port_mapping(&gw, port, port, "UDP", "sip-edge RTP", 3600);
                }

                // Periodic UPnP renewal (every 30 minutes, lease is 3600s = 1h)
                let gw_clone = upnp::UpnpGateway {
                    control_url: gw.control_url.clone(),
                    local_ip: gw.local_ip.clone(),
                    service_type: gw.service_type.clone(),
                };
                let sip_port_renew = sip_port;
                let rtp_min_renew = rtp_min;
                let rtp_max_renew = rtp_max;
                tokio::spawn(async move {
                    let mut interval = tokio::time::interval(std::time::Duration::from_secs(1800));
                    interval.tick().await;
                    loop {
                        interval.tick().await;
                        upnp::add_port_mapping(
                            &gw_clone,
                            sip_port_renew,
                            sip_port_renew,
                            "UDP",
                            "sip-edge SIP UDP",
                            3600,
                        );
                        upnp::add_port_mapping(
                            &gw_clone,
                            sip_port_renew,
                            sip_port_renew,
                            "TCP",
                            "sip-edge SIP TCP",
                            3600,
                        );
                        for port in (rtp_min_renew..=rtp_min_renew.min(rtp_max_renew)).step_by(2) {
                            upnp::add_port_mapping(
                                &gw_clone,
                                port,
                                port,
                                "UDP",
                                "sip-edge RTP",
                                3600,
                            );
                        }
                        debug!("UPnP: port mappings renewed");
                    }
                });
            }
        } else {
            warn!("UPnP: no gateway found on network, port mapping disabled");
        }
    }

    // Wrap EdgeConfig in Arc — all mutations done, now read-only shared access
    let edge_config = Arc::new(edge_config);

    let media_relay = MediaRelayState::new();
    let cdr_sinks = cdr_sinks_from_env().await?;
    let db_store = cdr_sinks.postgres.clone();
    let cdr_sinks = std::sync::Arc::new(cdr_sinks);

    let edge_state = Arc::new(EdgeState::with_media_relay_and_db(
        CallManager::new(route_table),
        media_relay.clone(),
        db_store.clone(),
    ));

    // 启动管理 API（活跃呼叫查询 / 强制拆线）
    let manage_addr =
        env::var("VOS_RS_MANAGE_BIND").unwrap_or_else(|_| "127.0.0.1:8082".to_string());
    {
        let manage_state = Arc::clone(&edge_state);
        let addr = manage_addr.clone();
        tokio::spawn(async move {
            manage::serve(addr, manage_state).await;
        });
    }

    if let Some(db) = &db_store {
        let has_users = sqlx::query("SELECT 1 FROM sip_users LIMIT 1")
            .fetch_optional(db.pool())
            .await?
            .is_some();
        if !has_users {
            if let Ok(raw_users) = env::var("VOS_RS_SIP_AUTH_USERS") {
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
        if !has_gateways {
            if let Ok(raw_gateway) = env::var("VOS_RS_SIP_DEFAULT_GATEWAY") {
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
        }

        let db_routes = db.load_routes().await?;
        let db_gateways = db.load_gateways().await?;
        let gateway_map: HashMap<String, (String, Option<u16>, String, Option<u32>)> = db_gateways
            .into_iter()
            .map(|(id, host, port, transport, cap, _, _, _)| (id, (host, port, transport, cap)))
            .collect();

        let mut routes = Vec::new();
        let now_hhmm = current_hhmm();
        for (id, prefix, priority, gateway_id, cost, time_start, time_end) in db_routes {
            // 时间路由：配置了 time_start/time_end 且当前不在窗口内则跳过
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
                routes.push(Route::with_cost(id, prefix, priority as u16, cost, target));
            }
        }
        if !routes.is_empty() {
            edge_state
                .call_manager
                .update_routes(RouteTable::new(routes));
            info!("loaded routes from database");
        }
    }

    let socket: Arc<UdpSocket> = Arc::new(UdpSocket::bind(&bind_addr).await?);

    // Increase UDP receive buffer for high CPS (default ~786KB, set to 4MB)
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        let fd = socket.as_raw_fd();
        let buf_size: i32 = 4 * 1024 * 1024; // 4MB
        unsafe {
            libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_RCVBUF,
                &buf_size as *const i32 as *const libc::c_void,
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
        let edge_state_clone = Arc::clone(&edge_state);
        let edge_config_clone = edge_config.clone();
        tokio::spawn(async move {
            loop {
                match l.accept().await {
                    Ok((stream, peer)) => {
                        debug!(%peer, "accepted TCP connection");
                        let (tx, rx) = tokio::sync::mpsc::channel(100);
                        edge_state_clone.register_tcp_connection(peer, tx.clone());

                        let state_clone = Arc::clone(&edge_state_clone);
                        let config_clone = edge_config_clone.clone();
                        tokio::spawn(handle_stream_connection(
                            SipStream::Tcp(stream),
                            peer,
                            tx,
                            rx,
                            move |msg_bytes, peer_addr, connection_tx| {
                                let state = Arc::clone(&state_clone);
                                let config = config_clone.clone();
                                async move {
                                    let datagrams =
                                        handle_datagram(&msg_bytes, peer_addr, &state, &config)
                                            .await;
                                    for datagram in datagrams {
                                        let _ = connection_tx.send(datagram.bytes).await;
                                    }
                                }
                            },
                        ));
                    }
                    Err(e) => {
                        error!(error = %e, "TCP accept error");
                    }
                }
            }
        });
    }

    // Start TLS listener (default derived port 5061) only when TLS material is configured.
    let tls_bind_addr = env_non_empty(TLS_BIND_ENV)
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
                        let edge_state_clone = Arc::clone(&edge_state);
                        let edge_config_clone = edge_config.clone();
                        tokio::spawn(async move {
                            loop {
                                match l.accept().await {
                                    Ok((stream, peer)) => {
                                        let acceptor_clone = acceptor.clone();
                                        let edge_state_clone_inner = Arc::clone(&edge_state_clone);
                                        let edge_config_clone_inner = edge_config_clone.clone();
                                        tokio::spawn(async move {
                                            match acceptor_clone.accept(stream).await {
                                                Ok(tls_stream) => {
                                                    debug!(%peer, "accepted TLS handshake");
                                                    let (tx, rx) = tokio::sync::mpsc::channel(100);
                                                    edge_state_clone_inner
                                                        .register_tcp_connection(peer, tx.clone());

                                                    let state_clone =
                                                        Arc::clone(&edge_state_clone_inner);
                                                    let config_clone =
                                                        edge_config_clone_inner.clone();
                                                    handle_stream_connection(
                                                        SipStream::TlsServer(tls_stream),
                                                        peer,
                                                        tx,
                                                        rx,
                                                        move |msg_bytes, peer_addr, connection_tx| {
                                                            let state = Arc::clone(&state_clone);
                                                            let config = config_clone.clone();
                                                            async move {
                                                                let datagrams = handle_datagram(
                                                                    &msg_bytes, peer_addr, &state,
                                                                    &config,
                                                                )
                                                                .await;
                                                                for datagram in datagrams {
                                                                    let _ = connection_tx
                                                                        .send(datagram.bytes)
                                                                        .await;
                                                                }
                                                            }
                                                        },
                                                    )
                                                    .await;
                                                }
                                                Err(e) => {
                                                    warn!(%peer, error = %e, "TLS handshake accept failed");
                                                }
                                            }
                                        });
                                    }
                                    Err(e) => {
                                        error!(error = %e, "TLS accept error");
                                    }
                                }
                            }
                        });
                    }
                    Err(e) => {
                        warn!(%tls_addr, error = %e, "failed to start TLS listener");
                    }
                }
            }
        }
        Ok(None) => {
            info!(
                cert_env = TLS_CERT_PATH_ENV,
                key_env = TLS_KEY_PATH_ENV,
                "SIP TLS listener disabled; configure cert/key paths to enable"
            );
        }
        Err(e) => {
            warn!(error = %e, "failed to create TLS acceptor; TLS listener disabled");
        }
    }

    // Start WebSocket listener
    let ws_bind_addr = env::var("VOS_RS_SIP_WS_BIND").unwrap_or_else(|_| {
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
        let edge_state_clone = Arc::clone(&edge_state);
        let edge_config_clone = edge_config.clone();
        tokio::spawn(async move {
            loop {
                match ws_listener.accept().await {
                    Ok((stream, peer)) => {
                        debug!(%peer, "accepted WebSocket TCP connection");
                        let state_clone = Arc::clone(&edge_state_clone);
                        let config_clone = edge_config_clone.clone();
                        tokio::spawn(async move {
                            match tokio_tungstenite::accept_async(stream).await {
                                Ok(ws_stream) => {
                                    debug!(%peer, "WebSocket handshake succeeded");
                                    let (tx, rx) = tokio::sync::mpsc::channel(100);
                                    state_clone.register_tcp_connection(peer, tx.clone());

                                    let on_msg_state = Arc::clone(&state_clone);
                                    let on_msg_config = config_clone.clone();

                                    handle_ws_connection(
                                        ws_stream,
                                        peer,
                                        tx,
                                        rx,
                                        move |msg_bytes, peer_addr, connection_tx| {
                                            let state = Arc::clone(&on_msg_state);
                                            let config = on_msg_config.clone();
                                            async move {
                                                let datagrams = handle_datagram(
                                                    &msg_bytes, peer_addr, &state, &config,
                                                )
                                                .await;
                                                for d in datagrams {
                                                    let _ = connection_tx.send(d.bytes).await;
                                                }
                                            }
                                        },
                                    )
                                    .await;
                                }
                                Err(e) => {
                                    warn!(%peer, error = %e, "WebSocket handshake failed");
                                }
                            }
                        });
                    }
                    Err(e) => {
                        error!(error = %e, "WebSocket accept error");
                    }
                }
            }
        });
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
    spawn_nat_keepalive_loop(Arc::clone(&edge_state), Arc::clone(&socket));

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

                let state = Arc::clone(&edge_state);
                let sock = Arc::clone(&socket);
                let cfg = edge_config.clone();
                let cdr_sinks_clone = cdr_sinks.clone();

                tokio::spawn(async move {
                    let datagrams = handle_datagram(&packet, peer, &state, &cfg).await;
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

                        if let Err(error) = state.send_sip_datagram(datagram.clone(), &sock, &cfg).await {
                            warn!(target = %datagram.target, error = %error, "failed to send SIP message");
                        } else {
                            debug!(
                                peer = %datagram.target,
                                bytes = datagram.bytes.len(),
                                "sent SIP datagram"
                            );

                            if transport == Transport::Udp {
                                if let Ok(SipMessage::Request(req)) = sip_core::parse_message(&datagram.bytes) {
                                    if !matches!(&req.method, Method::Ack) {
                                        if let Some(key) = ClientTransactionKey::from_request(&req) {
                                            if !state.client_transactions.contains_key(&key) {
                                                spawn_client_transaction_retransmission(
                                                    Arc::clone(&state),
                                                    Arc::clone(&sock),
                                                    datagram.target.clone(),
                                                    datagram.bytes.clone(),
                                                    key,
                                                    cfg.clone(),
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if let Err(error) = flush_completed_cdrs(&cdr_sinks_clone, &state).await {
                        warn!(%error, "failed to flush completed CDRs");
                    }
                });

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

fn env_non_empty(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}


async fn cdr_sinks_from_env() -> Result<CdrSinks, AnyError> {
    let nats = match env::var(NATS_URL_ENV) {
        Ok(nats_url) => {
            let subject =
                env::var(NATS_CDR_SUBJECT_ENV).unwrap_or_else(|_| DEFAULT_CDR_SUBJECT.to_string());
            let stream =
                env::var(NATS_CDR_STREAM_ENV).unwrap_or_else(|_| DEFAULT_CDR_STREAM.to_string());
            let publisher =
                NatsCdrPublisher::connect(&nats_url, subject.clone(), stream.clone()).await?;
            info!(subject, stream, "NATS JetStream CDR queue enabled");
            Some(publisher)
        }
        Err(_) => {
            info!(
                env = NATS_URL_ENV,
                "NATS CDR queue disabled; set env var to enable"
            );
            None
        }
    };

    let postgres = match env::var(DATABASE_URL_ENV) {
        Ok(database_url) => {
            let store = PostgresCdrStore::connect(&database_url).await?;
            if nats.is_some() {
                info!("PostgreSQL direct CDR persistence disabled because NATS CDR queue is enabled (database connection will still be used for configuration and registration store)");
            } else {
                info!("PostgreSQL CDR persistence enabled");
            }
            Some(store)
        }
        Err(_) => {
            info!(
                env = DATABASE_URL_ENV,
                "PostgreSQL database connection disabled; set env var to enable"
            );
            None
        }
    };

    Ok(CdrSinks { postgres, nats })
}

async fn flush_completed_cdrs(
    cdr_sinks: &CdrSinks,
    edge_state: &EdgeState,
) -> Result<(), AnyError> {
    let cdrs = edge_state.call_manager.completed_cdrs().to_vec();

    if cdrs.is_empty() {
        return Ok(());
    }

    if let Some(nats) = &cdr_sinks.nats {
        for cdr in &cdrs {
            nats.publish_cdr(cdr).await?;
        }

        let queued = edge_state.call_manager.take_completed_cdrs().len();
        debug!(count = queued, "queued completed CDRs to NATS");
        return Ok(());
    }

    if let Some(cdr_store) = &cdr_sinks.postgres {
        for cdr in &cdrs {
            cdr_store.insert_call_cdr(cdr).await?;
        }

        let persisted = edge_state.call_manager.take_completed_cdrs().len();
        debug!(count = persisted, "persisted completed CDRs to PostgreSQL");
        return Ok(());
    }

    {
        let dropped = edge_state.call_manager.take_completed_cdrs().len();
        debug!(count = dropped, "discarded completed CDRs without CDR sink");
    }
    Ok(())
}

fn route_table_from_env() -> Result<RouteTable, AnyError> {
    let Ok(gateway) = env::var(DEFAULT_GATEWAY_ENV) else {
        return Ok(RouteTable::default());
    };

    let target = parse_gateway_target("default", &gateway)?;
    Ok(RouteTable::new(vec![Route::new(
        "default", "", 100, target,
    )]))
}

fn parse_gateway_target(gateway_id: &str, raw: &str) -> Result<RouteTarget, AnyError> {
    let value = raw.trim();
    if value.is_empty() {
        return Err(Box::new(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{DEFAULT_GATEWAY_ENV} must not be empty"),
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

pub(crate) async fn handle_datagram(
    packet: &[u8],
    peer: SocketAddr,
    edge_state: &EdgeState,
    edge_config: &EdgeConfig,
) -> Vec<PendingDatagram> {
    if !edge_state.sbc_engine.is_allowed(peer.ip()) {
        warn!(%peer, "packet blocked by SBC IP ACL");
        return Vec::new();
    }

    if !edge_state.sbc_engine.check_rate(peer.ip()) {
        warn!(%peer, "packet blocked by SBC rate limit");
        if let Ok(SipMessage::Request(req)) = parse_message(packet) {
            return vec![PendingDatagram::new(
                peer.to_string(),
                response::build_response_with_owned_headers(
                    &req,
                    503,
                    "Service Unavailable - Rate Limit Exceeded",
                    &[("Retry-After".to_string(), "10".to_string())],
                    "",
                ),
            )];
        }
        return Vec::new();
    }

    match parse_message(packet) {
        Ok(SipMessage::Request(request)) => {
            info!(method = %request.method, uri = %request.uri, "received SIP request");

            let transaction_key = RequestTransactionKey::from_request(&request, peer);
            let has_socket = edge_state.get_socket().is_some();
            if !has_socket {
                if let Some(ref key) = transaction_key {
                    if let Some(cached) = edge_state.test_request_cache.get(key) {
                        debug!(%peer, method = %request.method, "replaying cached test response");
                        return cached.clone();
                    }
                }
            } else if let Some(ref key) = transaction_key {
                if let Some(tx) = edge_state.get_server_transaction(key) {
                    debug!(%peer, method = %request.method, "feeding duplicate request to active Server Transaction");
                    let _ = tx
                        .send(transaction::ServerTransactionEvent::Request(
                            request.clone(),
                        ))
                        .await;
                    return Vec::new();
                }
            }

            let is_ack = matches!(&request.method, Method::Ack);
            if is_ack {
                let ack_branch = request
                    .headers
                    .get("via")
                    .and_then(|v| transaction::branch_param(v.as_str()));
                let ack_call_id = request
                    .headers
                    .get("call-id")
                    .map(|v| v.as_str().to_string());
                let ack_cseq_num = request
                    .headers
                    .get("cseq")
                    .and_then(|v| v.as_str().split_whitespace().next().map(|s| s.to_string()));
                let invite_key = RequestTransactionKey::new_manual(
                    peer.to_string(),
                    "INVITE".to_string(),
                    ack_branch,
                    ack_call_id,
                    ack_cseq_num.map(|num| format!("{} INVITE", num)),
                );
                if let Some(tx) = edge_state.get_server_transaction(&invite_key) {
                    let _ = tx
                        .send(transaction::ServerTransactionEvent::Ack(request.clone()))
                        .await;
                }
            }

            let datagrams = handle_request(request.clone(), peer, edge_state, edge_config).await;

            if is_ack {
                return datagrams;
            }

            if !has_socket {
                if let Some(ref key) = transaction_key {
                    let peer_str = peer.to_string();
                    let peer_resps: Vec<PendingDatagram> = datagrams
                        .iter()
                        .filter(|d| d.target == peer_str)
                        .cloned()
                        .collect();
                    if !peer_resps.is_empty() {
                        edge_state
                            .test_request_cache
                            .insert(key.clone(), peer_resps);
                    }
                }
                return datagrams;
            }

            let has_socket = edge_state.get_socket().is_some();
            let mut final_datagrams = Vec::new();
            if let (true, Some(key)) = (has_socket, transaction_key) {
                let peer_str = peer.to_string();
                let mut peer_resps = Vec::new();

                for datagram in datagrams {
                    if datagram.target == peer_str {
                        peer_resps.push(datagram.bytes);
                    } else {
                        final_datagrams.push(datagram);
                    }
                }

                if !peer_resps.is_empty() {
                    let is_invite = request.method == Method::Invite;
                    if is_invite {
                        let has_2xx = peer_resps.iter().any(|resp| resp.starts_with(b"SIP/2.0 2"));
                        if has_2xx {
                            for resp in peer_resps {
                                final_datagrams.push(PendingDatagram::new(peer_str.clone(), resp));
                            }
                        } else {
                            let (tx, rx) = tokio::sync::mpsc::channel(4);
                            edge_state.register_server_transaction(key.clone(), tx.clone());
                            transaction::spawn_invite_server_transaction(
                                key,
                                request,
                                peer,
                                edge_state.get_socket(),
                                rx,
                            );
                            for resp in peer_resps {
                                let _ = tx
                                    .send(transaction::ServerTransactionEvent::Response(resp))
                                    .await;
                            }
                        }
                    } else {
                        let (tx, rx) = tokio::sync::mpsc::channel(16);
                        edge_state.register_server_transaction(key.clone(), tx.clone());
                        transaction::spawn_non_invite_server_transaction(
                            key,
                            request,
                            peer,
                            edge_state.get_socket(),
                            rx,
                        );
                        for resp in peer_resps {
                            let _ = tx
                                .send(transaction::ServerTransactionEvent::Response(resp))
                                .await;
                        }
                    }
                }
            } else {
                final_datagrams = datagrams;
            }

            final_datagrams
        }
        Ok(SipMessage::Response(mut sip_response)) => {
            let is_self_refresh = sip_response
                .headers
                .get_all("via")
                .any(|v| v.as_str().contains("branch=z9hG4bK-refresh-"));

            if is_self_refresh {
                let call_id = sip_response
                    .headers
                    .get("call-id")
                    .map(|v| v.as_str().to_string());
                if sip_response.status_code >= 200 && sip_response.status_code < 300 {
                    if let Some(ref cid) = call_id {
                        if let Some(mut t_mut) = edge_state.inbound_transactions.get_mut(cid) {
                            t_mut.last_session_refresh = Some(std::time::Instant::now());
                            debug!(call_id = %cid, "received 200 OK for self-generated session refresh");
                        }
                    }
                } else if sip_response.status_code >= 300 {
                    warn!(
                        call_id = ?call_id,
                        status = sip_response.status_code,
                        "self-generated session refresh request failed"
                    );
                }
                return Vec::new();
            }

            if let Some(key) = ClientTransactionKey::from_response(&sip_response) {
                edge_state.cancel_client_transaction(&key);
            }
            let call_id = sip_response
                .headers
                .get("call-id")
                .map(|call_id| call_id.as_str().to_string());

            // Topology Hiding: translate external Call-ID (seen by gateway) back to internal
            // Call-ID (used by inbound_transactions map and the caller-facing leg).
            // We also patch sip_response.headers in-place so all downstream code (including
            // call_manager.handle_outbound_response) sees the internal Call-ID.
            let call_id = if let Some(ref cid) = call_id {
                if let Some(internal_cid) = edge_state.get_internal_call_id(cid) {
                    debug!(external_call_id = %cid, internal_call_id = %internal_cid, "topology hiding: translated gateway Call-ID to internal");
                    // Patch the raw response so downstream code never sees the external Call-ID.
                    if let Ok(name) = HeaderName::new("call-id") {
                        sip_response
                            .headers
                            .replace(name, HeaderValue::new(&internal_cid));
                    }
                    Some(internal_cid)
                } else {
                    call_id.clone()
                }
            } else {
                call_id.clone()
            };

            let original_call_id = if let Some(ref cid) = call_id {
                edge_state.refer_transfers.get(cid).map(|r| r.clone())
            } else {
                None
            };

            if let Some(orig_cid) = original_call_id {
                if let Some((_, mut t)) = edge_state.inbound_transactions.remove(&orig_cid) {
                    if let Some(ref mut sub) = t.refer_subscription {
                        sub.notify_cseq += 1;
                        let status_code = sip_response.status_code;
                        let notify_body =
                            format!("SIP/2.0 {} {}\r\n", status_code, sip_response.reason_phrase);

                        let sub_state = if status_code >= 200 {
                            "terminated;reason=noresource"
                        } else {
                            "active;expires=60"
                        };

                        let notify = outbound::build_notify_sipfrag_with_state(
                            &orig_cid,
                            &sub.from_header,
                            &sub.to_header,
                            sub.notify_cseq,
                            &edge_config.advertised_addr,
                            &notify_body,
                            sub_state,
                        );
                        let mut datagrams =
                            vec![PendingDatagram::new(sub.referrer_peer.clone(), notify)];

                        if (200..300).contains(&status_code) {
                            // Successful transfer!
                            // Send BYE to the referrer to terminate the old session.
                            let bye_cseq = t.last_outbound_cseq.unwrap_or(100) + 1;
                            t.last_outbound_cseq = Some(bye_cseq);

                            let to_tag_str = t
                                .inbound_to_tag
                                .as_ref()
                                .map(|s| format!(";tag={}", s))
                                .unwrap_or_default();
                            let from_tag_str = t
                                .inbound_from_tag
                                .as_ref()
                                .map(|s| format!(";tag={}", s))
                                .unwrap_or_default();
                            let from_hdr = format!(
                                "{};tag={}",
                                sub.to_header.split(';').next().unwrap_or(""),
                                to_tag_str
                            );
                            let to_hdr = format!(
                                "{};tag={}",
                                sub.from_header.split(';').next().unwrap_or(""),
                                from_tag_str
                            );

                            let req_uri = t
                                .caller_contact
                                .clone()
                                .unwrap_or_else(|| sip_uri_from_peer(&t.peer));

                            let bye_branch = format!("z9hG4bK-bye-{}-{}", orig_cid, bye_cseq);
                            let bye_bytes = format!(
                                "BYE {req_uri} SIP/2.0\r\n\
                                 Via: SIP/2.0/UDP {addr};branch={bye_branch}\r\n\
                                 Max-Forwards: 70\r\n\
                                 From: {from_hdr}\r\n\
                                 To: {to_hdr}\r\n\
                                 Call-ID: {orig_cid}\r\n\
                                 CSeq: {bye_cseq} BYE\r\n\
                                 Content-Length: 0\r\n\r\n",
                                req_uri = req_uri,
                                addr = edge_config.advertised_addr,
                                bye_branch = bye_branch,
                                from_hdr = from_hdr,
                                to_hdr = to_hdr,
                                orig_cid = orig_cid,
                                bye_cseq = bye_cseq
                            )
                            .into_bytes();

                            datagrams
                                .push(PendingDatagram::new(sub.referrer_peer.clone(), bye_bytes));

                            // Bridge the media! Update transferee target to point to C's media endpoint.
                            if let (Some(target_port), Some(transferee_port)) =
                                (sub.target_relay_port, sub.transferee_relay_port)
                            {
                                if let Ok(c_media_rtp) =
                                    media::parse_sdp_rtp_endpoint(&sip_response.body)
                                {
                                    let target_ep = RtpEndpoint {
                                        address: c_media_rtp.address.clone(),
                                        port: c_media_rtp.port,
                                    };
                                    let transferee_relay_ep = RtpEndpoint {
                                        address: edge_config
                                            .advertised_addr
                                            .split(':')
                                            .next()
                                            .unwrap_or("127.0.0.1")
                                            .to_string(),
                                        port: transferee_port,
                                    };
                                    let _ = edge_state
                                        .media_relay
                                        .set_target(&transferee_relay_ep, &target_ep);

                                    // ALSO set target destination of C's relay port to the transferee's remote media endpoint!
                                    let transferee_is_caller = sub.transferee_relay_port.is_some()
                                        && t.caller_relay_rtp.as_ref().map(|ep| ep.port)
                                            == sub.transferee_relay_port;
                                    let transferee_dest = if transferee_is_caller {
                                        t.caller_rtp.clone()
                                    } else {
                                        t.gateway_rtp.clone()
                                    };
                                    if let Some(dest) = transferee_dest {
                                        let target_relay_ep = RtpEndpoint {
                                            address: edge_config
                                                .advertised_addr
                                                .split(':')
                                                .next()
                                                .unwrap_or("127.0.0.1")
                                                .to_string(),
                                            port: target_port,
                                        };
                                        let _ = edge_state
                                            .media_relay
                                            .set_target(&target_relay_ep, &dest);
                                    }
                                }
                            }

                            // Setup bridged transfer routing fields in InboundTransaction
                            let from_header_val = sip_response
                                .headers
                                .get("from")
                                .map(|v| v.as_str().to_string());
                            let to_header_val = sip_response
                                .headers
                                .get("to")
                                .map(|v| v.as_str().to_string());
                            let contact_val = sip_response
                                .headers
                                .get("contact")
                                .and_then(|v| extract_uri_from_contact(v.as_str()));

                            t.transfer_from_header = from_header_val;
                            t.transfer_to_header = to_header_val;
                            t.transfer_call_id = call_id.clone();
                            t.transfer_contact = contact_val;
                            t.transfer_peer = Some(peer.to_string());
                            t.transferee_is_caller = sub.transferee_relay_port.is_some()
                                && t.caller_relay_rtp.as_ref().map(|ep| ep.port)
                                    == sub.transferee_relay_port;

                            // Insert bridged transfers mapping
                            if let Some(ref cid) = call_id {
                                edge_state
                                    .bridged_transfers
                                    .insert(cid.clone(), orig_cid.clone());
                                edge_state
                                    .bridged_transfers
                                    .insert(orig_cid.clone(), cid.clone());
                            }
                        }

                        if status_code >= 300 {
                            if let Some(target_port) = sub.target_relay_port {
                                edge_state.media_relay.clear_target(target_port);
                            }

                            // Transfer failed! Restore/Rollback the original media session between referrer and transferee
                            let transferee_is_caller = sub.transferee_relay_port.is_some()
                                && t.caller_relay_rtp.as_ref().map(|ep| ep.port)
                                    == sub.transferee_relay_port;
                            let transferee_relay = if transferee_is_caller {
                                t.caller_relay_rtp.clone()
                            } else {
                                t.gateway_relay_rtp.clone()
                            };
                            let referrer_relay = if transferee_is_caller {
                                t.gateway_relay_rtp.clone()
                            } else {
                                t.caller_relay_rtp.clone()
                            };
                            if let (Some(transferee_relay), Some(referrer_relay)) =
                                (transferee_relay, referrer_relay)
                            {
                                // 1. Re-pair the original ports
                                edge_state
                                    .media_relay
                                    .pair_ports(transferee_relay.port, referrer_relay.port);

                                // 2. Restore remote destinations (targets)
                                let ref_dest = if transferee_is_caller {
                                    t.gateway_rtp.clone()
                                } else {
                                    t.caller_rtp.clone()
                                };
                                if let Some(ref_dest) = ref_dest {
                                    let _ = edge_state
                                        .media_relay
                                        .set_target(&transferee_relay, &ref_dest);
                                }
                                let trans_dest = if transferee_is_caller {
                                    t.caller_rtp.clone()
                                } else {
                                    t.gateway_rtp.clone()
                                };
                                if let Some(trans_dest) = trans_dest {
                                    let _ = edge_state
                                        .media_relay
                                        .set_target(&referrer_relay, &trans_dest);
                                }
                                debug!(orig_cid = ?orig_cid, "restored original media session after transfer failure");
                            }
                        }

                        if status_code >= 200 {
                            // Final response: clean up refer transfers map and refer subscription from transaction
                            if let Some(ref cid) = call_id {
                                edge_state.refer_transfers.remove(cid);
                            }
                            t.refer_subscription = None;
                        }

                        edge_state.inbound_transactions.insert(orig_cid, t);
                        return datagrams;
                    }
                }
                return Vec::new();
            }

            if let Some(call_id) = call_id.as_deref() {
                edge_state.remember_inbound_to_tag(call_id, &sip_response);
                {
                    if let Some(mut t_mut) = edge_state.inbound_transactions.get_mut(call_id) {
                        t_mut.outbound_peer = Some(peer.to_string());
                    }
                }
                if sip_response.status_code >= 180 && sip_response.status_code < 300 {
                    if let Some(mut t_mut) = edge_state.inbound_transactions.get_mut(call_id) {
                        t_mut.outbound_route_set = sip_response
                            .headers
                            .get_all("record-route")
                            .map(|value| value.as_str().to_string())
                            .collect::<Vec<_>>();
                        if let Some(contact_val) = sip_response.headers.get("contact") {
                            if let Some(mut uri) = extract_uri_from_contact(contact_val.as_str()) {
                                if uri.port.is_none() {
                                    uri.port = t_mut.outbound_uri.port;
                                }
                                t_mut.callee_contact = Some(uri);
                            }
                        }
                    }
                }
            }
            let transaction = call_id.as_deref().and_then(|call_id| {
                edge_state
                    .inbound_transactions
                    .get(call_id)
                    .map(|r| r.clone())
            });

            // Parse Session-Expires from 200 OK and store on the transaction
            if sip_response.status_code >= 200 && sip_response.status_code < 300 {
                if let Some(cid) = call_id.as_deref() {
                    let se_header = sip_response
                        .headers
                        .get("session-expires")
                        .or_else(|| sip_response.headers.get("x"))
                        .map(|v| v.as_str().to_string());
                    if let Some(se_val) = se_header {
                        // "600;refresher=uac" → parse seconds and optional refresher
                        let mut parts = se_val.splitn(2, ';');
                        let secs: Option<u32> = parts.next().and_then(|s| s.trim().parse().ok());
                        let refresher = parts
                            .next()
                            .and_then(|p| p.split('=').nth(1).map(|r| r.trim().to_string()))
                            .unwrap_or_else(|| "uac".to_string());
                        if let Some(secs) = secs {
                            if let Some(mut t_mut) = edge_state.inbound_transactions.get_mut(cid) {
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
                }
            }

            let is_message = sip_response
                .headers
                .get("cseq")
                .map(|cseq| cseq.as_str().contains("MESSAGE"))
                .unwrap_or(false);
            if is_message && sip_response.status_code >= 200 {
                if let Some(cid) = call_id.as_deref() {
                    edge_state.inbound_transactions.remove(cid);
                    debug!(call_id = %cid, "cleaned up temporary MESSAGE transaction");
                }
            }

            let is_invite = sip_response
                .headers
                .get("cseq")
                .map(|cseq| cseq.as_str().contains("INVITE"))
                .unwrap_or(false);

            // Check if this is a Re-INVITE response (call already established - has caller_relay_rtp set)
            let is_reinvite_response = is_invite
                && transaction
                    .as_ref()
                    .map(|t| t.caller_relay_rtp.is_some())
                    .unwrap_or(false);

            let outbound_response_outcome = if is_invite && !is_reinvite_response {
                match edge_state
                    .call_manager
                    .handle_outbound_response(&sip_response)
                {
                    Ok(outcome) => outcome,
                    Err(error) => {
                        warn!(%error, "failed to apply outbound SIP response");
                        return Vec::new();
                    }
                }
            } else {
                call_core::OutboundResponseOutcome {
                    call_id: call_core::CallId::new(
                        sip_response
                            .headers
                            .get("call-id")
                            .map(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                    ),
                    state: call_core::CallState::Established,
                    failover_uri: None,
                }
            };

            // Record gateway health based on the outbound response outcome.
            if is_invite && !is_reinvite_response {
                let gateway_host = transaction
                    .as_ref()
                    .map(|t| t.outbound_uri.host.clone())
                    .unwrap_or_default();
                if !gateway_host.is_empty() {
                    let mut health = edge_state.gateway_health.lock().unwrap();
                    if sip_response.status_code >= 200 && sip_response.status_code <= 299 {
                        health.record_success(&gateway_host);
                    } else if sip_response.status_code >= 400 {
                        health.record_failure(&gateway_host);
                    }
                }
            }

            if let Some(next_uri) = outbound_response_outcome.failover_uri {
                info!(
                    call_id = ?call_id,
                    status = sip_response.status_code,
                    %next_uri,
                    "triggering gateway failover"
                );

                if let Some(transaction) = transaction.as_ref() {
                    edge_state.clear_media_targets(transaction);
                }

                let original_request = transaction
                    .as_ref()
                    .and_then(|t| t.original_request.as_ref());
                let rewritten_sdp = if let Some(req) = original_request {
                    match prepare_rewritten_sdp(
                        &req.headers,
                        &req.body,
                        &edge_state.media_relay,
                        &edge_config.media,
                        "failover INVITE offer",
                    ) {
                        Ok(rewritten_sdp) => rewritten_sdp,
                        Err(error) => {
                            warn!(%error, "failed to prepare media for failover INVITE");
                            None
                        }
                    }
                } else {
                    None
                };

                if let (Some(call_id), Some(sdp)) = (call_id.as_deref(), rewritten_sdp.as_ref()) {
                    if let Some(caller_rtp) = &sdp.original_endpoint {
                        register_relay_target(
                            &edge_state.media_relay,
                            &sdp.relay_endpoint,
                            caller_rtp,
                            "gateway-to-caller RTP (failover)",
                        );
                    }

                    if let Some(mut t_mut) = edge_state.inbound_transactions.get_mut(call_id) {
                        t_mut.outbound_uri = next_uri.clone();
                        t_mut.gateway_relay_rtp = Some(sdp.relay_endpoint.clone());
                        t_mut.caller_rtp = sdp.original_endpoint.clone();
                        t_mut.gateway_rtp = None;
                        t_mut.caller_relay_rtp = None;
                    }
                }

                if let (Some(req), Some(sdp)) = (original_request, rewritten_sdp) {
                    let target = outbound::target_addr_for(&next_uri);
                    // Topology Hiding: generate a fresh external Call-ID for the failover leg.
                    let failover_internal_cid = req
                        .headers
                        .get("call-id")
                        .map(|v| v.as_str().to_string())
                        .unwrap_or_default();
                    let failover_external_cid = edge_state
                        .get_external_call_id(&failover_internal_cid)
                        .unwrap_or_else(|| failover_internal_cid.clone());
                    let bytes = outbound::build_outbound_invite_with_body_and_call_id(
                        req,
                        &next_uri,
                        &edge_config.advertised_addr,
                        sdp.body.as_slice(),
                        &failover_external_cid,
                    );
                    return vec![PendingDatagram::new(target, bytes)];
                } else {
                    warn!("could not perform failover because original request or rewritten sdp is missing");
                    return Vec::new();
                }
            }

            if matches!(
                outbound_response_outcome.state,
                call_core::CallState::Failed
            ) {
                if let Some(transaction) = transaction.as_ref() {
                    edge_state.clear_media_targets(transaction);
                }
            }

            let mut rewritten_sdp_body = None;
            let mut mid_dialog_rewritten = false;

            if let Some(t) = &transaction {
                if t.gateway_relay_rtp.is_some() && t.caller_relay_rtp.is_some() {
                    mid_dialog_rewritten = true;
                    if media::is_sdp_body(&sip_response.headers, &sip_response.body) {
                        let is_to_caller = peer.to_string() != t.peer;
                        let relay_ep = if is_to_caller {
                            t.caller_relay_rtp.as_ref()
                        } else {
                            t.gateway_relay_rtp.as_ref()
                        };

                        if let Some(ep) = relay_ep {
                            // Single-pass: rewrite SDP + extract original endpoint
                            if let Ok((rewritten, remote_ep)) =
                                media::rewrite_sdp_and_extract_endpoint(&sip_response.body, ep)
                            {
                                rewritten_sdp_body = Some(rewritten);
                                register_relay_target(
                                    &edge_state.media_relay,
                                    ep,
                                    &remote_ep,
                                    "mid-dialog response target update",
                                );

                                if let Some(cid) = call_id.as_deref() {
                                    if let Some(mut t_mut) =
                                        edge_state.inbound_transactions.get_mut(cid)
                                    {
                                        if is_to_caller {
                                            t_mut.gateway_rtp = Some(remote_ep);
                                        } else {
                                            t_mut.caller_rtp = Some(remote_ep);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            let rewritten_sdp_bytes = if mid_dialog_rewritten {
                rewritten_sdp_body
            } else {
                match prepare_rewritten_sdp(
                    &sip_response.headers,
                    &sip_response.body,
                    &edge_state.media_relay,
                    &edge_config.media,
                    "outbound response answer",
                ) {
                    Ok(Some(sdp)) => {
                        if let (Some(call_id), Some(gateway_rtp)) =
                            (call_id.as_deref(), &sdp.original_endpoint)
                        {
                            register_relay_target(
                                &edge_state.media_relay,
                                &sdp.relay_endpoint,
                                gateway_rtp,
                                "caller-to-gateway RTP",
                            );

                            if let Some(pt) = media::parse_sdp_dtmf_payload_type(&sip_response.body)
                            {
                                edge_state.media_relay.register_port_dtmf_tracking(
                                    call_id,
                                    sdp.relay_endpoint.port,
                                    pt,
                                );
                            }

                            if let Some(t) = &transaction {
                                if let Some(original_req) = &t.original_request {
                                    if let Some(pt) =
                                        media::parse_sdp_dtmf_payload_type(&original_req.body)
                                    {
                                        if let Some(gateway_relay) = &t.gateway_relay_rtp {
                                            edge_state.media_relay.register_port_dtmf_tracking(
                                                call_id,
                                                gateway_relay.port,
                                                pt,
                                            );
                                        }
                                    }
                                }
                            }

                            edge_state.remember_gateway_media(
                                call_id,
                                sdp.original_endpoint.clone(),
                                sdp.relay_endpoint.clone(),
                                &edge_config.media,
                            );
                        }
                        Some(sdp.body)
                    }
                    _ => None,
                }
            };

            let cseq_method = sip_response
                .headers
                .get("cseq")
                .map(|cseq| cseq.as_str())
                .unwrap_or("");
            let is_renegotiation_response =
                cseq_method.contains("INVITE") || cseq_method.contains("UPDATE");
            let is_message_response = cseq_method.contains("MESSAGE");
            if !is_renegotiation_response && !is_message_response {
                return Vec::new();
            }

            // ── RFC 3262: 100rel intercept ────────────────────────────────────
            // If a provisional response carries `Require: 100rel` and `RSeq`,
            // sip-edge must:
            //   1. Send PRACK toward the gateway on behalf of the caller
            //   2. Rewrite RSeq with our own outbound sequence counter
            //   3. Forward the (rewritten) provisional response to the caller
            let is_100rel = sip_response.status_code >= 180
                && sip_response.status_code < 200
                && sip_response
                    .headers
                    .get("require")
                    .map(|v| v.as_str().contains("100rel"))
                    .unwrap_or(false);

            if is_100rel {
                if let Some(cid) = call_id.as_deref() {
                    // Extract RSeq from gateway response
                    let gw_rseq = sip_response
                        .headers
                        .get("rseq")
                        .and_then(|v| v.as_str().trim().parse::<u32>().ok())
                        .unwrap_or(1);

                    // Determine our own PRACK sequence number and increment the counter
                    let (our_rseq, prack_cseq, from_val, to_val, outbound_uri) = {
                        if let Some(mut t_mut) = edge_state.inbound_transactions.get_mut(cid) {
                            t_mut.prack_rseq += 1;
                            t_mut.gateway_100rel = true;
                            let our_rseq = t_mut.prack_rseq;
                            let prack_cseq = t_mut.last_inbound_cseq.unwrap_or(1) + 100 + our_rseq;
                            let from_val = sip_response
                                .headers
                                .get("from")
                                .map(|v| v.as_str().to_string())
                                .unwrap_or_default();
                            let to_val = sip_response
                                .headers
                                .get("to")
                                .map(|v| v.as_str().to_string())
                                .unwrap_or_default();
                            (
                                our_rseq,
                                prack_cseq,
                                from_val,
                                to_val,
                                t_mut.outbound_uri.clone(),
                            )
                        } else {
                            (
                                1,
                                1,
                                String::new(),
                                String::new(),
                                transaction
                                    .as_ref()
                                    .map(|t| t.outbound_uri.clone())
                                    .unwrap_or_else(|| {
                                        SipUri::from_str("sip:unknown@127.0.0.1").unwrap()
                                    }),
                            )
                        }
                    };

                    // "RAck: <rseq> <cseq-number> <cseq-method>"
                    // cseq from the gateway's 1xx, rseq from the gateway's 1xx
                    let gw_cseq_num = sip_response
                        .headers
                        .get("cseq")
                        .and_then(|v| v.as_str().split_whitespace().next()?.parse::<u32>().ok())
                        .unwrap_or(1);
                    let rack_value = format!("{gw_rseq} {gw_cseq_num} INVITE");

                    // 1. Send PRACK toward the gateway
                    // Topology Hiding: gateway expects its external Call-ID, not our internal one.
                    let prack_call_id = edge_state
                        .get_external_call_id(cid)
                        .unwrap_or_else(|| cid.to_string());
                    let prack_bytes = outbound::build_outbound_prack(
                        &prack_call_id,
                        &from_val,
                        &to_val,
                        prack_cseq,
                        &rack_value,
                        &edge_config.advertised_addr,
                        &outbound_uri,
                    );
                    let gw_target = outbound::target_addr_for(&outbound_uri);
                    let mut datagrams: Vec<PendingDatagram> =
                        vec![PendingDatagram::new(gw_target, prack_bytes)];

                    // 2. Forward the 1xx to the caller with our own RSeq
                    if let Some(t) = transaction.as_ref() {
                        // Replace RSeq with our outbound counter, keep Require: 100rel
                        // Topology Hiding: override Call-ID with the internal one so caller sees its original Call-ID.
                        let mut rewritten_response =
                            response::forward_response_to_inbound_with_body_and_call_id(
                                &sip_response,
                                &t.vias,
                                &t.inbound_route_set,
                                rewritten_sdp_bytes
                                    .as_deref()
                                    .unwrap_or(sip_response.body.as_slice()),
                                call_id.as_deref(),
                            );
                        // Patch the RSeq header in the raw response bytes
                        let raw_str = String::from_utf8_lossy(&rewritten_response);
                        let patched = replace_header_value(&raw_str, "RSeq", &our_rseq.to_string());
                        rewritten_response = patched.into_bytes();

                        datagrams.push(PendingDatagram::new(t.peer.clone(), rewritten_response));
                    }
                    return datagrams;
                }
            }
            // ─────────────────────────────────────────────────────────────────

            match transaction {
                Some(transaction) => vec![PendingDatagram::new(
                    transaction.peer,
                    // Topology Hiding: forward the response with the internal Call-ID so
                    // the caller never sees the gateway's external Call-ID.
                    response::forward_response_to_inbound_with_body_and_call_id(
                        &sip_response,
                        &transaction.vias,
                        &transaction.inbound_route_set,
                        rewritten_sdp_bytes
                            .as_deref()
                            .unwrap_or(sip_response.body.as_slice()),
                        call_id.as_deref(),
                    ),
                )],
                None => {
                    warn!("received outbound SIP response without inbound transaction");
                    Vec::new()
                }
            }
        }
        Err(error) => {
            warn!(%error, "failed to parse SIP datagram");
            Vec::new()
        }
    }
}

async fn handle_request(
    request: SipRequest,
    peer: SocketAddr,
    edge_state: &EdgeState,
    edge_config: &EdgeConfig,
) -> Vec<PendingDatagram> {
    if matches!(&request.method, Method::Register) {
        return handle_register_request(request, peer, edge_state, edge_config).await;
    }

    if matches!(&request.method, Method::Message) {
        let to_tag = request
            .headers
            .get("to")
            .and_then(|v| dialog::tag_param(v.as_str()));
        if to_tag.is_some() {
            return handle_in_dialog_request(request, peer, edge_state, edge_config).await;
        } else {
            return handle_out_of_dialog_message(request, peer, edge_state, edge_config).await;
        }
    }

    if outbound::is_forwardable_in_dialog_method(&request.method) {
        return handle_in_dialog_request(request, peer, edge_state, edge_config).await;
    }

    // Mid-dialog Re-INVITE: To header contains a tag, meaning this is within an established dialog
    if matches!(&request.method, Method::Invite) {
        let to_tag = request
            .headers
            .get("to")
            .and_then(|v| dialog::tag_param(v.as_str()));
        if to_tag.is_some() {
            return handle_in_dialog_request(request, peer, edge_state, edge_config).await;
        }
    }

    // RFC 3262: PRACK is always an in-dialog message (has To-tag) — route to in-dialog handler
    if matches!(&request.method, Method::Prack) {
        return handle_in_dialog_request(request, peer, edge_state, edge_config).await;
    }

    if edge_state.draining.load(Ordering::Relaxed) && matches!(&request.method, Method::Invite) {
        info!(
            call_id = %request.headers.get("call-id").map(|v| v.as_str()).unwrap_or(""),
            "rejecting new INVITE with 503 during drain"
        );
        return vec![PendingDatagram::new(
            peer.to_string(),
            response::response_503_service_unavailable(&request),
        )];
    }

    if matches!(&request.method, Method::Invite) {
        if let Some(username) = request.headers.get("from").and_then(|v| {
            let s = v.as_str();
            let start = s.find("sip:")?;
            let end = s[start..].find('@')?;
            Some(s[start + 4..start + end].to_string())
        }) {
            let active_count = {
                edge_state
                    .inbound_transactions
                    .iter()
                    .filter(|entry| {
                        let tx = entry.value();
                        if let Some(ref orig) = tx.original_request {
                            if let Some(orig_username) = orig.headers.get("from").and_then(|v| {
                                let s = v.as_str();
                                let start = s.find("sip:")?;
                                let end = s[start..].find('@')?;
                                Some(s[start + 4..start + end].to_string())
                            }) {
                                orig_username == username
                            } else {
                                false
                            }
                        } else {
                            false
                        }
                    })
                    .count() as u32
            };

            if active_count >= edge_config.sbc_max_concurrency {
                warn!(%username, active_count, limit = edge_config.sbc_max_concurrency, "rejecting INVITE due to user concurrency limit exceeded");
                return vec![PendingDatagram::new(
                    peer.to_string(),
                    response::build_response_with_owned_headers(
                        &request,
                        486,
                        "Busy Here - Concurrency Limit Exceeded",
                        &[],
                        "",
                    ),
                )];
            }
        }

        let from_gw = edge_state.is_peer_gateway(peer).await;
        if !from_gw {
            let db_store = edge_state.db_store.clone();
            if matches!(
                edge_config
                    .auth
                    .verify_request(
                        &request,
                        db_store.as_ref(),
                        Some(&edge_state.nonce_replay_cache)
                    )
                    .await,
                AuthDecision::Challenge
            ) {
                return vec![PendingDatagram::new(
                    peer.to_string(),
                    proxy_unauthorized_for_request(&request, &edge_config.auth),
                )];
            }
        }
    }

    let registered_contact = {
        if matches!(&request.method, Method::Invite) {
            edge_state
                .registrar
                .lock()
                .await
                .lookup_contact(
                    &request.uri,
                    SystemTime::now(),
                    edge_state.db_store.as_ref(),
                )
                .await
        } else {
            None
        }
    };
    let response::RequestHandling {
        response,
        mut outbound_invite,
    } = if let Some(ref contact) = registered_contact {
        if let Ok(outbound_uri) = SipUri::from_str(&contact.uri) {
            response::response_for_invite_to_uri(&request, &edge_state.call_manager, outbound_uri)
        } else {
            response::response_for_request(&request, &edge_state.call_manager)
        }
    } else {
        response::response_for_request(&request, &edge_state.call_manager)
    };

    if let Some(ref contact) = registered_contact {
        if let Some(ref mut plan) = outbound_invite {
            plan.target_override_addr = Some(contact.received_from.clone());
        }
    }

    if let Some(outbound_invite) = outbound_invite.as_ref() {
        let rewritten_sdp = match prepare_rewritten_sdp(
            &request.headers,
            &request.body,
            &edge_state.media_relay,
            &edge_config.media,
            "inbound INVITE offer",
        ) {
            Ok(rewritten_sdp) => rewritten_sdp,
            Err(error) => {
                warn!(%error, "rejecting INVITE after media negotiation failure");
                return vec![PendingDatagram::new(
                    peer.to_string(),
                    response_for_media_error(&request, &error),
                )];
            }
        };
        if let Some(rewritten_sdp) = &rewritten_sdp {
            if let Some(caller_rtp) = &rewritten_sdp.original_endpoint {
                register_relay_target(
                    &edge_state.media_relay,
                    &rewritten_sdp.relay_endpoint,
                    caller_rtp,
                    "gateway-to-caller RTP",
                );
            }
        }
        edge_state.remember_inbound_invite(
            &request,
            peer,
            outbound_invite.outbound_uri.clone(),
            rewritten_sdp
                .as_ref()
                .and_then(|sdp| sdp.original_endpoint.clone()),
            rewritten_sdp.as_ref().map(|sdp| sdp.relay_endpoint.clone()),
            outbound_invite.target_override_addr.is_some(),
        );

        let mut datagrams = vec![PendingDatagram::new(peer.to_string(), response)];
        let target = if let Some(ref override_addr) = outbound_invite.target_override_addr {
            override_addr.clone()
        } else {
            outbound::target_addr_for(&outbound_invite.outbound_uri)
        };
        let path = if let Some(ref contact) = registered_contact {
            contact.path.as_slice()
        } else {
            &[]
        };

        // Topology Hiding: generate a fresh Call-ID for the outbound (gateway) leg.
        // The inbound Call-ID is retained internally; the gateway sees only the external one.
        let internal_call_id = request
            .headers
            .get("call-id")
            .map(|v| v.as_str().to_string())
            .unwrap_or_default();
        let nonce_input = format!(
            "{}-{}",
            internal_call_id,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        let md5_hex = format!("{:x}", md5::compute(nonce_input.as_bytes()));
        let external_call_id = format!(
            "{}@{}",
            &md5_hex[..16],
            edge_config
                .advertised_addr
                .split(':')
                .next()
                .unwrap_or("vos-rs")
        );
        edge_state.register_call_id_mapping(&internal_call_id, &external_call_id);
        debug!(
            internal_call_id,
            external_call_id, "topology hiding: registered Call-ID mapping"
        );

        let bytes = outbound::build_outbound_invite_with_session_timer_and_call_id(
            &request,
            &outbound_invite.outbound_uri,
            &edge_config.advertised_addr,
            rewritten_sdp
                .as_ref()
                .map(|sdp| sdp.body.as_slice())
                .unwrap_or(request.body.as_slice()),
            edge_config.session_expires_gateway,
            path,
            &external_call_id,
        );
        datagrams.push(PendingDatagram::new(target, bytes));
        return datagrams;
    }

    vec![PendingDatagram::new(peer.to_string(), response)]
}

async fn handle_out_of_dialog_message(
    request: SipRequest,
    peer: SocketAddr,
    edge_state: &EdgeState,
    edge_config: &EdgeConfig,
) -> Vec<PendingDatagram> {
    let call_id = match request
        .headers
        .get("call-id")
        .map(|v| v.as_str().to_string())
    {
        Some(cid) => cid,
        None => {
            return vec![PendingDatagram::new(
                peer.to_string(),
                response::build_response_with_owned_headers(&request, 400, "Bad Request", &[], ""),
            )];
        }
    };

    let target_contact = {
        let registrar = edge_state.registrar.lock().await;
        registrar
            .lookup_contact(
                &request.uri,
                SystemTime::now(),
                edge_state.db_store.as_ref(),
            )
            .await
    };

    let outbound_uri = if let Some(ref contact) = target_contact {
        SipUri::from_str(&contact.uri).ok()
    } else {
        edge_state
            .call_manager
            .routes()
            .select(&request.uri)
            .ok()
            .map(|sr| sr.outbound_uri)
    };

    let Some(outbound_uri) = outbound_uri else {
        info!(call_id = %call_id, to = %request.uri, "destination for MESSAGE not found");
        let route_error = call_core::CallError::NoRouteForDestination(request.uri.to_string());
        return vec![PendingDatagram::new(
            peer.to_string(),
            response::error_for_call_error(&request, &route_error),
        )];
    };

    let vias = request
        .headers
        .get_all("via")
        .map(|v| v.as_str().to_string())
        .collect::<Vec<_>>();
    let inbound_route_set = request
        .headers
        .get_all("record-route")
        .map(|v| v.as_str().to_string())
        .collect::<Vec<_>>();

    let target_addr = if let Some(ref contact) = target_contact {
        contact.received_from.clone()
    } else {
        outbound::target_addr_for(&outbound_uri)
    };

    {
        edge_state.inbound_transactions.insert(
            call_id.clone(),
            InboundTransaction {
                peer: peer.to_string(),
                outbound_peer: target_contact.as_ref().map(|c| c.received_from.clone()),
                vias,
                outbound_uri: outbound_uri.clone(),
                inbound_from_tag: request
                    .headers
                    .get("from")
                    .and_then(|v| dialog::tag_param(v.as_str())),
                inbound_to_tag: None,
                last_inbound_cseq: request
                    .headers
                    .get("cseq")
                    .and_then(|v| dialog::cseq_number(v.as_str())),
                last_outbound_cseq: None,
                caller_rtp: None,
                gateway_relay_rtp: None,
                gateway_rtp: None,
                caller_relay_rtp: None,
                original_request: Some(Arc::new(request.clone())),
                inbound_route_set,
                outbound_route_set: Vec::new(),
                caller_contact: None,
                callee_contact: None,
                session_expires: None,
                session_refresher: None,
                last_session_refresh: None,
                prack_rseq: 0,
                gateway_100rel: false,
                refer_subscription: None,
                transfer_from_header: None,
                transfer_to_header: None,
                transfer_call_id: None,
                transfer_contact: None,
                transfer_peer: None,
                transferee_is_caller: false,
                callee_behind_nat: target_contact.is_some(),
            },
        );
    }

    let outbound_bytes =
        outbound::build_outbound_message(&request, &outbound_uri, &edge_config.advertised_addr);

    vec![PendingDatagram::new(target_addr, outbound_bytes)]
}

async fn handle_register_request(
    request: SipRequest,
    peer: SocketAddr,
    edge_state: &EdgeState,
    edge_config: &EdgeConfig,
) -> Vec<PendingDatagram> {
    let db_store = edge_state.db_store.clone();

    if matches!(
        edge_config
            .auth
            .verify_request(
                &request,
                db_store.as_ref(),
                Some(&edge_state.nonce_replay_cache)
            )
            .await,
        AuthDecision::Challenge
    ) {
        return vec![PendingDatagram::new(
            peer.to_string(),
            unauthorized_for_request(&request, &edge_config.auth),
        )];
    }

    let response = {
        let mut registrar_guard = edge_state.registrar.lock().await;
        match registrar_guard
            .handle_register(&request, peer, SystemTime::now(), db_store.as_ref())
            .await
        {
            Ok(outcome) => response_for_register_outcome(&request, &outcome),
            Err(error) => response::build_response_with_owned_headers(
                &request,
                400,
                "Bad Request",
                &[("X-VOS-RS-Error".to_string(), error.to_string())],
                "",
            ),
        }
    };

    vec![PendingDatagram::new(peer.to_string(), response)]
}

fn unauthorized_for_request(request: &SipRequest, auth_config: &AuthConfig) -> Vec<u8> {
    let nonce = auth_config.select_nonce();
    let challenge = auth_config.challenge_header_with_nonce(&nonce);
    response::build_response_with_owned_headers(
        request,
        401,
        "Unauthorized",
        &[("WWW-Authenticate".to_string(), challenge)],
        "",
    )
}

fn proxy_unauthorized_for_request(request: &SipRequest, auth_config: &AuthConfig) -> Vec<u8> {
    let nonce = auth_config.select_nonce();
    let challenge = auth_config.challenge_header_with_nonce(&nonce);
    response::build_response_with_owned_headers(
        request,
        407,
        "Proxy Authentication Required",
        &[("Proxy-Authenticate".to_string(), challenge)],
        "",
    )
}

fn response_for_register_outcome(request: &SipRequest, outcome: &RegisterOutcome) -> Vec<u8> {
    let mut headers = Vec::with_capacity(outcome.contacts.len() + 1);
    headers.push(("X-VOS-RS-AOR".to_string(), outcome.aor.clone()));
    headers.extend(outcome.contacts.iter().map(|contact| {
        (
            "Contact".to_string(),
            format!("<{}>;expires={}", contact.uri, contact.expires),
        )
    }));

    let advertised = EdgeConfig::from_env().advertised_addr;
    headers.push((
        "Service-Route".to_string(),
        format!("<sip:{};lr>", advertised),
    ));

    response::build_response_with_owned_headers(request, 200, "OK", &headers, "")
}

async fn handle_in_dialog_request(
    request: SipRequest,
    peer: SocketAddr,
    edge_state: &EdgeState,
    edge_config: &EdgeConfig,
) -> Vec<PendingDatagram> {
    let Some(call_id) = request
        .headers
        .get("call-id")
        .map(|v| v.as_str().to_string())
    else {
        if matches!(&request.method, Method::Ack) {
            return Vec::new();
        }

        let error = CallError::MissingRequiredHeader("Call-ID");
        return vec![PendingDatagram::new(
            peer.to_string(),
            response::error_for_call_error(&request, &error),
        )];
    };

    let mut mutable_request = request;

    let (transaction, source_leg, is_target) = {
        let lookup_cid = edge_state
            .bridged_transfers
            .get(call_id.as_str())
            .map(|r| r.clone());
        let actual_cid = lookup_cid.as_deref().unwrap_or(call_id.as_str());

        let Some(mut t) = edge_state.inbound_transactions.get_mut(actual_cid) else {
            if matches!(&mutable_request.method, Method::Ack) {
                return Vec::new();
            }

            let error = call_error_for_unknown_request(&mutable_request);
            return vec![PendingDatagram::new(
                peer.to_string(),
                response::error_for_call_error(&mutable_request, &error),
            )];
        };

        let is_target = t.transfer_call_id.as_deref() == Some(call_id.as_str());

        let source_leg = if is_target {
            let leg = if t.transferee_is_caller {
                DialogLeg::Gateway
            } else {
                DialogLeg::Caller
            };

            // Rewrite Call-ID, From, To for original leg B
            mutable_request.headers.replace(
                HeaderName::new("call-id").unwrap(),
                HeaderValue::new(actual_cid),
            );
            if let Some(orig_req) = &t.original_request {
                let (from_val, to_val) = if t.transferee_is_caller {
                    (
                        orig_req.headers.get("to").cloned(),
                        orig_req.headers.get("from").cloned(),
                    )
                } else {
                    (
                        orig_req.headers.get("from").cloned(),
                        orig_req.headers.get("to").cloned(),
                    )
                };
                if let Some(f) = from_val {
                    mutable_request.headers.replace(
                        HeaderName::new("from").unwrap(),
                        HeaderValue::new(f.as_str()),
                    );
                }
                if let Some(o) = to_val {
                    mutable_request
                        .headers
                        .replace(HeaderName::new("to").unwrap(), HeaderValue::new(o.as_str()));
                }
            }

            leg
        } else {
            let is_bridged = t.transfer_call_id.is_some();

            let (leg, cseq_update) = match t.validate_in_dialog_request(&mutable_request, peer) {
                Ok(result) => result,
                Err(error) => {
                    return vec![PendingDatagram::new(
                        peer.to_string(),
                        response_for_dialog_validation_error(&mutable_request, &error),
                    )];
                }
            };

            // Update cseq outside of validation
            if let Some(cseq) = cseq_update {
                match leg {
                    DialogLeg::Caller => t.last_inbound_cseq = Some(cseq),
                    DialogLeg::Gateway => t.last_outbound_cseq = Some(cseq),
                }
            }

            if is_bridged {
                // Rewrite Call-ID, From, To for target leg C
                if let Some(ref tf_cid) = t.transfer_call_id {
                    mutable_request.headers.insert(
                        HeaderName::new("call-id").unwrap(),
                        HeaderValue::new(tf_cid),
                    );
                }
                if let Some(ref tf_from) = t.transfer_from_header {
                    mutable_request
                        .headers
                        .insert(HeaderName::new("from").unwrap(), HeaderValue::new(tf_from));
                }
                if let Some(ref tf_to) = t.transfer_to_header {
                    mutable_request
                        .headers
                        .insert(HeaderName::new("to").unwrap(), HeaderValue::new(tf_to));
                }
            }

            leg
        };

        (t.clone(), source_leg, is_target)
    };

    let mut datagrams = Vec::new();
    match &mutable_request.method {
        Method::Bye | Method::Cancel => {
            let mut caller_rtcp = None;
            let mut gateway_rtcp = None;

            if let Some(endpoint) = &transaction.caller_relay_rtp {
                caller_rtcp = Some(
                    edge_state
                        .media_relay
                        .metrics_for_port(endpoint.port)
                        .rtcp_quality,
                );
            }
            if let Some(endpoint) = &transaction.gateway_relay_rtp {
                gateway_rtcp = Some(
                    edge_state
                        .media_relay
                        .metrics_for_port(endpoint.port)
                        .rtcp_quality,
                );
            }

            let metrics = if caller_rtcp.is_some() || gateway_rtcp.is_some() {
                Some(calculate_mos_for_legs(
                    caller_rtcp.as_ref(),
                    gateway_rtcp.as_ref(),
                ))
            } else {
                None
            };

            let cid = mutable_request
                .headers
                .get("call-id")
                .map(|val| val.as_str())
                .unwrap_or("missing-call-id");
            let dtmf_digits = edge_state.media_relay.get_dtmf_digits(cid);
            if let Some(digits) = &dtmf_digits {
                info!(call_id = cid, digits = %digits, "collected DTMF digits for call");
            }
            edge_state.media_relay.clear_dtmf_digits(cid);

            // Collect DTMF audit events for persistence to the detail table.
            let dtmf_events = edge_state.media_relay.take_dtmf_events(cid);
            if !dtmf_events.is_empty() {
                info!(
                    call_id = cid,
                    count = dtmf_events.len(),
                    "collected DTMF audit events for call"
                );
                if let Some(ref db) = edge_state.db_store {
                    if let Err(error) = db.insert_dtmf_events_batch(&dtmf_events).await {
                        warn!(%error, call_id = cid, "failed to persist DTMF audit events");
                    }
                }
            } else {
                edge_state.media_relay.clear_dtmf_events(cid);
            }

            // Clean up bridged mappings
            if transaction.transfer_call_id.is_some() {
                if let Some(ref tf_cid) = transaction.transfer_call_id {
                    edge_state.bridged_transfers.remove(tf_cid);
                }
                let lookup_cid = edge_state
                    .bridged_transfers
                    .get(call_id.as_str())
                    .map(|r| r.clone());
                let actual_cid = lookup_cid.as_deref().unwrap_or(call_id.as_str());
                edge_state.bridged_transfers.remove(actual_cid);
            }

            match edge_state.call_manager.handle_inbound_termination(
                &mutable_request,
                metrics,
                dtmf_digits,
            ) {
                Ok(_) => {
                    edge_state.clear_media_targets(&transaction);
                    if transaction.transfer_call_id.is_some() {
                        let transferee_port = if transaction.transferee_is_caller {
                            transaction.caller_relay_rtp.as_ref().map(|ep| ep.port)
                        } else {
                            transaction.gateway_relay_rtp.as_ref().map(|ep| ep.port)
                        };
                        if let Some(tp) = transferee_port {
                            if let Some(cp) = edge_state.media_relay.peer_port_for(tp) {
                                edge_state.media_relay.clear_target(cp);
                            }
                        }
                    }

                    datagrams.push(PendingDatagram::new(
                        peer.to_string(),
                        response::ok_for_request(&mutable_request),
                    ));
                }
                Err(error) => {
                    datagrams.push(PendingDatagram::new(
                        peer.to_string(),
                        response::error_for_call_error(&mutable_request, &error),
                    ));
                    return datagrams;
                }
            }
        }
        Method::Info => {
            let content_type = mutable_request
                .headers
                .get("content-type")
                .map(|v| v.as_str())
                .unwrap_or("");
            if let Some(digit) = parse_sip_info_dtmf(content_type, &mutable_request.body) {
                let cid = mutable_request
                    .headers
                    .get("call-id")
                    .map(|v| v.as_str())
                    .unwrap_or("");
                if !cid.is_empty() {
                    edge_state.media_relay.register_info_dtmf_digit(cid, digit);
                }
            }

            datagrams.push(PendingDatagram::new(
                peer.to_string(),
                response::ok_for_request(&mutable_request),
            ));
        }
        Method::Prack => {
            let rack_valid = if let Some(rack) = mutable_request.headers.get("rack") {
                let parts = rack.as_str().split_whitespace().collect::<Vec<_>>();
                if parts.len() == 3 {
                    let rseq_ok = parts[0].parse::<u32>().is_ok();
                    let cseq_ok = parts[1].parse::<u32>().is_ok();
                    let method_ok = !parts[2].is_empty();
                    rseq_ok && cseq_ok && method_ok
                } else {
                    false
                }
            } else {
                false
            };

            if !rack_valid {
                warn!("received PRACK with missing or invalid RAck header");
                datagrams.push(PendingDatagram::new(
                    peer.to_string(),
                    response::build_response_with_owned_headers(
                        &mutable_request,
                        400,
                        "Bad Request - Invalid RAck",
                        &[],
                        "",
                    ),
                ));
                return datagrams;
            }

            debug!(
                call_id = mutable_request
                    .headers
                    .get("call-id")
                    .map(|v| v.as_str())
                    .unwrap_or("?"),
                "received PRACK from caller — responding 200 OK (already confirmed to gateway)"
            );
            datagrams.push(PendingDatagram::new(
                peer.to_string(),
                response::ok_for_request(&mutable_request),
            ));
            return datagrams;
        }
        Method::Refer => {
            // RFC 3515 Blind Transfer B2BUA handling
            let refer_to_str = mutable_request.headers.get("refer-to").map(|v| v.as_str());
            let target_uri = refer_to_str.and_then(extract_uri_from_contact);

            datagrams.push(PendingDatagram::new(
                peer.to_string(),
                response::accepted_202_for_request(&mutable_request),
            ));

            if let Some(target_uri) = target_uri {
                let local_cseq = transaction.last_inbound_cseq.unwrap_or(1) + 50;

                let notify_body = "SIP/2.0 100 Trying\r\n";
                let notify = outbound::build_notify_sipfrag(
                    call_id.as_str(),
                    mutable_request
                        .headers
                        .get("from")
                        .map(|v| v.as_str())
                        .unwrap_or(""),
                    mutable_request
                        .headers
                        .get("to")
                        .map(|v| v.as_str())
                        .unwrap_or(""),
                    local_cseq,
                    &edge_config.advertised_addr,
                    notify_body,
                );
                datagrams.push(PendingDatagram::new(peer.to_string(), notify));

                let outbound_uri = {
                    let registrar = edge_state.registrar.lock().await;
                    if let Some(contact) = registrar
                        .lookup_contact(
                            &target_uri,
                            SystemTime::now(),
                            edge_state.db_store.as_ref(),
                        )
                        .await
                    {
                        SipUri::from_str(&contact.uri).ok()
                    } else {
                        edge_state
                            .call_manager
                            .routes()
                            .select(&target_uri)
                            .ok()
                            .map(|sr| sr.outbound_uri)
                    }
                };

                if outbound_uri.is_none() {
                    let notify_404 = outbound::build_notify_sipfrag_with_state(
                        call_id.as_str(),
                        mutable_request
                            .headers
                            .get("from")
                            .map(|v| v.as_str())
                            .unwrap_or(""),
                        mutable_request
                            .headers
                            .get("to")
                            .map(|v| v.as_str())
                            .unwrap_or(""),
                        local_cseq + 1,
                        &edge_config.advertised_addr,
                        "SIP/2.0 404 Not Found\r\n",
                        "terminated;reason=noresource",
                    );
                    datagrams.push(PendingDatagram::new(peer.to_string(), notify_404));
                    return datagrams;
                }
                let outbound_uri = outbound_uri.unwrap();

                let target_relay_rtp = match edge_state
                    .media_relay
                    .allocate_endpoint(&edge_config.media)
                {
                    Ok(ep) => ep,
                    Err(error) => {
                        warn!(%error, "failed to allocate media relay endpoint for transfer target");
                        let notify_503 = outbound::build_notify_sipfrag_with_state(
                            call_id.as_str(),
                            mutable_request
                                .headers
                                .get("from")
                                .map(|v| v.as_str())
                                .unwrap_or(""),
                            mutable_request
                                .headers
                                .get("to")
                                .map(|v| v.as_str())
                                .unwrap_or(""),
                            local_cseq + 1,
                            &edge_config.advertised_addr,
                            "SIP/2.0 503 Service Unavailable\r\n",
                            "terminated;reason=noresource",
                        );
                        datagrams.push(PendingDatagram::new(peer.to_string(), notify_503));
                        return datagrams;
                    }
                };

                let transferee_relay_rtp = match source_leg {
                    DialogLeg::Caller => transaction.gateway_relay_rtp.clone(),
                    DialogLeg::Gateway => transaction.caller_relay_rtp.clone(),
                };

                if let Some(transferee_relay) = &transferee_relay_rtp {
                    edge_state
                        .media_relay
                        .pair_ports(target_relay_rtp.port, transferee_relay.port);
                }

                let transfer_call_id = format!("transfer-{}-{}", call_id.as_str(), local_cseq);

                let refer_sub = ReferSubscription {
                    refer_to: target_uri.to_string(),
                    from_header: mutable_request
                        .headers
                        .get("from")
                        .map(|v| v.as_str().to_string())
                        .unwrap_or_default(),
                    to_header: mutable_request
                        .headers
                        .get("to")
                        .map(|v| v.as_str().to_string())
                        .unwrap_or_default(),
                    notify_cseq: local_cseq,
                    transfer_call_id: transfer_call_id.clone(),
                    referrer_peer: peer.to_string(),
                    refer_cseq: mutable_request
                        .headers
                        .get("cseq")
                        .and_then(|v| dialog::cseq_number(v.as_str()))
                        .unwrap_or(1),
                    target_relay_port: Some(target_relay_rtp.port),
                    transferee_relay_port: transferee_relay_rtp.as_ref().map(|ep| ep.port),
                };

                {
                    if let Some(mut t_mut) =
                        edge_state.inbound_transactions.get_mut(call_id.as_str())
                    {
                        t_mut.refer_subscription = Some(refer_sub);
                    }
                }

                edge_state
                    .refer_transfers
                    .insert(transfer_call_id.clone(), call_id.as_str().to_string());

                let from_header = match source_leg {
                    DialogLeg::Caller => mutable_request
                        .headers
                        .get("to")
                        .map(|v| v.as_str())
                        .unwrap_or(""),
                    DialogLeg::Gateway => mutable_request
                        .headers
                        .get("from")
                        .map(|v| v.as_str())
                        .unwrap_or(""),
                };
                let to_header = refer_to_str.unwrap_or("");

                let sdp_body = format!(
                    "v=0\r\no=- 0 0 IN IP4 {addr}\r\ns=-\r\nc=IN IP4 {addr}\r\nt=0 0\r\nm=audio {port} RTP/AVP 0 8 101\r\na=rtpmap:0 PCMU/8000\r\na=rtpmap:8 PCMA/8000\r\na=rtpmap:101 telephone-event/8000\r\na=fmtp:101 0-16\r\n",
                    addr = edge_config.advertised_addr,
                    port = target_relay_rtp.port,
                );

                let invite_bytes = outbound::build_transfer_invite(
                    &transfer_call_id,
                    from_header,
                    to_header,
                    1,
                    &edge_config.advertised_addr,
                    &outbound_uri,
                    sdp_body.as_bytes(),
                );

                let target_addr = outbound::target_addr_for(&outbound_uri);
                datagrams.push(PendingDatagram::new(target_addr, invite_bytes));
                return datagrams;
            } else {
                warn!(
                    call_id = call_id.as_str(),
                    "missing or invalid Refer-To header in REFER"
                );
                let notify_400 = outbound::build_notify_sipfrag_with_state(
                    call_id.as_str(),
                    mutable_request
                        .headers
                        .get("from")
                        .map(|v| v.as_str())
                        .unwrap_or(""),
                    mutable_request
                        .headers
                        .get("to")
                        .map(|v| v.as_str())
                        .unwrap_or(""),
                    transaction.last_inbound_cseq.unwrap_or(1) + 50,
                    &edge_config.advertised_addr,
                    "SIP/2.0 400 Bad Request\r\n",
                    "terminated;reason=noresource",
                );
                datagrams.push(PendingDatagram::new(peer.to_string(), notify_400));
                return datagrams;
            }
        }
        _ => {}
    }

    let (request_uri, route_set, target) =
        if transaction.transfer_call_id.is_some() {
            if is_target {
                if transaction.transferee_is_caller {
                    let uri = transaction
                        .caller_contact
                        .clone()
                        .unwrap_or_else(|| sip_uri_from_peer(&transaction.peer));
                    (uri, Vec::new(), transaction.peer.clone())
                } else {
                    let uri = transaction
                        .callee_contact
                        .clone()
                        .unwrap_or_else(|| transaction.outbound_uri.clone());
                    let target = if !transaction.outbound_route_set.is_empty() {
                        parse_target_addr_from_route(&transaction.outbound_route_set[0])
                            .unwrap_or_else(|| {
                                if transaction.callee_behind_nat {
                                    transaction.outbound_peer.clone().unwrap_or_else(|| {
                                        outbound::target_addr_for(&transaction.outbound_uri)
                                    })
                                } else {
                                    outbound::target_addr_for(&transaction.outbound_uri)
                                }
                            })
                    } else if transaction.callee_behind_nat {
                        transaction
                            .outbound_peer
                            .clone()
                            .unwrap_or_else(|| outbound::target_addr_for(&uri))
                    } else {
                        outbound::target_addr_for(&uri)
                    };
                    (uri, transaction.outbound_route_set.clone(), target)
                }
            } else {
                let uri = transaction
                    .transfer_contact
                    .clone()
                    .unwrap_or_else(|| transaction.outbound_uri.clone());
                let target = transaction
                    .transfer_peer
                    .clone()
                    .unwrap_or_else(|| outbound::target_addr_for(&uri));
                (uri, Vec::new(), target)
            }
        } else {
            match source_leg {
                DialogLeg::Caller => {
                    let request_uri = transaction
                        .callee_contact
                        .clone()
                        .unwrap_or_else(|| transaction.outbound_uri.clone());
                    let target = if !transaction.outbound_route_set.is_empty() {
                        parse_target_addr_from_route(&transaction.outbound_route_set[0])
                            .unwrap_or_else(|| {
                                if transaction.callee_behind_nat {
                                    transaction.outbound_peer.clone().unwrap_or_else(|| {
                                        outbound::target_addr_for(&transaction.outbound_uri)
                                    })
                                } else {
                                    outbound::target_addr_for(&transaction.outbound_uri)
                                }
                            })
                    } else if transaction.callee_behind_nat {
                        transaction
                            .outbound_peer
                            .clone()
                            .unwrap_or_else(|| outbound::target_addr_for(&request_uri))
                    } else {
                        outbound::target_addr_for(&request_uri)
                    };
                    (request_uri, transaction.outbound_route_set.clone(), target)
                }
                DialogLeg::Gateway => {
                    let request_uri = transaction
                        .caller_contact
                        .clone()
                        .unwrap_or_else(|| sip_uri_from_peer(&transaction.peer));
                    let target = if !transaction.inbound_route_set.is_empty() {
                        parse_target_addr_from_route(&transaction.inbound_route_set[0])
                            .unwrap_or_else(|| transaction.peer.clone())
                    } else {
                        transaction.peer.clone()
                    };
                    (request_uri, transaction.inbound_route_set.clone(), target)
                }
            }
        };

    let mut rewritten_sdp = None;
    let is_bridged = transaction.transfer_call_id.is_some();
    if !is_bridged && matches!(&mutable_request.method, Method::Invite | Method::Update) {
        {
            if let Some(mut t_mut) = edge_state.inbound_transactions.get_mut(call_id.as_str()) {
                t_mut.last_session_refresh = Some(Instant::now());
                debug!(
                    call_id = call_id.as_str(),
                    "session timer refreshed by Re-INVITE/UPDATE"
                );
            }
        }

        if media::is_sdp_body(&mutable_request.headers, &mutable_request.body) {
            let is_from_caller = peer.to_string() == transaction.peer;
            if is_from_caller {
                if let Some(gw_relay) = &transaction.gateway_relay_rtp {
                    // Single-pass: rewrite SDP + extract original endpoint
                    if let Ok((rewritten, remote_ep)) =
                        media::rewrite_sdp_and_extract_endpoint(&mutable_request.body, gw_relay)
                    {
                        rewritten_sdp = Some(rewritten);
                        register_relay_target(
                            &edge_state.media_relay,
                            gw_relay,
                            &remote_ep,
                            "mid-dialog caller target update",
                        );

                        if let Some(mut t_mut) =
                            edge_state.inbound_transactions.get_mut(call_id.as_str())
                        {
                            t_mut.caller_rtp = Some(remote_ep);
                            t_mut.original_request = Some(Arc::new(mutable_request.clone()));
                        }
                    }
                }
            } else if let Some(caller_relay) = &transaction.caller_relay_rtp {
                // Single-pass: rewrite SDP + extract original endpoint
                if let Ok((rewritten, remote_ep)) =
                    media::rewrite_sdp_and_extract_endpoint(&mutable_request.body, caller_relay)
                {
                    rewritten_sdp = Some(rewritten);
                    register_relay_target(
                        &edge_state.media_relay,
                        caller_relay,
                        &remote_ep,
                        "mid-dialog gateway target update",
                    );

                    if let Some(mut t_mut) =
                        edge_state.inbound_transactions.get_mut(call_id.as_str())
                    {
                        t_mut.gateway_rtp = Some(remote_ep);
                    }
                }
            }
        }
    }

    // Topology Hiding: when forwarding a request from the caller toward the gateway,
    // replace the inbound (internal) Call-ID with the external Call-ID the gateway knows.
    // When forwarding from gateway toward caller, no rewrite is needed — the caller sees
    // the internal Call-ID.
    if matches!(source_leg, DialogLeg::Caller) {
        let internal_cid = mutable_request
            .headers
            .get("call-id")
            .map(|v| v.as_str().to_string())
            .unwrap_or_default();
        if let Some(external_cid) = edge_state.get_external_call_id(&internal_cid) {
            mutable_request.headers.replace(
                HeaderName::new("call-id").unwrap(),
                HeaderValue::new(&external_cid),
            );
            debug!(
                internal_cid,
                external_cid, "topology hiding: rewrote Call-ID for in-dialog request to gateway"
            );
        }
    }

    let bytes = if let Some(body) = &rewritten_sdp {
        outbound::build_outbound_in_dialog_request_with_body(
            &mutable_request,
            &request_uri,
            &edge_config.advertised_addr,
            &route_set,
            body,
        )
    } else {
        outbound::build_outbound_in_dialog_request(
            &mutable_request,
            &request_uri,
            &edge_config.advertised_addr,
            &route_set,
        )
    };
    datagrams.push(PendingDatagram::new(target, bytes));
    datagrams
}

fn call_error_for_unknown_request(request: &SipRequest) -> CallError {
    match request.headers.get("call-id") {
        Some(call_id) => CallError::UnknownCall(call_id.as_str().to_string()),
        None => CallError::MissingRequiredHeader("Call-ID"),
    }
}

fn response_for_dialog_validation_error(
    request: &SipRequest,
    error: &DialogValidationError,
) -> Vec<u8> {
    let (status_code, reason_phrase) = error.status();
    response::build_response_with_owned_headers(
        request,
        status_code,
        reason_phrase,
        &[("X-VOS-RS-Error".to_string(), error.to_string())],
        "",
    )
}

fn response_for_media_error(request: &SipRequest, error: &media::MediaError) -> Vec<u8> {
    match error {
        media::MediaError::PortRangeExhausted { .. } => {
            response::service_unavailable_for_request(request, &error.to_string())
        }
        _ => response::not_acceptable_for_request(request, &error.to_string()),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RewrittenSdp {
    original_endpoint: Option<RtpEndpoint>,
    relay_endpoint: RtpEndpoint,
    body: Vec<u8>,
}

fn prepare_rewritten_sdp(
    headers: &HeaderMap,
    body: &[u8],
    media_relay: &MediaRelayState,
    media_config: &MediaConfig,
    direction: &'static str,
) -> Result<Option<RewrittenSdp>, media::MediaError> {
    if !media::is_sdp_body(headers, body) {
        return Ok(None);
    }

    let relay_endpoint = media_relay.allocate_endpoint(media_config)?;
    match media::rewrite_sdp_and_extract_endpoint(body, &relay_endpoint) {
        Ok((body, original_endpoint)) => Ok(Some(RewrittenSdp {
            original_endpoint: Some(original_endpoint),
            relay_endpoint,
            body,
        })),
        Err(error) => {
            media_relay.clear_target(relay_endpoint.port);
            warn!(%error, direction, "failed to rewrite SDP body for media relay");
            Err(error)
        }
    }
}

fn register_relay_target(
    media_relay: &MediaRelayState,
    relay_endpoint: &RtpEndpoint,
    target_endpoint: &RtpEndpoint,
    direction: &'static str,
) {
    if let Err(error) = media_relay.set_target(relay_endpoint, target_endpoint) {
        warn!(%error, direction, "failed to register RTP relay target");
    }
}

/// Replace the value of a SIP header in a raw message string.
/// Only replaces the first occurrence (case-insensitive header name match).
fn replace_header_value(raw: &str, header_name: &str, new_value: &str) -> String {
    let needle_lower = header_name.to_ascii_lowercase();
    let mut result = String::with_capacity(raw.len() + 8);
    for line in raw.split_inclusive("\r\n") {
        let header_part = line.split(':').next().unwrap_or("");
        if header_part.trim().to_ascii_lowercase() == needle_lower {
            result.push_str(&format!("{header_name}: {new_value}\r\n"));
        } else {
            result.push_str(line);
        }
    }
    result
}

fn parse_sip_info_dtmf(content_type: &str, body: &[u8]) -> Option<char> {
    let body_str = std::str::from_utf8(body).ok()?.trim();
    if content_type.contains("application/dtmf-relay") {
        for line in body_str.lines() {
            let line = line.trim();
            if line.to_ascii_lowercase().starts_with("signal=") {
                let parts: Vec<&str> = line.split('=').collect();
                if parts.len() == 2 {
                    let signal = parts[1].trim();
                    if signal.len() == 1 {
                        let c = signal.chars().next()?;
                        if c.is_ascii_digit() || c == '*' || c == '#' || ('A'..='D').contains(&c) {
                            return Some(c);
                        }
                    }
                }
            }
        }
    } else if content_type.contains("application/dtmf") && body_str.len() == 1 {
        let c = body_str.chars().next()?;
        if c.is_ascii_digit() || c == '*' || c == '#' || ('A'..='D').contains(&c) {
            return Some(c);
        }
    }
    None
}
#[cfg(test)]
mod tests {
    use super::auth::{digest_response, AuthConfig};
    use super::{
        flush_completed_cdrs, handle_datagram, handle_ws_connection, media, response,
        spawn_client_transaction_retransmission, spawn_nat_keepalive_loop,
        spawn_session_timer_watchdog, CdrSinks, ClientTransactionKey, EdgeConfig, EdgeState,
        PendingDatagram,
    };
    use call_core::{CallId, CallManager, CallState, Route, RouteTable, RouteTarget};
    use sdp_core::RtpEndpoint;
    use sip_core::{parse_message, SipMessage, SipUri};
    use std::{collections::HashMap, net::SocketAddr, str::FromStr, sync::Arc, time::Duration};
    use tokio::net::UdpSocket;

    include!("tests/all_tests.rs");
}
