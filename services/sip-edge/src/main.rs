mod auth;
mod dialog;
mod media;
mod nats_cdr;
mod outbound;
mod registrar;
mod response;
pub mod sbc;
mod transaction;
mod transport;

use auth::{AuthConfig, AuthDecision};
use call_core::{
    CallError, CallManager, CallQualityMetrics, GatewayHealthTracker, Route, RouteTable, RouteTarget,
};
use cdr_core::{PostgresCdrStore, DEFAULT_CDR_STREAM, DEFAULT_CDR_SUBJECT};
use dialog::DialogValidationError;
use media::{MediaConfig, MediaRelayState};
use nats_cdr::NatsCdrPublisher;
use registrar::{RegisterOutcome, RegistrationStore};
use rustls_pki_types::ServerName;
use sdp_core::RtpEndpoint;
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
use tokio::net::{TcpStream, UdpSocket};
use tracing::{debug, error, info, warn};
use tracing_subscriber::EnvFilter;
use transaction::{ClientTransactionKey, RequestTransactionKey};
use transport::{
    create_tls_acceptor, create_tls_connector, handle_stream_connection, handle_ws_connection,
    SipStream, Transport,
};

const ADVERTISED_ADDR_ENV: &str = "VOS_RS_SIP_ADVERTISED_ADDR";
const DATABASE_URL_ENV: &str = "VOS_RS_DATABASE_URL";
const NATS_URL_ENV: &str = "VOS_RS_NATS_URL";
const NATS_CDR_STREAM_ENV: &str = "VOS_RS_NATS_CDR_STREAM";
const NATS_CDR_SUBJECT_ENV: &str = "VOS_RS_NATS_CDR_SUBJECT";
const DEFAULT_BIND_ADDR: &str = "0.0.0.0:5060";
const DEFAULT_ADVERTISED_ADDR: &str = "127.0.0.1:5060";
const DEFAULT_GATEWAY_ENV: &str = "VOS_RS_SIP_DEFAULT_GATEWAY";
const TLS_BIND_ENV: &str = "VOS_RS_SIP_TLS_BIND";
const TLS_CERT_PATH_ENV: &str = "VOS_RS_SIP_TLS_CERT_PATH";
const TLS_KEY_PATH_ENV: &str = "VOS_RS_SIP_TLS_KEY_PATH";
const TLS_ALLOW_TEST_CERT_ENV: &str = "VOS_RS_SIP_TLS_ALLOW_TEST_CERT";
const TLS_CA_PATH_ENV: &str = "VOS_RS_SIP_TLS_CA_PATH";
const TLS_INSECURE_SKIP_VERIFY_ENV: &str = "VOS_RS_SIP_TLS_INSECURE_SKIP_VERIFY";
const TLS_SERVER_NAME_ENV: &str = "VOS_RS_SIP_TLS_SERVER_NAME";
const MAX_DATAGRAM_SIZE: usize = 65_535;

type AnyError = Box<dyn std::error::Error + Send + Sync>;

#[derive(Debug, Clone, PartialEq)]
struct EdgeConfig {
    advertised_addr: String,
    media: MediaConfig,
    auth: AuthConfig,
    /// Session-Expires (seconds) advertised to gateway leg. Default: 600.
    session_expires_gateway: u32,
    /// Session-Expires (seconds) advertised to caller leg. Default: 1800.
    session_expires_caller: u32,
    sbc_allow_rules: Vec<String>,
    sbc_block_rules: Vec<String>,
    sbc_rate_limit_capacity: f64,
    sbc_rate_limit_fill_rate: f64,
    sbc_max_concurrency: u32,
    tls_cert_path: Option<String>,
    tls_key_path: Option<String>,
    tls_allow_test_certificate: bool,
    tls_ca_path: Option<String>,
    tls_insecure_skip_verify: bool,
    tls_server_name: Option<String>,
}

impl EdgeConfig {
    fn from_env() -> Self {
        let sbc_allow_rules = env::var("VOS_RS_SBC_ALLOW")
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        let sbc_block_rules = env::var("VOS_RS_SBC_BLOCK")
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        let sbc_rate_limit_capacity = env::var("VOS_RS_SBC_LIMIT_CAPACITY")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(100.0);
        let sbc_rate_limit_fill_rate = env::var("VOS_RS_SBC_LIMIT_FILL_RATE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(10.0);
        let sbc_max_concurrency = env::var("VOS_RS_SBC_MAX_CONCURRENCY")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(10);

        Self {
            advertised_addr: env::var(ADVERTISED_ADDR_ENV)
                .unwrap_or_else(|_| DEFAULT_ADVERTISED_ADDR.to_string()),
            media: MediaConfig::from_env(),
            auth: AuthConfig::from_env(),
            session_expires_gateway: env::var("VOS_RS_SESSION_EXPIRES_GATEWAY")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(600),
            session_expires_caller: env::var("VOS_RS_SESSION_EXPIRES_CALLER")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(1800),
            sbc_allow_rules,
            sbc_block_rules,
            sbc_rate_limit_capacity,
            sbc_rate_limit_fill_rate,
            sbc_max_concurrency,
            tls_cert_path: env_non_empty(TLS_CERT_PATH_ENV),
            tls_key_path: env_non_empty(TLS_KEY_PATH_ENV),
            tls_allow_test_certificate: env_bool(TLS_ALLOW_TEST_CERT_ENV).unwrap_or(false),
            tls_ca_path: env_non_empty(TLS_CA_PATH_ENV),
            tls_insecure_skip_verify: env_bool(TLS_INSECURE_SKIP_VERIFY_ENV).unwrap_or(false),
            tls_server_name: env_non_empty(TLS_SERVER_NAME_ENV),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingDatagram {
    target: String,
    bytes: Vec<u8>,
}

impl PendingDatagram {
    fn new(target: impl Into<String>, bytes: Vec<u8>) -> Self {
        Self {
            target: target.into(),
            bytes,
        }
    }
}

#[derive(Debug, Clone, Default)]
struct CdrSinks {
    postgres: Option<PostgresCdrStore>,
    nats: Option<NatsCdrPublisher>,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
struct ReferSubscription {
    refer_to: String,
    from_header: String,
    to_header: String,
    notify_cseq: u32,
    transfer_call_id: String,
    referrer_peer: String,
    refer_cseq: u32,
    target_relay_port: Option<u16>,
    transferee_relay_port: Option<u16>,
}

#[derive(Debug, Clone)]
struct InboundTransaction {
    peer: String,
    outbound_peer: Option<String>,
    vias: Vec<String>,
    outbound_uri: SipUri,
    inbound_from_tag: Option<String>,
    inbound_to_tag: Option<String>,
    last_inbound_cseq: Option<u32>,
    last_outbound_cseq: Option<u32>,
    caller_rtp: Option<RtpEndpoint>,
    gateway_relay_rtp: Option<RtpEndpoint>,
    gateway_rtp: Option<RtpEndpoint>,
    caller_relay_rtp: Option<RtpEndpoint>,
    original_request: Option<SipRequest>,
    inbound_route_set: Vec<String>,
    outbound_route_set: Vec<String>,
    caller_contact: Option<SipUri>,
    callee_contact: Option<SipUri>,
    /// Negotiated Session-Expires value in seconds (from 200 OK)
    session_expires: Option<u32>,
    /// Who is responsible for refresh: "uac" (caller) or "uas" (gateway)
    session_refresher: Option<String>,
    /// Timestamp of the last session refresh (Re-INVITE / UPDATE received)
    last_session_refresh: Option<Instant>,
    /// RSeq counter for 100rel provisional responses sent toward the caller
    prack_rseq: u32,
    /// Whether the gateway negotiated 100rel (Require: 100rel seen in a 1xx)
    gateway_100rel: bool,
    /// Information about any active REFER transfer subscription handled locally
    refer_subscription: Option<ReferSubscription>,
    transfer_from_header: Option<String>,
    transfer_to_header: Option<String>,
    transfer_call_id: Option<String>,
    transfer_contact: Option<SipUri>,
    transfer_peer: Option<String>,
    transferee_is_caller: bool,
    callee_behind_nat: bool,
}

impl InboundTransaction {
    fn validate_in_dialog_request(
        &self,
        request: &SipRequest,
        peer: SocketAddr,
    ) -> Result<(DialogLeg, Option<u32>), DialogValidationError> {
        let leg = self
            .dialog_leg_for_peer(peer)
            .ok_or(DialogValidationError::PeerMismatch)?;

        let request_from_tag = request
            .headers
            .get("from")
            .and_then(|value| dialog::tag_param(value.as_str()))
            .ok_or(DialogValidationError::MissingFromTag)?;
        let expected_from_tag = match leg {
            DialogLeg::Caller => self.inbound_from_tag.as_ref(),
            DialogLeg::Gateway => self.inbound_to_tag.as_ref(),
        };
        if expected_from_tag.is_some_and(|tag| tag != &request_from_tag) {
            return Err(DialogValidationError::FromTagMismatch);
        }

        if !matches!(&request.method, Method::Cancel) {
            let expected_to_tag = match leg {
                DialogLeg::Caller => self.inbound_to_tag.as_ref(),
                DialogLeg::Gateway => self.inbound_from_tag.as_ref(),
            };
            if let Some(expected_to_tag) = expected_to_tag {
                let request_to_tag = request
                    .headers
                    .get("to")
                    .and_then(|value| dialog::tag_param(value.as_str()));
                if request_to_tag.as_ref() != Some(expected_to_tag) {
                    return Err(DialogValidationError::ToTagMismatch);
                }
            }
        }

        if !matches!(&request.method, Method::Ack | Method::Cancel) {
            let cseq = request
                .headers
                .get("cseq")
                .ok_or(DialogValidationError::MissingCSeq)
                .and_then(|value| {
                    dialog::cseq_number(value.as_str()).ok_or(DialogValidationError::InvalidCSeq)
                })?;
            let last_cseq = match leg {
                DialogLeg::Caller => self.last_inbound_cseq,
                DialogLeg::Gateway => self.last_outbound_cseq,
            };
            if let Some(last_cseq) = last_cseq {
                if cseq <= last_cseq {
                    return Err(DialogValidationError::CSeqOutOfOrder {
                        received: cseq,
                        last: last_cseq,
                    });
                }
            }
            return Ok((leg, Some(cseq)));
        }

        Ok((leg, None))
    }

    fn dialog_leg_for_peer(&self, peer: SocketAddr) -> Option<DialogLeg> {
        let peer = peer.to_string();
        if self.peer == peer {
            return Some(DialogLeg::Caller);
        }
        if self.outbound_peer.as_deref() == Some(peer.as_str()) {
            return Some(DialogLeg::Gateway);
        }
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DialogLeg {
    Caller,
    Gateway,
}

fn extract_uri_from_contact(contact: &str) -> Option<SipUri> {
    let contact = contact.trim();
    let uri_str = if let Some(start) = contact.find('<') {
        let end = contact.find('>')?;
        &contact[start + 1..end]
    } else {
        contact.split(';').next()?
    };
    SipUri::from_str(uri_str.trim()).ok()
}

fn sip_uri_from_peer(peer: &str) -> SipUri {
    match peer.parse::<SocketAddr>() {
        Ok(addr) => SipUri {
            secure: false,
            user: None,
            host: match addr.ip() {
                std::net::IpAddr::V4(ip) => ip.to_string(),
                std::net::IpAddr::V6(ip) => format!("[{ip}]"),
            },
            port: Some(addr.port()),
            params: Vec::new(),
        },
        Err(_) => SipUri {
            secure: false,
            user: None,
            host: peer
                .split(':')
                .next()
                .filter(|host| !host.is_empty())
                .unwrap_or(peer)
                .to_string(),
            port: None,
            params: Vec::new(),
        },
    }
}

fn parse_target_addr_from_route(route: &str) -> Option<String> {
    let route = route.trim();
    let uri_str = if let Some(start) = route.find('<') {
        let end = route.find('>')?;
        &route[start + 1..end]
    } else {
        route.split(';').next()?
    };
    let uri = SipUri::from_str(uri_str.trim()).ok()?;
    Some(format!("{}:{}", uri.host, uri.port.unwrap_or(5060)))
}

mod manage;

#[derive(Debug)]
pub(crate) struct EdgeState {
    call_manager: std::sync::Arc<CallManager>,
    gateway_health: std::sync::Mutex<GatewayHealthTracker>,
    inbound_transactions: dashmap::DashMap<String, InboundTransaction>,
    media_relay: MediaRelayState,
    registrar: tokio::sync::Mutex<RegistrationStore>,
    db_store: Option<PostgresCdrStore>,
    client_transactions:
        dashmap::DashMap<ClientTransactionKey, tokio::sync::oneshot::Sender<()>>,
    draining: std::sync::atomic::AtomicBool,
    refer_transfers: dashmap::DashMap<String, String>,
    bridged_transfers: dashmap::DashMap<String, String>,
    server_transactions: dashmap::DashMap<
        RequestTransactionKey,
        tokio::sync::mpsc::Sender<transaction::ServerTransactionEvent>,
    >,
    socket: std::sync::Mutex<Option<Arc<UdpSocket>>>,
    test_request_cache: dashmap::DashMap<RequestTransactionKey, Vec<PendingDatagram>>,
    nonce_replay_cache: std::sync::Mutex<HashMap<String, u64>>,
    tcp_connections: dashmap::DashMap<SocketAddr, tokio::sync::mpsc::Sender<Vec<u8>>>,
    sbc_engine: sbc::SbcEngine,
    external_to_internal_call_ids: dashmap::DashMap<String, String>,
    internal_to_external_call_ids: dashmap::DashMap<String, String>,
    #[cfg(test)]
    test_gateways: std::sync::Mutex<Vec<String>>,
}

impl EdgeState {
    #[cfg(test)]
    fn new(call_manager: CallManager) -> Self {
        Self::with_media_relay_and_db(call_manager, MediaRelayState::new(), None)
    }

    fn with_media_relay_and_db(
        call_manager: CallManager,
        media_relay: MediaRelayState,
        db_store: Option<PostgresCdrStore>,
    ) -> Self {
        let config = EdgeConfig::from_env();
        let sbc_engine = sbc::SbcEngine::new(
            &config
                .sbc_allow_rules
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>(),
            &config
                .sbc_block_rules
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>(),
            config.sbc_rate_limit_capacity,
            config.sbc_rate_limit_fill_rate,
        );

        Self {
            call_manager: std::sync::Arc::new(call_manager),
            gateway_health: std::sync::Mutex::new(GatewayHealthTracker::default()),
            inbound_transactions: dashmap::DashMap::new(),
            media_relay,
            registrar: tokio::sync::Mutex::new(RegistrationStore::new()),
            db_store,
            client_transactions: dashmap::DashMap::new(),
            draining: std::sync::atomic::AtomicBool::new(false),
            refer_transfers: dashmap::DashMap::new(),
            bridged_transfers: dashmap::DashMap::new(),
            server_transactions: dashmap::DashMap::new(),
            socket: std::sync::Mutex::new(None),
            test_request_cache: dashmap::DashMap::new(),
            nonce_replay_cache: std::sync::Mutex::new(HashMap::new()),
            tcp_connections: dashmap::DashMap::new(),
            sbc_engine,
            external_to_internal_call_ids: dashmap::DashMap::new(),
            internal_to_external_call_ids: dashmap::DashMap::new(),
            #[cfg(test)]
            test_gateways: std::sync::Mutex::new(Vec::new()),
        }
    }

    #[cfg(test)]
    fn with_config(call_manager: CallManager, config: &EdgeConfig) -> Self {
        let sbc_engine = sbc::SbcEngine::new(
            &config
                .sbc_allow_rules
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>(),
            &config
                .sbc_block_rules
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>(),
            config.sbc_rate_limit_capacity,
            config.sbc_rate_limit_fill_rate,
        );

        Self {
            call_manager: std::sync::Arc::new(call_manager),
            gateway_health: std::sync::Mutex::new(GatewayHealthTracker::default()),
            inbound_transactions: dashmap::DashMap::new(),
            media_relay: MediaRelayState::new(),
            registrar: tokio::sync::Mutex::new(RegistrationStore::new()),
            db_store: None,
            client_transactions: dashmap::DashMap::new(),
            draining: std::sync::atomic::AtomicBool::new(false),
            refer_transfers: dashmap::DashMap::new(),
            bridged_transfers: dashmap::DashMap::new(),
            server_transactions: dashmap::DashMap::new(),
            socket: std::sync::Mutex::new(None),
            test_request_cache: dashmap::DashMap::new(),
            nonce_replay_cache: std::sync::Mutex::new(HashMap::new()),
            tcp_connections: dashmap::DashMap::new(),
            sbc_engine,
            external_to_internal_call_ids: dashmap::DashMap::new(),
            internal_to_external_call_ids: dashmap::DashMap::new(),
            test_gateways: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn get_internal_call_id(&self, external_call_id: &str) -> Option<String> {
        self.external_to_internal_call_ids
            .get(external_call_id)
            .map(|r| r.clone())
    }

    fn get_external_call_id(&self, internal_call_id: &str) -> Option<String> {
        self.internal_to_external_call_ids
            .get(internal_call_id)
            .map(|r| r.clone())
    }

    fn register_call_id_mapping(&self, internal_call_id: &str, external_call_id: &str) {
        self.internal_to_external_call_ids
            .insert(internal_call_id.to_string(), external_call_id.to_string());
        self.external_to_internal_call_ids
            .insert(external_call_id.to_string(), internal_call_id.to_string());
    }

    /// Remove a transaction, apply a closure, and re-insert it.
    /// Returns None if the call_id doesn't exist.
    fn with_transaction_mut<F, R>(&self, call_id: &str, f: F) -> Option<R>
    where
        F: FnOnce(&mut InboundTransaction) -> R,
    {
        let mut tx = self.inbound_transactions.get_mut(call_id)?;
        Some(f(&mut tx))
    }

    fn with_transaction<F, R>(&self, call_id: &str, f: F) -> Option<R>
    where
        F: FnOnce(&InboundTransaction) -> R,
    {
        let tx = self.inbound_transactions.get(call_id)?;
        Some(f(&*tx))
    }

    fn set_socket(&self, socket: Arc<UdpSocket>) {
        *self.socket.lock().unwrap() = Some(socket);
    }

    fn get_socket(&self) -> Option<Arc<UdpSocket>> {
        self.socket.lock().unwrap().clone()
    }

    fn register_tcp_connection(&self, addr: SocketAddr, tx: tokio::sync::mpsc::Sender<Vec<u8>>) {
        self.tcp_connections.insert(addr, tx);
    }

    fn get_tcp_connection(&self, addr: &SocketAddr) -> Option<tokio::sync::mpsc::Sender<Vec<u8>>> {
        if let Some(tx) = self.tcp_connections.get(addr) {
            if tx.is_closed() {
                drop(tx);
                self.tcp_connections.remove(addr);
                None
            } else {
                Some(tx.clone())
            }
        } else {
            None
        }
    }

    async fn send_sip_datagram(
        self: &Arc<Self>,
        datagram: PendingDatagram,
        fallback_socket: &UdpSocket,
        edge_config: &EdgeConfig,
    ) -> Result<(), std::io::Error> {
        let target_addr: SocketAddr = match datagram.target.parse() {
            Ok(addr) => addr,
            Err(_) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "invalid target address",
                ));
            }
        };

        let mut transport = Transport::Udp;
        if let Ok(msg) = parse_message(&datagram.bytes) {
            if let Some(via) = msg.headers().get("via") {
                let via_str = via.as_str().to_uppercase();
                if via_str.contains("SIP/2.0/TLS") {
                    transport = Transport::Tls;
                } else if via_str.contains("SIP/2.0/TCP") {
                    transport = Transport::Tcp;
                } else if via_str.contains("SIP/2.0/WSS") {
                    transport = Transport::Wss;
                } else if via_str.contains("SIP/2.0/WS") {
                    transport = Transport::Ws;
                }
            }
        }

        if let Some(tx) = self.get_tcp_connection(&target_addr) {
            if tx.send(datagram.bytes.clone()).await.is_ok() {
                return Ok(());
            }
        }

        match transport {
            Transport::Tcp => {
                debug!(%target_addr, "establishing new outbound TCP connection");
                match TcpStream::connect(target_addr).await {
                    Ok(stream) => {
                        let (tx, rx) = tokio::sync::mpsc::channel(100);
                        self.register_tcp_connection(target_addr, tx.clone());

                        let state_clone = Arc::clone(self);
                        let config_clone = edge_config.clone();
                        tokio::spawn(handle_stream_connection(
                            SipStream::Tcp(stream),
                            target_addr,
                            tx.clone(),
                            rx,
                            move |msg_bytes, peer_addr, connection_tx| {
                                let state = Arc::clone(&state_clone);
                                let config = config_clone.clone();
                                async move {
                                    let datagrams =
                                        handle_datagram(&msg_bytes, peer_addr, &state, &config)
                                            .await;
                                    for d in datagrams {
                                        let _ = connection_tx.send(d.bytes).await;
                                    }
                                }
                            },
                        ));

                        let _ = tx.send(datagram.bytes).await;
                        Ok(())
                    }
                    Err(e) => {
                        error!(%target_addr, error = %e, "failed to establish outbound TCP connection");
                        Err(e)
                    }
                }
            }
            Transport::Tls => {
                debug!(%target_addr, "establishing new outbound TLS connection");
                match TcpStream::connect(target_addr).await {
                    Ok(stream) => {
                        let connector = create_tls_connector(
                            edge_config.tls_ca_path.as_deref(),
                            edge_config.tls_insecure_skip_verify,
                        )
                        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
                        let domain = match &edge_config.tls_server_name {
                            Some(name) => ServerName::try_from(name.clone()).map_err(|_| {
                                std::io::Error::new(
                                    std::io::ErrorKind::InvalidInput,
                                    format!("invalid TLS server name: {name}"),
                                )
                            })?,
                            None => ServerName::from(target_addr.ip()),
                        };
                        match connector.connect(domain, stream).await {
                            Ok(tls_stream) => {
                                let (tx, rx) = tokio::sync::mpsc::channel(100);
                                self.register_tcp_connection(target_addr, tx.clone());

                                let state_clone = Arc::clone(self);
                                let config_clone = edge_config.clone();
                                tokio::spawn(handle_stream_connection(
                                    SipStream::TlsClient(tls_stream),
                                    target_addr,
                                    tx.clone(),
                                    rx,
                                    move |msg_bytes, peer_addr, connection_tx| {
                                        let state = Arc::clone(&state_clone);
                                        let config = config_clone.clone();
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
                                ));

                                let _ = tx.send(datagram.bytes).await;
                                Ok(())
                            }
                            Err(e) => {
                                error!(%target_addr, error = %e, "failed to establish outbound TLS handshake");
                                Err(std::io::Error::new(
                                    std::io::ErrorKind::ConnectionRefused,
                                    e,
                                ))
                            }
                        }
                    }
                    Err(e) => {
                        error!(%target_addr, error = %e, "failed to connect TCP for TLS");
                        Err(e)
                    }
                }
            }
            Transport::Ws | Transport::Wss => {
                error!(%target_addr, "no active WebSocket connection found for outbound datagram");
                Err(std::io::Error::new(
                    std::io::ErrorKind::NotConnected,
                    "no active WebSocket connection",
                ))
            }
            Transport::Udp => {
                fallback_socket
                    .send_to(&datagram.bytes, target_addr)
                    .await?;
                Ok(())
            }
        }
    }

    pub async fn send_keepalive_probe(&self, target_str: &str, fallback_socket: &UdpSocket) {
        let Ok(target_addr) = target_str.parse::<SocketAddr>() else {
            return;
        };

        if let Some(tx) = self.get_tcp_connection(&target_addr) {
            let _ = tx.send(b"\r\n\r\n".to_vec()).await;
            return;
        }

        let _ = fallback_socket.send_to(b"\r\n", target_addr).await;
    }

    fn register_server_transaction(
        &self,
        key: RequestTransactionKey,
        tx: tokio::sync::mpsc::Sender<transaction::ServerTransactionEvent>,
    ) {
        self.server_transactions.insert(key, tx);
    }

    fn get_server_transaction(
        &self,
        key: &RequestTransactionKey,
    ) -> Option<tokio::sync::mpsc::Sender<transaction::ServerTransactionEvent>> {
        if let Some(tx) = self.server_transactions.get(key) {
            if tx.is_closed() {
                drop(tx);
                self.server_transactions.remove(key);
                None
            } else {
                Some(tx.clone())
            }
        } else {
            None
        }
    }

    async fn is_peer_gateway(&self, peer: SocketAddr) -> bool {
        let peer_ip = peer.ip().to_string();

        #[cfg(test)]
        {
            if self.test_gateways.lock().unwrap().contains(&peer_ip) {
                return true;
            }
        }

        if let Some(ref db) = self.db_store {
            if let Ok(gateways) = db.load_gateways().await {
                for (_id, host, _port, _transport, _cap) in gateways {
                    if host == peer_ip {
                        return true;
                    }
                }
            }
        }
        false
    }

    fn cancel_client_transaction(&self, key: &ClientTransactionKey) {
        if let Some((_, cancel_tx)) = self.client_transactions.remove(key) {
            let _ = cancel_tx.send(());
        }
    }

    fn remember_inbound_invite(
        &self,
        request: &SipRequest,
        peer: SocketAddr,
        outbound_uri: SipUri,
        caller_rtp: Option<RtpEndpoint>,
        gateway_relay_rtp: Option<RtpEndpoint>,
        callee_behind_nat: bool,
    ) {
        let Some(call_id) = request.headers.get("call-id") else {
            return;
        };

        let vias = request
            .headers
            .get_all("via")
            .map(|value| value.as_str().to_string())
            .collect::<Vec<_>>();

        let inbound_route_set = request
            .headers
            .get_all("record-route")
            .map(|value| value.as_str().to_string())
            .collect::<Vec<_>>();
        let caller_contact = request
            .headers
            .get("contact")
            .and_then(|value| extract_uri_from_contact(value.as_str()))
            .map(|mut uri| {
                if uri.port.is_none() {
                    uri.port = Some(peer.port());
                }
                uri
            });

        self.inbound_transactions.insert(
            call_id.as_str().to_string(),
            InboundTransaction {
                peer: peer.to_string(),
                outbound_peer: None,
                vias,
                outbound_uri,
                inbound_from_tag: request
                    .headers
                    .get("from")
                    .and_then(|value| dialog::tag_param(value.as_str())),
                inbound_to_tag: request
                    .headers
                    .get("to")
                    .and_then(|value| dialog::tag_param(value.as_str())),
                last_inbound_cseq: request
                    .headers
                    .get("cseq")
                    .and_then(|value| dialog::cseq_number(value.as_str())),
                last_outbound_cseq: None,
                caller_rtp,
                gateway_relay_rtp,
                gateway_rtp: None,
                caller_relay_rtp: None,
                original_request: Some(request.clone()),
                inbound_route_set,
                outbound_route_set: Vec::new(),
                caller_contact,
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
                callee_behind_nat,
            },
        );
    }

    fn remember_gateway_media(
        &self,
        call_id: &str,
        gateway_rtp: Option<RtpEndpoint>,
        caller_relay_rtp: RtpEndpoint,
        media_config: &MediaConfig,
    ) {
        let gateway_relay_port = self.inbound_transactions.get(call_id)
            .and_then(|t| t.gateway_relay_rtp.as_ref().map(|ep| ep.port));

        if let Some(mut transaction) = self.inbound_transactions.get_mut(call_id) {
            transaction.gateway_rtp = gateway_rtp;
            if let Some(gw_port) = gateway_relay_port {
                self.media_relay
                    .pair_ports(gw_port, caller_relay_rtp.port);
                match self.media_relay.start_call_recording(
                    call_id,
                    caller_relay_rtp.port,
                    gw_port,
                    media_config,
                ) {
                    Ok(Some(path)) => {
                        debug!(call_id, path = %path.display(), "started call recording");
                    }
                    Ok(None) => {}
                    Err(error) => {
                        warn!(call_id, %error, "failed to start call recording");
                    }
                }
            }
            transaction.caller_relay_rtp = Some(caller_relay_rtp);
        }
    }

    fn remember_inbound_to_tag(&self, call_id: &str, response: &sip_core::SipResponse) {
        let Some(to_tag) = response
            .headers
            .get("to")
            .and_then(|value| dialog::tag_param(value.as_str()))
        else {
            return;
        };
        let Some(mut transaction) = self.inbound_transactions.get_mut(call_id) else {
            return;
        };

        match &transaction.inbound_to_tag {
            Some(existing_tag) if existing_tag != &to_tag => {
                warn!(
                    call_id,
                    existing_tag,
                    new_tag = %to_tag,
                    "ignoring additional dialog To tag; forked dialogs are not implemented yet"
                );
            }
            Some(_) => {}
            None => {
                transaction.inbound_to_tag = Some(to_tag);
            }
        }
    }

    fn clear_media_targets(&self, transaction: &InboundTransaction) {
        if let Some(endpoint) = &transaction.gateway_relay_rtp {
            let metrics = self.media_relay.metrics_for_port(endpoint.port);
            debug!(
                port = endpoint.port,
                received_packets = metrics.received_packets,
                forwarded_packets = metrics.forwarded_packets,
                dropped_invalid_packets = metrics.dropped_invalid_packets,
                dropped_no_target_packets = metrics.dropped_no_target_packets,
                send_errors = metrics.send_errors,
                learned_source_updates = metrics.learned_source_updates,
                rtcp_quality = ?metrics.rtcp_quality,
                recorded_packets = metrics.recorded_packets,
                recording_errors = metrics.recording_errors,
                dtmf_events = metrics.dtmf_events,
                "clearing gateway RTP relay target"
            );
            self.media_relay.clear_target(endpoint.port);
        }
        if let Some(endpoint) = &transaction.caller_relay_rtp {
            let metrics = self.media_relay.metrics_for_port(endpoint.port);
            debug!(
                port = endpoint.port,
                received_packets = metrics.received_packets,
                forwarded_packets = metrics.forwarded_packets,
                dropped_invalid_packets = metrics.dropped_invalid_packets,
                dropped_no_target_packets = metrics.dropped_no_target_packets,
                send_errors = metrics.send_errors,
                learned_source_updates = metrics.learned_source_updates,
                rtcp_quality = ?metrics.rtcp_quality,
                recorded_packets = metrics.recorded_packets,
                recording_errors = metrics.recording_errors,
                dtmf_events = metrics.dtmf_events,
                "clearing caller RTP relay target"
            );
            self.media_relay.clear_target(endpoint.port);
        }
        let totals = self.media_relay.metrics_totals();
        debug!(
            received_packets = totals.received_packets,
            forwarded_packets = totals.forwarded_packets,
            dropped_invalid_packets = totals.dropped_invalid_packets,
            dropped_no_target_packets = totals.dropped_no_target_packets,
            send_errors = totals.send_errors,
            learned_source_updates = totals.learned_source_updates,
            rtcp_quality = ?totals.rtcp_quality,
            recorded_packets = totals.recorded_packets,
            recording_errors = totals.recording_errors,
            dtmf_events = totals.dtmf_events,
            "RTP relay metrics totals"
        );
    }
}

fn current_hhmm() -> Option<String> {
    let fmt = time::format_description::parse("[hour]:[minute]").ok()?;
    time::OffsetDateTime::now_utc().format(&fmt).ok()
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), AnyError> {
    init_tracing();

    let bind_addr =
        env::var("VOS_RS_SIP_UDP_BIND").unwrap_or_else(|_| DEFAULT_BIND_ADDR.to_string());
    let route_table = route_table_from_env()?;
    if route_table.is_empty() {
        warn!(
            env = DEFAULT_GATEWAY_ENV,
            "no outbound route configured; INVITE requests will receive 404"
        );
    }
    let edge_config = EdgeConfig::from_env();
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
        let gateway_map: HashMap<String, (String, Option<u16>, String, Option<u32>)> =
            db_gateways
                .into_iter()
                .map(|(id, host, port, transport, cap)| (id, (host, port, transport, cap)))
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
            edge_state.call_manager
                .update_routes(RouteTable::new(routes));
            info!("loaded routes from database");
        }
    }

    let socket = Arc::new(UdpSocket::bind(&bind_addr).await?);
    edge_state.set_socket(Arc::clone(&socket));
    info!(%bind_addr, "sip-edge UDP listener started");

    // Start TCP listener
    let tcp_listener = match tokio::net::TcpListener::bind(&bind_addr).await {
        Ok(l) => {
            info!(%bind_addr, "sip-edge TCP listener started");
            Some(Arc::new(l))
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

    let mut buffer = [0u8; MAX_DATAGRAM_SIZE];
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

fn env_bool(name: &str) -> Option<bool> {
    let value = env::var(name).ok()?;
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn spawn_client_transaction_retransmission(
    edge_state: Arc<EdgeState>,
    socket: Arc<UdpSocket>,
    target: String,
    bytes: Vec<u8>,
    key: ClientTransactionKey,
    edge_config: EdgeConfig,
) {
    let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel::<()>();

    let key_clone = key.clone();
    tokio::spawn(async move {
        edge_state
            .client_transactions
            .insert(key_clone, cancel_tx);

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
fn spawn_session_timer_watchdog(
    edge_state: Arc<EdgeState>,
    socket: Arc<UdpSocket>,
    edge_config: EdgeConfig,
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
            let expired: Vec<(String, String, String)> = {
                edge_state.inbound_transactions.iter()
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

                // Clean up the transaction and call state
                edge_state
                    .inbound_transactions
                    .remove(&call_id);
                edge_state.call_manager
                    .terminate_call(&call_id);
                info!(call_id, "session-expired call terminated by watchdog");
            }
        }
    });
}

fn spawn_nat_keepalive_loop(edge_state: Arc<EdgeState>, socket: Arc<UdpSocket>) {
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

fn calculate_mos_for_legs(
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
    let cdrs = edge_state.call_manager
        .completed_cdrs()
        .to_vec();

    if cdrs.is_empty() {
        return Ok(());
    }

    if let Some(nats) = &cdr_sinks.nats {
        for cdr in &cdrs {
            nats.publish_cdr(cdr).await?;
        }

        let queued = edge_state.call_manager
            .take_completed_cdrs()
            .len();
        debug!(count = queued, "queued completed CDRs to NATS");
        return Ok(());
    }

    if let Some(cdr_store) = &cdr_sinks.postgres {
        for cdr in &cdrs {
            cdr_store.insert_call_cdr(cdr).await?;
        }

        let persisted = edge_state.call_manager
            .take_completed_cdrs()
            .len();
        debug!(count = persisted, "persisted completed CDRs to PostgreSQL");
        return Ok(());
    }

    {
        let dropped = edge_state.call_manager
            .take_completed_cdrs()
            .len();
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

async fn handle_datagram(
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
                            let (tx, rx) = tokio::sync::mpsc::channel(16);
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
                                edge_state.bridged_transfers.insert(cid.clone(), orig_cid.clone());
                                edge_state.bridged_transfers.insert(orig_cid.clone(), cid.clone());
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
                match edge_state.call_manager
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
                                        edge_state.inbound_transactions.get_mut(cid) {
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
                edge_state.inbound_transactions.iter()
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
            response::response_for_invite_to_uri(
                &request,
                &edge_state.call_manager,
                outbound_uri,
            )
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
        edge_state.call_manager
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
                original_request: Some(request.clone()),
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
                info!(call_id = cid, count = dtmf_events.len(), "collected DTMF audit events for call");
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

            match edge_state.call_manager
                .handle_inbound_termination(&mutable_request, metrics, dtmf_digits)
            {
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
                        edge_state.call_manager
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
                    if let Some(mut t_mut) = edge_state.inbound_transactions.get_mut(call_id.as_str()) {
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

                        if let Some(mut t_mut) = edge_state.inbound_transactions.get_mut(call_id.as_str()) {
                            t_mut.caller_rtp = Some(remote_ep);
                            t_mut.original_request = Some(mutable_request.clone());
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

                    if let Some(mut t_mut) = edge_state.inbound_transactions.get_mut(call_id.as_str()) {
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

    fn sdp_body() -> &'static str {
        concat!(
            "v=0\r\n",
            "o=caller 1 1 IN IP4 192.0.2.10\r\n",
            "s=caller\r\n",
            "c=IN IP4 192.0.2.10\r\n",
            "t=0 0\r\n",
            "m=audio 49170 RTP/AVP 0\r\n",
            "a=rtpmap:0 PCMU/8000\r\n"
        )
    }

    use super::media::MediaConfig;

    #[tokio::test]
    async fn replies_to_options() {
        let edge_state = state_without_routes();
        let request = concat!(
            "OPTIONS sip:edge.example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-1\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:edge.example.com>\r\n",
            "Call-ID: options-1@example.com\r\n",
            "CSeq: 1 OPTIONS\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        );

        let datagrams =
            handle_datagram(request.as_bytes(), peer(), &edge_state, &edge_config()).await;
        assert_eq!(datagrams.len(), 1);

        let response = datagram_text(&datagrams[0]);

        assert_eq!(datagrams[0].target, "192.0.2.10:5060");
        assert!(response.starts_with("SIP/2.0 200 OK\r\n"));
        assert!(response.contains("Allow: REGISTER, INVITE, ACK, BYE, CANCEL, OPTIONS, INFO\r\n"));
        assert!(response.contains("To: <sip:edge.example.com>;tag=vosrs-edge\r\n"));
    }

    #[tokio::test]
    async fn register_stores_contact_and_returns_binding() {
        let edge_state = state_without_routes();
        let request = concat!(
            "REGISTER sip:example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-reg\r\n",
            "Max-Forwards: 70\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:1001@example.com>\r\n",
            "Call-ID: reg-1@example.com\r\n",
            "CSeq: 1 REGISTER\r\n",
            "Contact: <sip:1001@192.0.2.10:5070;transport=udp>;expires=120\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        );

        let datagrams =
            handle_datagram(request.as_bytes(), peer(), &edge_state, &edge_config()).await;
        assert_eq!(datagrams.len(), 1);
        assert_eq!(datagrams[0].target, "192.0.2.10:5060");

        let response = datagram_text(&datagrams[0]);
        assert!(response.starts_with("SIP/2.0 200 OK\r\n"));
        assert!(response.contains("X-VOS-RS-AOR: sip:1001@example.com\r\n"));
        assert!(
            response.contains("Contact: <sip:1001@192.0.2.10:5070;transport=udp>;expires=120\r\n")
        );
        assert_eq!(edge_state.registrar.lock().await.binding_count(), 1);
    }

    #[tokio::test]
    async fn register_query_returns_existing_contact() {
        let edge_state = state_without_routes();
        let register = concat!(
            "REGISTER sip:example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-reg\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:1001@example.com>\r\n",
            "Call-ID: reg-query@example.com\r\n",
            "CSeq: 1 REGISTER\r\n",
            "Contact: <sip:1001@192.0.2.10:5070>;expires=120\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        );
        let _ = handle_datagram(register.as_bytes(), peer(), &edge_state, &edge_config()).await;

        let query = concat!(
            "REGISTER sip:example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-query\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:1001@example.com>\r\n",
            "Call-ID: reg-query@example.com\r\n",
            "CSeq: 2 REGISTER\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        );

        let datagrams =
            handle_datagram(query.as_bytes(), peer(), &edge_state, &edge_config()).await;

        assert_eq!(datagrams.len(), 1);
        let response = datagram_text(&datagrams[0]);
        assert!(response.starts_with("SIP/2.0 200 OK\r\n"));
        assert!(response.contains("Contact: <sip:1001@192.0.2.10:5070>;expires="));
    }

    #[tokio::test]
    async fn unregister_removes_contact() {
        let edge_state = state_without_routes();
        let register = concat!(
            "REGISTER sip:example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-reg\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:1001@example.com>\r\n",
            "Call-ID: reg-unregister@example.com\r\n",
            "CSeq: 1 REGISTER\r\n",
            "Contact: <sip:1001@192.0.2.10:5070>;expires=120\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        );
        let _ = handle_datagram(register.as_bytes(), peer(), &edge_state, &edge_config()).await;

        let unregister = concat!(
            "REGISTER sip:example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-unreg\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:1001@example.com>\r\n",
            "Call-ID: reg-unregister@example.com\r\n",
            "CSeq: 2 REGISTER\r\n",
            "Contact: <sip:1001@192.0.2.10:5070>;expires=0\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        );

        let datagrams =
            handle_datagram(unregister.as_bytes(), peer(), &edge_state, &edge_config()).await;

        assert_eq!(datagrams.len(), 1);
        let response = datagram_text(&datagrams[0]);
        assert!(response.starts_with("SIP/2.0 200 OK\r\n"));
        assert!(!response.contains("Contact: <sip:1001@192.0.2.10:5070>"));
        assert_eq!(edge_state.registrar.lock().await.binding_count(), 0);
    }

    #[tokio::test]
    async fn invite_to_registered_contact_bypasses_gateway_routes() {
        let edge_state = state_with_default_route();
        register_contact(&edge_state, "1002", "192.0.2.20", 5070).await;
        let invite = concat!(
            "INVITE sip:1002@example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-internal\r\n",
            "Max-Forwards: 70\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:1002@example.com>\r\n",
            "Call-ID: invite-internal@example.com\r\n",
            "CSeq: 1 INVITE\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        );

        let datagrams =
            handle_datagram(invite.as_bytes(), peer(), &edge_state, &edge_config()).await;

        assert_eq!(datagrams.len(), 2);
        assert_eq!(datagrams[0].target, "192.0.2.10:5060");
        assert_eq!(datagrams[1].target, "192.0.2.20:5070");

        let trying = datagram_text(&datagrams[0]);
        assert!(trying.starts_with("SIP/2.0 100 Trying\r\n"));

        let outbound_invite = datagram_text(&datagrams[1]);
        assert!(
            outbound_invite
                .starts_with("INVITE sip:1002@192.0.2.20:5070;transport=udp SIP/2.0\r\n"),
            "{outbound_invite}"
        );
        assert!(!outbound_invite.contains("gw1.example.com"));
    }

    #[tokio::test]
    async fn invite_to_registered_contact_works_without_default_route() {
        let edge_state = state_without_routes();
        register_contact(&edge_state, "1002", "192.0.2.20", 5070).await;
        let invite = concat!(
            "INVITE sip:1002@example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-internal-no-route\r\n",
            "Max-Forwards: 70\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:1002@example.com>\r\n",
            "Call-ID: invite-internal-no-route@example.com\r\n",
            "CSeq: 1 INVITE\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        );

        let datagrams =
            handle_datagram(invite.as_bytes(), peer(), &edge_state, &edge_config()).await;

        assert_eq!(datagrams.len(), 2);
        assert_eq!(datagrams[1].target, "192.0.2.20:5070");

        let guard = &edge_state.call_manager;
        let call = guard
            .get(&CallId::new("invite-internal-no-route@example.com"))
            .expect("call should be stored");
        assert_eq!(call.state, CallState::Routing);
        assert_eq!(
            call.outbound.as_ref().unwrap().remote_uri.to_string(),
            "sip:1002@192.0.2.20:5070;transport=udp"
        );
    }

    #[tokio::test]
    async fn invalid_register_receives_bad_request() {
        let edge_state = state_without_routes();
        let request = concat!(
            "REGISTER sip:example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-reg-bad\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:1001@example.com>\r\n",
            "Call-ID: reg-bad@example.com\r\n",
            "CSeq: 1 REGISTER\r\n",
            "Contact: *\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        );

        let datagrams =
            handle_datagram(request.as_bytes(), peer(), &edge_state, &edge_config()).await;

        assert_eq!(datagrams.len(), 1);
        let response = datagram_text(&datagrams[0]);
        assert!(response.starts_with("SIP/2.0 400 Bad Request\r\n"));
        assert!(response.contains("X-VOS-RS-Error: invalid REGISTER Contact: *\r\n"));
    }

    #[tokio::test]
    async fn register_requires_digest_auth_when_configured() {
        let edge_state = state_without_routes();
        let request = concat!(
            "REGISTER sip:example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-auth\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:1001@example.com>\r\n",
            "Call-ID: reg-auth@example.com\r\n",
            "CSeq: 1 REGISTER\r\n",
            "Contact: <sip:1001@192.0.2.10:5070>;expires=120\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        );

        let datagrams = handle_datagram(
            request.as_bytes(),
            peer(),
            &edge_state,
            &edge_config_with_auth(),
        )
        .await;

        assert_eq!(datagrams.len(), 1);
        let response = datagram_text(&datagrams[0]);
        assert!(response.starts_with("SIP/2.0 401 Unauthorized\r\n"));
        assert!(response.contains(
            "WWW-Authenticate: Digest realm=\"vos-rs\", nonce=\"test-nonce\", algorithm=MD5, qop=\"auth\"\r\n"
        ));
        assert_eq!(edge_state.registrar.lock().await.binding_count(), 0);
    }

    #[tokio::test]
    async fn register_accepts_valid_digest_auth_when_configured() {
        let edge_state = state_without_routes();
        let uri = "sip:example.com";
        let digest = digest_response(
            "1001",
            "secret",
            "vos-rs",
            "test-nonce",
            "REGISTER",
            uri,
            Some(("auth", "00000001", "abcdef")),
        );
        let request = format!(
            concat!(
                "REGISTER {uri} SIP/2.0\r\n",
                "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-auth-ok\r\n",
                "From: <sip:1001@example.com>;tag=from-tag\r\n",
                "To: <sip:1001@example.com>\r\n",
                "Call-ID: reg-auth-ok@example.com\r\n",
                "CSeq: 1 REGISTER\r\n",
                "Authorization: Digest username=\"1001\", realm=\"vos-rs\", nonce=\"test-nonce\", uri=\"{uri}\", response=\"{digest}\", algorithm=MD5, qop=auth, nc=00000001, cnonce=\"abcdef\"\r\n",
                "Contact: <sip:1001@192.0.2.10:5070>;expires=120\r\n",
                "Content-Length: 0\r\n",
                "\r\n"
            ),
            uri = uri,
            digest = digest
        );

        let datagrams = handle_datagram(
            request.as_bytes(),
            peer(),
            &edge_state,
            &edge_config_with_auth(),
        )
        .await;

        assert_eq!(datagrams.len(), 1);
        let response = datagram_text(&datagrams[0]);
        assert!(response.starts_with("SIP/2.0 200 OK\r\n"));
        assert!(response.contains("Contact: <sip:1001@192.0.2.10:5070>;expires=120\r\n"));
        assert_eq!(edge_state.registrar.lock().await.binding_count(), 1);
    }

    #[tokio::test]
    async fn replies_to_invite_with_trying_and_dispatches_outbound_invite() {
        let edge_state = state_with_default_route();
        let request = concat!(
            "INVITE sip:13800138000@example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-2\r\n",
            "Max-Forwards: 70\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:13800138000@example.com>\r\n",
            "Call-ID: invite-1@example.com\r\n",
            "CSeq: 1 INVITE\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        );

        let datagrams =
            handle_datagram(request.as_bytes(), peer(), &edge_state, &edge_config()).await;
        assert_eq!(datagrams.len(), 2);

        let response = datagram_text(&datagrams[0]);
        assert_eq!(datagrams[0].target, "192.0.2.10:5060");
        assert!(response.starts_with("SIP/2.0 100 Trying\r\n"));
        assert!(response.contains("Call-ID: invite-1@example.com\r\n"));
        assert!(response.contains("CSeq: 1 INVITE\r\n"));
        assert!(response.contains("To: <sip:13800138000@example.com>;tag=vosrs-edge\r\n"));

        let outbound_invite = datagram_text(&datagrams[1]);
        assert_eq!(datagrams[1].target, "gw1.example.com:5060");
        assert!(outbound_invite
            .starts_with("INVITE sip:13800138000@gw1.example.com:5060;transport=udp SIP/2.0\r\n"));
        assert!(outbound_invite.contains("Via: SIP/2.0/UDP edge.example.com:5060;branch="));
        assert!(outbound_invite.contains("Max-Forwards: 69\r\n"));
        assert!(outbound_invite.contains("Contact: <sip:vosrs@edge.example.com:5060>\r\n"));
        assert_eq!(edge_state.call_manager.len(), 1);
    }

    #[tokio::test]
    async fn retransmitted_invite_replays_trying_without_duplicate_outbound_invite() {
        let edge_state = state_with_default_route();
        let request = concat!(
            "INVITE sip:13800138000@example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-retry-invite\r\n",
            "Max-Forwards: 70\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:13800138000@example.com>\r\n",
            "Call-ID: invite-retry@example.com\r\n",
            "CSeq: 1 INVITE\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        );

        let first = handle_datagram(request.as_bytes(), peer(), &edge_state, &edge_config()).await;
        let second = handle_datagram(request.as_bytes(), peer(), &edge_state, &edge_config()).await;

        assert_eq!(first.len(), 2);
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].target, "192.0.2.10:5060");

        let response = datagram_text(&second[0]);
        assert!(response.starts_with("SIP/2.0 100 Trying\r\n"));
        assert!(response.contains("Call-ID: invite-retry@example.com\r\n"));
        assert_eq!(edge_state.call_manager.len(), 1);
    }

    #[tokio::test]
    async fn rewrites_invite_sdp_offer_for_gateway_media_relay() {
        let edge_state = state_with_default_route();
        let body = concat!(
            "v=0\r\n",
            "o=caller 1 1 IN IP4 192.0.2.10\r\n",
            "s=caller\r\n",
            "c=IN IP4 192.0.2.10\r\n",
            "t=0 0\r\n",
            "m=audio 49170 RTP/AVP 0 8 101\r\n",
            "a=rtpmap:0 PCMU/8000\r\n",
            "a=rtpmap:8 PCMA/8000\r\n",
            "a=rtpmap:101 telephone-event/8000\r\n"
        );
        let request = format!(
            concat!(
                "INVITE sip:13800138000@example.com SIP/2.0\r\n",
                "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-sdp-offer\r\n",
                "Max-Forwards: 70\r\n",
                "From: <sip:1001@example.com>;tag=from-tag\r\n",
                "To: <sip:13800138000@example.com>\r\n",
                "Call-ID: invite-sdp-offer@example.com\r\n",
                "CSeq: 1 INVITE\r\n",
                "Content-Type: application/sdp\r\n",
                "Content-Length: {}\r\n",
                "\r\n",
                "{}"
            ),
            body.len(),
            body
        );

        let port_min = get_test_port_min();
        let datagrams =
            handle_datagram(request.as_bytes(), peer(), &edge_state, &edge_config()).await;
        assert_eq!(datagrams.len(), 2);

        let outbound_invite = datagram_text(&datagrams[1]);
        assert!(outbound_invite.contains("c=IN IP4 203.0.113.10\r\n"));
        assert!(outbound_invite.contains(&format!("m=audio {} RTP/AVP 0 8 101\r\n", port_min)));
        assert!(outbound_invite.contains("a=rtpmap:0 PCMU/8000\r\n"));
        assert!(outbound_invite.contains("a=rtpmap:8 PCMA/8000\r\n"));
        assert!(outbound_invite.contains("a=rtpmap:101 telephone-event/8000\r\n"));

        // DashMap get returns Ref which owns the lock
        let transaction = edge_state.inbound_transactions.get("invite-sdp-offer@example.com")
            .expect("transaction should be remembered");
        assert_eq!(
            transaction.caller_rtp,
            Some(RtpEndpoint::new("192.0.2.10", 49170))
        );
        assert_eq!(
            transaction.gateway_relay_rtp,
            Some(RtpEndpoint::new("203.0.113.10", port_min))
        );
        assert_eq!(
            edge_state.media_relay.target_for_port(port_min),
            Some("192.0.2.10:49170".parse().unwrap())
        );
    }

    #[tokio::test]
    async fn invite_with_unsupported_audio_codec_receives_not_acceptable() {
        let edge_state = state_with_default_route();
        let body = concat!(
            "v=0\r\n",
            "o=caller 1 1 IN IP4 192.0.2.10\r\n",
            "s=caller\r\n",
            "c=IN IP4 192.0.2.10\r\n",
            "t=0 0\r\n",
            "m=audio 49170 RTP/AVP 101\r\n",
            "a=rtpmap:101 telephone-event/8000\r\n"
        );
        let request = format!(
            concat!(
                "INVITE sip:13800138000@example.com SIP/2.0\r\n",
                "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-sdp-unsupported\r\n",
                "Max-Forwards: 70\r\n",
                "From: <sip:1001@example.com>;tag=from-tag\r\n",
                "To: <sip:13800138000@example.com>\r\n",
                "Call-ID: invite-sdp-unsupported@example.com\r\n",
                "CSeq: 1 INVITE\r\n",
                "Content-Type: application/sdp\r\n",
                "Content-Length: {}\r\n",
                "\r\n",
                "{}"
            ),
            body.len(),
            body
        );

        let datagrams =
            handle_datagram(request.as_bytes(), peer(), &edge_state, &edge_config()).await;

        assert_eq!(datagrams.len(), 1);
        let response = datagram_text(&datagrams[0]);
        assert!(response.starts_with("SIP/2.0 488 Not Acceptable Here\r\n"));
        assert!(response.contains("X-VOS-RS-Error: missing compatible audio codec in SDP\r\n"));
    }

    #[tokio::test]
    async fn invite_with_exhausted_rtp_ports_receives_service_unavailable() {
        let edge_state = state_with_default_route();
        let edge_config = EdgeConfig {
            advertised_addr: "edge.example.com:5060".to_string(),
            media: MediaConfig::new("203.0.113.10", 31_000, 31_000),
            auth: AuthConfig::disabled(),
            session_expires_gateway: 600,
            session_expires_caller: 1800,
            sbc_allow_rules: Vec::new(),
            sbc_block_rules: Vec::new(),
            sbc_rate_limit_capacity: 100.0,
            sbc_rate_limit_fill_rate: 10.0,
            sbc_max_concurrency: 10,
            tls_cert_path: None,
            tls_key_path: None,
            tls_allow_test_certificate: false,
            tls_ca_path: None,
            tls_insecure_skip_verify: false,
            tls_server_name: None,
        };
        let body = concat!(
            "v=0\r\n",
            "o=caller 1 1 IN IP4 192.0.2.10\r\n",
            "s=caller\r\n",
            "c=IN IP4 192.0.2.10\r\n",
            "t=0 0\r\n",
            "m=audio 49170 RTP/AVP 0\r\n",
            "a=rtpmap:0 PCMU/8000\r\n"
        );
        let first_invite = format!(
            concat!(
                "INVITE sip:13800138000@example.com SIP/2.0\r\n",
                "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-rtp-port-one\r\n",
                "Max-Forwards: 70\r\n",
                "From: <sip:1001@example.com>;tag=from-tag\r\n",
                "To: <sip:13800138000@example.com>\r\n",
                "Call-ID: invite-rtp-port-one@example.com\r\n",
                "CSeq: 1 INVITE\r\n",
                "Content-Type: application/sdp\r\n",
                "Content-Length: {}\r\n",
                "\r\n",
                "{}"
            ),
            body.len(),
            body
        );
        let first_datagrams =
            handle_datagram(first_invite.as_bytes(), peer(), &edge_state, &edge_config).await;
        assert_eq!(first_datagrams.len(), 2);
        assert!(datagram_text(&first_datagrams[1]).contains("m=audio 31000 RTP/AVP 0\r\n"));

        let second_invite = format!(
            concat!(
                "INVITE sip:13800138001@example.com SIP/2.0\r\n",
                "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-rtp-port-two\r\n",
                "Max-Forwards: 70\r\n",
                "From: <sip:1001@example.com>;tag=from-tag\r\n",
                "To: <sip:13800138001@example.com>\r\n",
                "Call-ID: invite-rtp-port-two@example.com\r\n",
                "CSeq: 1 INVITE\r\n",
                "Content-Type: application/sdp\r\n",
                "Content-Length: {}\r\n",
                "\r\n",
                "{}"
            ),
            body.len(),
            body
        );
        let second_datagrams =
            handle_datagram(second_invite.as_bytes(), peer(), &edge_state, &edge_config).await;

        assert_eq!(second_datagrams.len(), 1);
        let response = datagram_text(&second_datagrams[0]);
        assert!(response.starts_with("SIP/2.0 503 Service Unavailable\r\n"));
        assert!(response.contains("X-VOS-RS-Error: RTP port range exhausted: 31000-31000\r\n"));
    }

    #[tokio::test]
    async fn outbound_failure_releases_rtp_port_lease() {
        let edge_state = state_with_default_route();
        let edge_config = EdgeConfig {
            advertised_addr: "edge.example.com:5060".to_string(),
            media: MediaConfig::new("203.0.113.10", 32_000, 32_000),
            auth: AuthConfig::disabled(),
            session_expires_gateway: 600,
            session_expires_caller: 1800,
            sbc_allow_rules: Vec::new(),
            sbc_block_rules: Vec::new(),
            sbc_rate_limit_capacity: 100.0,
            sbc_rate_limit_fill_rate: 10.0,
            sbc_max_concurrency: 10,
            tls_cert_path: None,
            tls_key_path: None,
            tls_allow_test_certificate: false,
            tls_ca_path: None,
            tls_insecure_skip_verify: false,
            tls_server_name: None,
        };
        let body = concat!(
            "v=0\r\n",
            "o=caller 1 1 IN IP4 192.0.2.10\r\n",
            "s=caller\r\n",
            "c=IN IP4 192.0.2.10\r\n",
            "t=0 0\r\n",
            "m=audio 49170 RTP/AVP 0\r\n",
            "a=rtpmap:0 PCMU/8000\r\n"
        );
        let first_invite = format!(
            concat!(
                "INVITE sip:13800138000@example.com SIP/2.0\r\n",
                "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-rtp-release-one\r\n",
                "Max-Forwards: 70\r\n",
                "From: <sip:1001@example.com>;tag=from-tag\r\n",
                "To: <sip:13800138000@example.com>\r\n",
                "Call-ID: invite-rtp-release-one@example.com\r\n",
                "CSeq: 1 INVITE\r\n",
                "Content-Type: application/sdp\r\n",
                "Content-Length: {}\r\n",
                "\r\n",
                "{}"
            ),
            body.len(),
            body
        );
        let first_datagrams =
            handle_datagram(first_invite.as_bytes(), peer(), &edge_state, &edge_config).await;
        assert_eq!(first_datagrams.len(), 2);

        let failure_response = concat!(
            "SIP/2.0 486 Busy Here\r\n",
            "Via: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-vosrs\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:13800138000@example.com>;tag=gw-tag\r\n",
            "Call-ID: invite-rtp-release-one@example.com\r\n",
            "CSeq: 1 INVITE\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        );
        let failure_datagrams = handle_datagram(
            failure_response.as_bytes(),
            "198.51.100.20:5060".parse().unwrap(),
            &edge_state,
            &edge_config,
        )
        .await;
        assert_eq!(failure_datagrams.len(), 1);
        assert!(datagram_text(&failure_datagrams[0]).starts_with("SIP/2.0 486 Busy Here\r\n"));

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let second_invite = format!(
            concat!(
                "INVITE sip:13800138001@example.com SIP/2.0\r\n",
                "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-rtp-release-two\r\n",
                "Max-Forwards: 70\r\n",
                "From: <sip:1001@example.com>;tag=from-tag\r\n",
                "To: <sip:13800138001@example.com>\r\n",
                "Call-ID: invite-rtp-release-two@example.com\r\n",
                "CSeq: 1 INVITE\r\n",
                "Content-Type: application/sdp\r\n",
                "Content-Length: {}\r\n",
                "\r\n",
                "{}"
            ),
            body.len(),
            body
        );
        let second_datagrams =
            handle_datagram(second_invite.as_bytes(), peer(), &edge_state, &edge_config).await;
        assert_eq!(second_datagrams.len(), 2);
        assert!(datagram_text(&second_datagrams[1]).contains("m=audio 32000 RTP/AVP 0\r\n"));
    }

    #[tokio::test]
    async fn invite_without_route_receives_not_found() {
        let edge_state = state_without_routes();
        let request = concat!(
            "INVITE sip:13900139000@example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-4\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:13900139000@example.com>\r\n",
            "Call-ID: invite-no-route@example.com\r\n",
            "CSeq: 1 INVITE\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        );

        let datagrams =
            handle_datagram(request.as_bytes(), peer(), &edge_state, &edge_config()).await;
        assert_eq!(datagrams.len(), 1);

        let response = datagram_text(&datagrams[0]);
        assert!(response.starts_with("SIP/2.0 404 Not Found\r\n"));
        assert!(response.contains("X-VOS-RS-Error: no route for destination: 13900139000\r\n"));
        assert_eq!(edge_state.call_manager.len(), 1);
    }

    #[tokio::test]
    async fn invalid_invite_receives_bad_request() {
        let edge_state = state_with_default_route();
        let request = concat!(
            "INVITE sip:13800138000@example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-3\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:13800138000@example.com>\r\n",
            "CSeq: 1 INVITE\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        );

        let datagrams =
            handle_datagram(request.as_bytes(), peer(), &edge_state, &edge_config()).await;
        assert_eq!(datagrams.len(), 1);

        let response = datagram_text(&datagrams[0]);
        assert!(response.starts_with("SIP/2.0 400 Bad Request\r\n"));
        assert!(response.contains("X-VOS-RS-Error: missing required SIP header: Call-ID\r\n"));
    }

    #[tokio::test]
    async fn forwards_gateway_ringing_response_to_inbound_peer() {
        let edge_state = state_with_default_route();
        let invite = concat!(
            "INVITE sip:13800138000@example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-inbound\r\n",
            "Max-Forwards: 70\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:13800138000@example.com>\r\n",
            "Call-ID: invite-2@example.com\r\n",
            "CSeq: 1 INVITE\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        );
        let _ = handle_datagram(invite.as_bytes(), peer(), &edge_state, &edge_config()).await;

        let gateway_response = concat!(
            "SIP/2.0 180 Ringing\r\n",
            "Via: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-vosrs\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:13800138000@example.com>;tag=gw-tag\r\n",
            "Call-ID: invite-2@example.com\r\n",
            "CSeq: 1 INVITE\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        );

        let datagrams = handle_datagram(
            gateway_response.as_bytes(),
            "198.51.100.20:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        assert_eq!(datagrams.len(), 1);
        assert_eq!(datagrams[0].target, "192.0.2.10:5060");

        let response = datagram_text(&datagrams[0]);
        assert!(response.starts_with("SIP/2.0 180 Ringing\r\n"));
        assert!(response.contains("Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-inbound\r\n"));
        assert!(response.contains("To: <sip:13800138000@example.com>;tag=gw-tag\r\n"));

        let call_guard = &edge_state.call_manager;
        let call = call_guard
            .get(&CallId::new("invite-2@example.com"))
            .expect("call should still be tracked");
        assert_eq!(call.state, CallState::Ringing);
    }

    #[tokio::test]
    async fn forwards_gateway_ok_with_sdp_and_establishes_call() {
        let edge_state = state_with_default_route();
        send_invite(&edge_state, "invite-3@example.com").await;

        let gateway_response = concat!(
            "SIP/2.0 200 OK\r\n",
            "Via: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-vosrs\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:13800138000@example.com>;tag=gw-tag\r\n",
            "Call-ID: invite-3@example.com\r\n",
            "CSeq: 1 INVITE\r\n",
            "Content-Type: application/sdp\r\n",
            "Content-Length: 5\r\n",
            "\r\n",
            "v=0\r\n"
        );

        let datagrams = handle_datagram(
            gateway_response.as_bytes(),
            "198.51.100.20:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        assert_eq!(datagrams.len(), 1);
        assert_eq!(datagrams[0].target, "192.0.2.10:5060");

        let response = datagram_text(&datagrams[0]);
        assert!(response.starts_with("SIP/2.0 200 OK\r\n"));
        assert!(response.contains("Content-Type: application/sdp\r\n"));
        assert!(response.contains("Content-Length: 5\r\n\r\nv=0\r\n"));

        let call_guard = &edge_state.call_manager;
        let call = call_guard
            .get(&CallId::new("invite-3@example.com"))
            .expect("call should still be tracked");
        assert_eq!(call.state, CallState::Established);
        // DashMap get returns Ref which owns the lock
        let transaction = edge_state.inbound_transactions.get("invite-3@example.com")
            .expect("transaction should be remembered");
        assert_eq!(transaction.inbound_from_tag.as_deref(), Some("from-tag"));
        assert_eq!(transaction.inbound_to_tag.as_deref(), Some("gw-tag"));
        assert_eq!(transaction.last_inbound_cseq, Some(1));
    }

    #[tokio::test]
    async fn rewrites_gateway_answer_sdp_for_caller_media_relay() {
        let edge_state = state_with_default_route();
        send_invite(&edge_state, "invite-sdp-answer@example.com").await;
        let body = concat!(
            "v=0\r\n",
            "o=gateway 1 1 IN IP4 198.51.100.20\r\n",
            "s=gateway\r\n",
            "c=IN IP4 198.51.100.20\r\n",
            "t=0 0\r\n",
            "m=audio 49172 RTP/AVP 0\r\n",
            "a=rtpmap:0 PCMU/8000\r\n"
        );
        let gateway_response = format!(
            concat!(
                "SIP/2.0 200 OK\r\n",
                "Via: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-vosrs\r\n",
                "From: <sip:1001@example.com>;tag=from-tag\r\n",
                "To: <sip:13800138000@example.com>;tag=gw-tag\r\n",
                "Call-ID: invite-sdp-answer@example.com\r\n",
                "CSeq: 1 INVITE\r\n",
                "Content-Type: application/sdp\r\n",
                "Content-Length: {}\r\n",
                "\r\n",
                "{}"
            ),
            body.len(),
            body
        );

        let datagrams = handle_datagram(
            gateway_response.as_bytes(),
            "198.51.100.20:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        assert_eq!(datagrams.len(), 1);
        let response = datagram_text(&datagrams[0]);
        let port_min = get_test_port_min();
        assert!(response.contains("c=IN IP4 203.0.113.10\r\n"));
        assert!(response.contains(&format!("m=audio {} RTP/AVP 0\r\n", port_min)));

        // DashMap get returns Ref which owns the lock
        let transaction = edge_state.inbound_transactions.get("invite-sdp-answer@example.com")
            .expect("transaction should be remembered");
        assert_eq!(
            transaction.gateway_rtp,
            Some(RtpEndpoint::new("198.51.100.20", 49172))
        );
        assert_eq!(
            transaction.caller_relay_rtp,
            Some(RtpEndpoint::new("203.0.113.10", port_min))
        );
        assert_eq!(
            edge_state.media_relay.target_for_port(port_min),
            Some("198.51.100.20:49172".parse().unwrap())
        );
    }

    #[tokio::test]
    async fn pairs_rtp_relay_ports_after_sdp_offer_answer() {
        let edge_state = state_with_default_route();
        let offer_body = concat!(
            "v=0\r\n",
            "o=caller 1 1 IN IP4 192.0.2.10\r\n",
            "s=caller\r\n",
            "c=IN IP4 192.0.2.10\r\n",
            "t=0 0\r\n",
            "m=audio 49170 RTP/AVP 0 8\r\n",
            "a=rtpmap:0 PCMU/8000\r\n",
            "a=rtpmap:8 PCMA/8000\r\n"
        );
        let invite = format!(
            concat!(
                "INVITE sip:13800138000@example.com SIP/2.0\r\n",
                "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-sdp-pair\r\n",
                "Max-Forwards: 70\r\n",
                "From: <sip:1001@example.com>;tag=from-tag\r\n",
                "To: <sip:13800138000@example.com>\r\n",
                "Call-ID: invite-sdp-pair@example.com\r\n",
                "CSeq: 1 INVITE\r\n",
                "Content-Type: application/sdp\r\n",
                "Content-Length: {}\r\n",
                "\r\n",
                "{}"
            ),
            offer_body.len(),
            offer_body
        );
        let invite_datagrams =
            handle_datagram(invite.as_bytes(), peer(), &edge_state, &edge_config()).await;
        assert_eq!(invite_datagrams.len(), 2);

        let answer_body = concat!(
            "v=0\r\n",
            "o=gateway 1 1 IN IP4 198.51.100.20\r\n",
            "s=gateway\r\n",
            "c=IN IP4 198.51.100.20\r\n",
            "t=0 0\r\n",
            "m=audio 49172 RTP/AVP 8\r\n",
            "a=rtpmap:8 PCMA/8000\r\n"
        );
        let gateway_response = format!(
            concat!(
                "SIP/2.0 200 OK\r\n",
                "Via: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-vosrs\r\n",
                "From: <sip:1001@example.com>;tag=from-tag\r\n",
                "To: <sip:13800138000@example.com>;tag=gw-tag\r\n",
                "Call-ID: invite-sdp-pair@example.com\r\n",
                "CSeq: 1 INVITE\r\n",
                "Content-Type: application/sdp\r\n",
                "Content-Length: {}\r\n",
                "\r\n",
                "{}"
            ),
            answer_body.len(),
            answer_body
        );
        let answer_datagrams = handle_datagram(
            gateway_response.as_bytes(),
            "198.51.100.20:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;
        assert_eq!(answer_datagrams.len(), 1);

        let response = datagram_text(&answer_datagrams[0]);
        let port_min = get_test_port_min();
        assert!(response.contains(&format!("m=audio {} RTP/AVP 8\r\n", port_min + 2)));

        // DashMap get returns Ref which owns the lock
        let transaction = edge_state.inbound_transactions.get("invite-sdp-pair@example.com")
            .expect("transaction should be remembered");
        assert_eq!(
            transaction.gateway_relay_rtp,
            Some(RtpEndpoint::new("203.0.113.10", port_min))
        );
        assert_eq!(
            transaction.caller_relay_rtp,
            Some(RtpEndpoint::new("203.0.113.10", port_min + 2))
        );
        assert_eq!(
            edge_state.media_relay.peer_port_for(port_min),
            Some(port_min + 2)
        );
        assert_eq!(
            edge_state.media_relay.peer_port_for(port_min + 2),
            Some(port_min)
        );
        assert_eq!(
            edge_state.media_relay.target_for_port(port_min),
            Some("192.0.2.10:49170".parse().unwrap())
        );
        assert_eq!(
            edge_state.media_relay.target_for_port(port_min + 2),
            Some("198.51.100.20:49172".parse().unwrap())
        );
    }

    #[tokio::test]
    async fn forwards_inbound_ack_to_gateway_without_response() {
        let edge_state = state_with_default_route();
        send_invite(&edge_state, "invite-4@example.com").await;
        send_gateway_ok(&edge_state, "invite-4@example.com").await;

        let ack = concat!(
            "ACK sip:13800138000@example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-ack\r\n",
            "Max-Forwards: 70\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:13800138000@example.com>;tag=gw-tag\r\n",
            "Call-ID: invite-4@example.com\r\n",
            "CSeq: 1 ACK\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        );

        let datagrams = handle_datagram(ack.as_bytes(), peer(), &edge_state, &edge_config()).await;

        assert_eq!(datagrams.len(), 1);
        assert_eq!(datagrams[0].target, "gw1.example.com:5060");

        let outbound_ack = datagram_text(&datagrams[0]);
        assert!(outbound_ack
            .starts_with("ACK sip:13800138000@gw1.example.com:5060;transport=udp SIP/2.0\r\n"));
        assert!(outbound_ack.contains("CSeq: 1 ACK\r\n"));
        assert!(outbound_ack.contains("Content-Length: 0\r\n\r\n"));
    }

    #[tokio::test]
    async fn retransmitted_ack_is_forwarded_instead_of_cached() {
        let edge_state = state_with_default_route();
        send_invite(&edge_state, "invite-ack-retry@example.com").await;
        send_gateway_ok(&edge_state, "invite-ack-retry@example.com").await;

        let ack = concat!(
            "ACK sip:13800138000@example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-ack-retry\r\n",
            "Max-Forwards: 70\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:13800138000@example.com>;tag=gw-tag\r\n",
            "Call-ID: invite-ack-retry@example.com\r\n",
            "CSeq: 1 ACK\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        );

        let first = handle_datagram(ack.as_bytes(), peer(), &edge_state, &edge_config()).await;
        let second = handle_datagram(ack.as_bytes(), peer(), &edge_state, &edge_config()).await;

        assert_eq!(first.len(), 1);
        assert_eq!(second.len(), 1);
        assert_eq!(first[0].target, "gw1.example.com:5060");
        assert_eq!(second[0].target, "gw1.example.com:5060");
        assert!(datagram_text(&second[0])
            .starts_with("ACK sip:13800138000@gw1.example.com:5060;transport=udp SIP/2.0\r\n"));
    }

    #[tokio::test]
    async fn inbound_info_gets_ok_and_is_forwarded_to_gateway_with_body() {
        let edge_state = state_with_default_route();
        send_invite(&edge_state, "invite-info@example.com").await;
        send_gateway_ok(&edge_state, "invite-info@example.com").await;
        let body = "Signal=1\r\nDuration=160\r\n";
        let info = format!(
            concat!(
                "INFO sip:13800138000@example.com SIP/2.0\r\n",
                "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-info\r\n",
                "Max-Forwards: 70\r\n",
                "From: <sip:1001@example.com>;tag=from-tag\r\n",
                "To: <sip:13800138000@example.com>;tag=gw-tag\r\n",
                "Call-ID: invite-info@example.com\r\n",
                "CSeq: 2 INFO\r\n",
                "Content-Type: application/dtmf-relay\r\n",
                "Content-Length: {}\r\n",
                "\r\n",
                "{}"
            ),
            body.len(),
            body
        );

        let datagrams = handle_datagram(info.as_bytes(), peer(), &edge_state, &edge_config()).await;

        assert_eq!(datagrams.len(), 2);
        assert_eq!(datagrams[0].target, "192.0.2.10:5060");
        assert_eq!(datagrams[1].target, "gw1.example.com:5060");

        let response = datagram_text(&datagrams[0]);
        assert!(response.starts_with("SIP/2.0 200 OK\r\n"));
        assert!(response.contains("CSeq: 2 INFO\r\n"));

        let outbound_info = datagram_text(&datagrams[1]);
        assert!(outbound_info
            .starts_with("INFO sip:13800138000@gw1.example.com:5060;transport=udp SIP/2.0\r\n"));
        assert!(outbound_info.contains("CSeq: 2 INFO\r\n"));
        assert!(outbound_info.contains("Content-Type: application/dtmf-relay\r\n"));
        assert!(outbound_info.contains("Content-Length: 24\r\n\r\nSignal=1\r\nDuration=160\r\n"));
    }

    #[tokio::test]
    async fn retransmitted_info_replays_ok_without_duplicate_outbound_info() {
        let edge_state = state_with_default_route();
        send_invite(&edge_state, "invite-info-retry@example.com").await;
        send_gateway_ok(&edge_state, "invite-info-retry@example.com").await;
        let body = "Signal=2\r\nDuration=120\r\n";
        let info = format!(
            concat!(
                "INFO sip:13800138000@example.com SIP/2.0\r\n",
                "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-info-retry\r\n",
                "Max-Forwards: 70\r\n",
                "From: <sip:1001@example.com>;tag=from-tag\r\n",
                "To: <sip:13800138000@example.com>;tag=gw-tag\r\n",
                "Call-ID: invite-info-retry@example.com\r\n",
                "CSeq: 2 INFO\r\n",
                "Content-Type: application/dtmf-relay\r\n",
                "Content-Length: {}\r\n",
                "\r\n",
                "{}"
            ),
            body.len(),
            body
        );

        let first = handle_datagram(info.as_bytes(), peer(), &edge_state, &edge_config()).await;
        let second = handle_datagram(info.as_bytes(), peer(), &edge_state, &edge_config()).await;

        assert_eq!(first.len(), 2);
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].target, "192.0.2.10:5060");

        let response = datagram_text(&second[0]);
        assert!(response.starts_with("SIP/2.0 200 OK\r\n"));
        assert!(response.contains("CSeq: 2 INFO\r\n"));
    }

    #[tokio::test]
    async fn inbound_refer_gets_accepted_and_notify_progress() {
        let edge_state = state_with_default_route();
        send_invite(&edge_state, "invite-refer@example.com").await;
        send_gateway_ok(&edge_state, "invite-refer@example.com").await;

        let refer = concat!(
            "REFER sip:13800138000@example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-refer\r\n",
            "Max-Forwards: 70\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:13800138000@example.com>;tag=gw-tag\r\n",
            "Call-ID: invite-refer@example.com\r\n",
            "CSeq: 2 REFER\r\n",
            "Refer-To: <sip:1002@example.com>\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        );

        let datagrams =
            handle_datagram(refer.as_bytes(), peer(), &edge_state, &edge_config()).await;

        assert_eq!(datagrams.len(), 3);
        assert_eq!(datagrams[0].target, peer().to_string());
        assert_eq!(datagrams[1].target, peer().to_string());
        assert_eq!(datagrams[2].target, "gw1.example.com:5060");

        let accepted = datagram_text(&datagrams[0]);
        assert!(accepted.starts_with("SIP/2.0 202 Accepted\r\n"));
        assert!(accepted.contains("CSeq: 2 REFER\r\n"));

        let notify = datagram_text(&datagrams[1]);
        assert!(notify.starts_with("NOTIFY sip:1001@example.com SIP/2.0\r\n"));
        assert!(notify.contains("From: <sip:13800138000@example.com>;tag=gw-tag\r\n"));
        assert!(notify.contains("To: <sip:1001@example.com>;tag=from-tag\r\n"));
        assert!(notify.contains("Call-ID: invite-refer@example.com\r\n"));
        assert!(notify.contains("CSeq: 52 NOTIFY\r\n"));
        assert!(notify.contains("Event: refer\r\n"));
        assert!(notify.contains("Subscription-State: active;expires=60\r\n"));
        assert!(notify.contains("Content-Type: message/sipfrag;version=2.0\r\n"));
        assert!(notify.ends_with("SIP/2.0 100 Trying\r\n"));

        let forwarded = datagram_text(&datagrams[2]);
        assert!(
            forwarded.starts_with("INVITE sip:1002@gw1.example.com:5060;transport=udp SIP/2.0\r\n")
        );
        assert!(forwarded.contains("CSeq: 1 INVITE\r\n"));
    }

    #[tokio::test]
    async fn in_dialog_request_with_wrong_from_tag_receives_481() {
        let edge_state = state_with_default_route();
        send_invite(&edge_state, "invite-bad-from-tag@example.com").await;
        send_gateway_ok(&edge_state, "invite-bad-from-tag@example.com").await;
        let info = concat!(
            "INFO sip:13800138000@example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-bad-from-tag\r\n",
            "Max-Forwards: 70\r\n",
            "From: <sip:1001@example.com>;tag=wrong-tag\r\n",
            "To: <sip:13800138000@example.com>;tag=gw-tag\r\n",
            "Call-ID: invite-bad-from-tag@example.com\r\n",
            "CSeq: 2 INFO\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        );

        let datagrams = handle_datagram(info.as_bytes(), peer(), &edge_state, &edge_config()).await;

        assert_eq!(datagrams.len(), 1);
        assert_eq!(datagrams[0].target, "192.0.2.10:5060");
        let response = datagram_text(&datagrams[0]);
        assert!(response.starts_with("SIP/2.0 481 Call/Transaction Does Not Exist\r\n"));
        assert!(
            response.contains("X-VOS-RS-Error: in-dialog From tag does not match call dialog\r\n")
        );
    }

    #[tokio::test]
    async fn in_dialog_request_with_wrong_to_tag_receives_481() {
        let edge_state = state_with_default_route();
        send_invite(&edge_state, "invite-bad-to-tag@example.com").await;
        send_gateway_ok(&edge_state, "invite-bad-to-tag@example.com").await;
        let info = concat!(
            "INFO sip:13800138000@example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-bad-to-tag\r\n",
            "Max-Forwards: 70\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:13800138000@example.com>;tag=wrong-tag\r\n",
            "Call-ID: invite-bad-to-tag@example.com\r\n",
            "CSeq: 2 INFO\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        );

        let datagrams = handle_datagram(info.as_bytes(), peer(), &edge_state, &edge_config()).await;

        assert_eq!(datagrams.len(), 1);
        assert_eq!(datagrams[0].target, "192.0.2.10:5060");
        let response = datagram_text(&datagrams[0]);
        assert!(response.starts_with("SIP/2.0 481 Call/Transaction Does Not Exist\r\n"));
        assert!(
            response.contains("X-VOS-RS-Error: in-dialog To tag does not match call dialog\r\n")
        );
    }

    #[tokio::test]
    async fn in_dialog_request_with_stale_cseq_receives_server_error_without_forwarding() {
        let edge_state = state_with_default_route();
        send_invite(&edge_state, "invite-stale-cseq@example.com").await;
        send_gateway_ok(&edge_state, "invite-stale-cseq@example.com").await;
        let info = concat!(
            "INFO sip:13800138000@example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-info-before-stale\r\n",
            "Max-Forwards: 70\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:13800138000@example.com>;tag=gw-tag\r\n",
            "Call-ID: invite-stale-cseq@example.com\r\n",
            "CSeq: 2 INFO\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        );
        let first = handle_datagram(info.as_bytes(), peer(), &edge_state, &edge_config()).await;
        assert_eq!(first.len(), 2);

        let stale_bye = concat!(
            "BYE sip:13800138000@example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-stale-bye\r\n",
            "Max-Forwards: 70\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:13800138000@example.com>;tag=gw-tag\r\n",
            "Call-ID: invite-stale-cseq@example.com\r\n",
            "CSeq: 2 BYE\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        );

        let datagrams =
            handle_datagram(stale_bye.as_bytes(), peer(), &edge_state, &edge_config()).await;

        assert_eq!(datagrams.len(), 1);
        assert_eq!(datagrams[0].target, "192.0.2.10:5060");
        let response = datagram_text(&datagrams[0]);
        assert!(response.starts_with("SIP/2.0 500 Server Internal Error\r\n"));
        assert!(response
            .contains("X-VOS-RS-Error: out-of-order in-dialog CSeq: received 2, last 2\r\n"));

        let guard = &edge_state.call_manager;
        let call = guard
            .get(&CallId::new("invite-stale-cseq@example.com"))
            .expect("call should still be tracked");
        assert_eq!(call.state, CallState::Established);
    }

    #[tokio::test]
    async fn inbound_bye_gets_ok_and_is_forwarded_to_gateway() {
        let edge_state = state_with_default_route();
        send_invite(&edge_state, "invite-5@example.com").await;
        send_gateway_ok(&edge_state, "invite-5@example.com").await;

        let bye = concat!(
            "BYE sip:13800138000@example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-bye\r\n",
            "Max-Forwards: 70\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:13800138000@example.com>;tag=gw-tag\r\n",
            "Call-ID: invite-5@example.com\r\n",
            "CSeq: 2 BYE\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        );

        let datagrams = handle_datagram(bye.as_bytes(), peer(), &edge_state, &edge_config()).await;

        assert_eq!(datagrams.len(), 2);
        assert_eq!(datagrams[0].target, "192.0.2.10:5060");
        assert_eq!(datagrams[1].target, "gw1.example.com:5060");

        let response = datagram_text(&datagrams[0]);
        assert!(response.starts_with("SIP/2.0 200 OK\r\n"));
        assert!(response.contains("CSeq: 2 BYE\r\n"));

        let outbound_bye = datagram_text(&datagrams[1]);
        assert!(outbound_bye
            .starts_with("BYE sip:13800138000@gw1.example.com:5060;transport=udp SIP/2.0\r\n"));
        assert!(outbound_bye.contains("CSeq: 2 BYE\r\n"));

        let guard = &edge_state.call_manager;
        let call = guard
            .get(&CallId::new("invite-5@example.com"))
            .expect("call should still be tracked");
        assert_eq!(call.state, CallState::Terminated);
    }

    #[tokio::test]
    async fn gateway_bye_gets_ok_and_is_forwarded_to_caller() {
        let edge_state = state_with_default_route();
        send_invite(&edge_state, "invite-gateway-bye@example.com").await;
        send_gateway_ok(&edge_state, "invite-gateway-bye@example.com").await;

        let bye = concat!(
            "BYE sip:1001@example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 198.51.100.20:5060;branch=z9hG4bK-gw-bye\r\n",
            "Max-Forwards: 70\r\n",
            "From: <sip:13800138000@example.com>;tag=gw-tag\r\n",
            "To: <sip:1001@example.com>;tag=from-tag\r\n",
            "Call-ID: invite-gateway-bye@example.com\r\n",
            "CSeq: 2 BYE\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        );

        let datagrams = handle_datagram(
            bye.as_bytes(),
            "198.51.100.20:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        assert_eq!(datagrams.len(), 2);
        assert_eq!(datagrams[0].target, "198.51.100.20:5060");
        assert_eq!(datagrams[1].target, peer().to_string());

        let response = datagram_text(&datagrams[0]);
        assert!(response.starts_with("SIP/2.0 200 OK\r\n"));
        assert!(response.contains("CSeq: 2 BYE\r\n"));

        let forwarded_bye = datagram_text(&datagrams[1]);
        assert!(forwarded_bye.starts_with("BYE sip:192.0.2.10:5060 SIP/2.0\r\n"));
        assert!(forwarded_bye.contains("From: <sip:13800138000@example.com>;tag=gw-tag\r\n"));
        assert!(forwarded_bye.contains("To: <sip:1001@example.com>;tag=from-tag\r\n"));
        assert!(forwarded_bye.contains("CSeq: 2 BYE\r\n"));

        let guard = &edge_state.call_manager;
        let call = guard
            .get(&CallId::new("invite-gateway-bye@example.com"))
            .expect("call should still be tracked");
        assert_eq!(call.state, CallState::Terminated);
    }

    #[tokio::test]
    async fn gateway_refer_gets_accepted_notify_and_forwarded_to_caller() {
        let edge_state = state_with_default_route();
        send_invite(&edge_state, "invite-gateway-refer@example.com").await;
        send_gateway_ok(&edge_state, "invite-gateway-refer@example.com").await;

        let refer = concat!(
            "REFER sip:1001@example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 198.51.100.20:5060;branch=z9hG4bK-gw-refer\r\n",
            "Max-Forwards: 70\r\n",
            "From: <sip:13800138000@example.com>;tag=gw-tag\r\n",
            "To: <sip:1001@example.com>;tag=from-tag\r\n",
            "Call-ID: invite-gateway-refer@example.com\r\n",
            "CSeq: 2 REFER\r\n",
            "Refer-To: <sip:1003@example.com>\r\n",
            "Referred-By: <sip:13800138000@example.com>\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        );

        let datagrams = handle_datagram(
            refer.as_bytes(),
            "198.51.100.20:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        assert_eq!(datagrams.len(), 3);
        assert_eq!(datagrams[0].target, "198.51.100.20:5060");
        assert_eq!(datagrams[1].target, "198.51.100.20:5060");
        assert_eq!(datagrams[2].target, "gw1.example.com:5060");

        let accepted = datagram_text(&datagrams[0]);
        assert!(accepted.starts_with("SIP/2.0 202 Accepted\r\n"));
        assert!(accepted.contains("CSeq: 2 REFER\r\n"));

        let notify = datagram_text(&datagrams[1]);
        assert!(notify.starts_with("NOTIFY sip:13800138000@example.com SIP/2.0\r\n"));
        assert!(notify.contains("From: <sip:1001@example.com>;tag=from-tag\r\n"));
        assert!(notify.contains("To: <sip:13800138000@example.com>;tag=gw-tag\r\n"));
        assert!(notify.contains("Event: refer\r\n"));
        assert!(notify.ends_with("SIP/2.0 100 Trying\r\n"));

        let forwarded = datagram_text(&datagrams[2]);
        assert!(
            forwarded.starts_with("INVITE sip:1003@gw1.example.com:5060;transport=udp SIP/2.0\r\n")
        );
        assert!(forwarded.contains("CSeq: 1 INVITE\r\n"));
    }

    #[tokio::test]
    async fn retransmitted_bye_replays_ok_without_duplicate_outbound_bye() {
        let edge_state = state_with_default_route();
        send_invite(&edge_state, "invite-bye-retry@example.com").await;
        send_gateway_ok(&edge_state, "invite-bye-retry@example.com").await;

        let bye = concat!(
            "BYE sip:13800138000@example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-bye-retry\r\n",
            "Max-Forwards: 70\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:13800138000@example.com>;tag=gw-tag\r\n",
            "Call-ID: invite-bye-retry@example.com\r\n",
            "CSeq: 2 BYE\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        );

        let first = handle_datagram(bye.as_bytes(), peer(), &edge_state, &edge_config()).await;
        let second = handle_datagram(bye.as_bytes(), peer(), &edge_state, &edge_config()).await;

        assert_eq!(first.len(), 2);
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].target, "192.0.2.10:5060");

        let response = datagram_text(&second[0]);
        assert!(response.starts_with("SIP/2.0 200 OK\r\n"));
        assert!(response.contains("CSeq: 2 BYE\r\n"));
    }

    #[tokio::test]
    async fn flush_completed_cdrs_discards_when_postgres_is_disabled() {
        let edge_state = state_with_default_route();
        send_invite(&edge_state, "invite-7@example.com").await;
        send_gateway_ok(&edge_state, "invite-7@example.com").await;

        let bye = concat!(
            "BYE sip:13800138000@example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-bye\r\n",
            "Max-Forwards: 70\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:13800138000@example.com>;tag=gw-tag\r\n",
            "Call-ID: invite-7@example.com\r\n",
            "CSeq: 2 BYE\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        );

        let _ = handle_datagram(bye.as_bytes(), peer(), &edge_state, &edge_config()).await;
        assert_eq!(
            edge_state.call_manager
                .completed_cdrs()
                .len(),
            1
        );

        flush_completed_cdrs(&CdrSinks::default(), &edge_state)
            .await
            .unwrap();

        assert!(edge_state.call_manager
            .completed_cdrs()
            .is_empty());
    }

    #[tokio::test]
    async fn inbound_cancel_gets_ok_and_is_forwarded_to_gateway() {
        let edge_state = state_with_default_route();
        send_invite(&edge_state, "invite-6@example.com").await;

        let cancel = concat!(
            "CANCEL sip:13800138000@example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-cancel\r\n",
            "Max-Forwards: 70\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:13800138000@example.com>\r\n",
            "Call-ID: invite-6@example.com\r\n",
            "CSeq: 1 CANCEL\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        );

        let datagrams =
            handle_datagram(cancel.as_bytes(), peer(), &edge_state, &edge_config()).await;

        assert_eq!(datagrams.len(), 2);
        assert_eq!(datagrams[0].target, "192.0.2.10:5060");
        assert_eq!(datagrams[1].target, "gw1.example.com:5060");

        let response = datagram_text(&datagrams[0]);
        assert!(response.starts_with("SIP/2.0 200 OK\r\n"));
        assert!(response.contains("CSeq: 1 CANCEL\r\n"));

        let outbound_cancel = datagram_text(&datagrams[1]);
        assert!(outbound_cancel
            .starts_with("CANCEL sip:13800138000@gw1.example.com:5060;transport=udp SIP/2.0\r\n"));
        assert!(outbound_cancel.contains("CSeq: 1 CANCEL\r\n"));

        let guard = &edge_state.call_manager;
        let call = guard
            .get(&CallId::new("invite-6@example.com"))
            .expect("call should still be tracked");
        assert_eq!(call.state, CallState::Terminated);
    }

    fn state_with_default_route() -> EdgeState {
        EdgeState::new(CallManager::new(RouteTable::new(vec![Route::new(
            "default",
            "",
            100,
            RouteTarget::new("gw1", "gw1.example.com", Some(5060)),
        )])))
    }

    fn state_with_default_route_and_config(config: &EdgeConfig) -> EdgeState {
        EdgeState::with_config(
            CallManager::new(RouteTable::new(vec![Route::new(
                "default",
                "",
                100,
                RouteTarget::new("gw1", "gw1.example.com", Some(5060)),
            )])),
            config,
        )
    }

    fn state_with_gateway_uri(uri: &str) -> EdgeState {
        let parsed = SipUri::from_str(uri).unwrap();
        EdgeState::new(CallManager::new(RouteTable::new(vec![Route::new(
            "default",
            "",
            100,
            RouteTarget::new("gw1", parsed.host, parsed.port),
        )])))
    }

    fn state_without_routes() -> EdgeState {
        EdgeState::new(CallManager::new(RouteTable::default()))
    }

    use std::sync::atomic::{AtomicU16, Ordering};
    static PORT_COUNTER: AtomicU16 = AtomicU16::new(0);

    thread_local! {
        static THREAD_PORT_OFFSET: u16 = PORT_COUNTER.fetch_add(10, Ordering::Relaxed);
    }

    fn get_thread_ports() -> (u16, u16) {
        let offset = THREAD_PORT_OFFSET.with(|o| *o);
        let port_min = 40_000 + offset;
        let port_max = port_min + 4;
        (port_min, port_max)
    }

    fn get_test_port_min() -> u16 {
        get_thread_ports().0
    }

    fn edge_config() -> EdgeConfig {
        let (port_min, port_max) = get_thread_ports();
        EdgeConfig {
            advertised_addr: "edge.example.com:5060".to_string(),
            media: MediaConfig::new("203.0.113.10", port_min, port_max),
            auth: AuthConfig::disabled(),
            session_expires_gateway: 600,
            session_expires_caller: 1800,
            sbc_allow_rules: std::env::var("VOS_RS_SBC_ALLOW")
                .unwrap_or_default()
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
            sbc_block_rules: std::env::var("VOS_RS_SBC_BLOCK")
                .unwrap_or_default()
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
            sbc_rate_limit_capacity: std::env::var("VOS_RS_SBC_LIMIT_CAPACITY")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(100.0),
            sbc_rate_limit_fill_rate: std::env::var("VOS_RS_SBC_LIMIT_FILL_RATE")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(10.0),
            sbc_max_concurrency: std::env::var("VOS_RS_SBC_MAX_CONCURRENCY")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(10),
            tls_cert_path: None,
            tls_key_path: None,
            tls_allow_test_certificate: false,
            tls_ca_path: None,
            tls_insecure_skip_verify: false,
            tls_server_name: None,
        }
    }

    fn edge_config_with_auth() -> EdgeConfig {
        let (port_min, port_max) = get_thread_ports();
        EdgeConfig {
            advertised_addr: "edge.example.com:5060".to_string(),
            media: MediaConfig::new("203.0.113.10", port_min, port_max),
            auth: AuthConfig::new(
                "vos-rs",
                "test-nonce",
                HashMap::from([("1001".to_string(), "secret".to_string())]),
            ),
            session_expires_gateway: 600,
            session_expires_caller: 1800,
            sbc_allow_rules: Vec::new(),
            sbc_block_rules: Vec::new(),
            sbc_rate_limit_capacity: 100.0,
            sbc_rate_limit_fill_rate: 10.0,
            sbc_max_concurrency: 10,
            tls_cert_path: None,
            tls_key_path: None,
            tls_allow_test_certificate: false,
            tls_ca_path: None,
            tls_insecure_skip_verify: false,
            tls_server_name: None,
        }
    }

    fn peer() -> SocketAddr {
        "192.0.2.10:5060".parse().unwrap()
    }

    fn datagram_text(datagram: &PendingDatagram) -> String {
        String::from_utf8(datagram.bytes.clone()).expect("datagram should be UTF-8")
    }

    async fn send_invite(edge_state: &EdgeState, call_id: &str) {
        let invite = format!(
            concat!(
                "INVITE sip:13800138000@example.com SIP/2.0\r\n",
                "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-{call_id}\r\n",
                "Max-Forwards: 70\r\n",
                "From: <sip:1001@example.com>;tag=from-tag\r\n",
                "To: <sip:13800138000@example.com>\r\n",
                "Call-ID: {call_id}\r\n",
                "CSeq: 1 INVITE\r\n",
                "Content-Length: 0\r\n",
                "\r\n"
            ),
            call_id = call_id
        );

        let datagrams =
            handle_datagram(invite.as_bytes(), peer(), edge_state, &edge_config()).await;
        assert_eq!(datagrams.len(), 2);
    }

    async fn register_contact(edge_state: &EdgeState, user: &str, host: &str, port: u16) {
        let register = format!(
            concat!(
                "REGISTER sip:example.com SIP/2.0\r\n",
                "Via: SIP/2.0/UDP {host}:{port};branch=z9hG4bK-reg-{user}\r\n",
                "From: <sip:{user}@example.com>;tag=from-tag\r\n",
                "To: <sip:{user}@example.com>\r\n",
                "Call-ID: reg-{user}@example.com\r\n",
                "CSeq: 1 REGISTER\r\n",
                "Contact: <sip:{user}@{host}:{port};transport=udp>;expires=120\r\n",
                "Content-Length: 0\r\n",
                "\r\n"
            ),
            user = user,
            host = host,
            port = port
        );

        let peer = format!("{host}:{port}").parse().unwrap();
        let datagrams =
            handle_datagram(register.as_bytes(), peer, edge_state, &edge_config()).await;
        assert_eq!(datagrams.len(), 1);
        assert!(datagram_text(&datagrams[0]).starts_with("SIP/2.0 200 OK\r\n"));
    }

    async fn send_gateway_ok(edge_state: &EdgeState, call_id: &str) {
        let gateway_response = format!(
            concat!(
                "SIP/2.0 200 OK\r\n",
                "Via: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-vosrs\r\n",
                "From: <sip:1001@example.com>;tag=from-tag\r\n",
                "To: <sip:13800138000@example.com>;tag=gw-tag\r\n",
                "Call-ID: {call_id}\r\n",
                "CSeq: 1 INVITE\r\n",
                "Content-Length: 0\r\n",
                "\r\n"
            ),
            call_id = call_id
        );

        let datagrams = handle_datagram(
            gateway_response.as_bytes(),
            "198.51.100.20:5060".parse().unwrap(),
            edge_state,
            &edge_config(),
        )
        .await;
        assert_eq!(datagrams.len(), 1);
    }

    #[tokio::test]
    async fn test_gateway_failover_on_503() {
        let routes = RouteTable::new(vec![
            Route::new(
                "primary",
                "",
                200,
                RouteTarget::new("gw1", "gw1.example.com", Some(5060)),
            ),
            Route::new(
                "backup",
                "",
                100,
                RouteTarget::new("gw2", "gw2.example.com", Some(5060)),
            ),
        ]);
        let edge_state = EdgeState::new(CallManager::new(routes));
        let call_id = "failover-test@example.com";

        let body = concat!(
            "v=0\r\n",
            "o=caller 1 1 IN IP4 192.0.2.10\r\n",
            "s=caller\r\n",
            "c=IN IP4 192.0.2.10\r\n",
            "t=0 0\r\n",
            "m=audio 49170 RTP/AVP 0\r\n",
            "a=rtpmap:0 PCMU/8000\r\n"
        );
        let invite = format!(
            concat!(
                "INVITE sip:13800138000@example.com SIP/2.0\r\n",
                "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-failover-invite\r\n",
                "Max-Forwards: 70\r\n",
                "From: <sip:1001@example.com>;tag=from-tag\r\n",
                "To: <sip:13800138000@example.com>\r\n",
                "Call-ID: {call_id}\r\n",
                "CSeq: 1 INVITE\r\n",
                "Content-Type: application/sdp\r\n",
                "Content-Length: {}\r\n",
                "\r\n",
                "{}"
            ),
            body.len(),
            body,
            call_id = call_id
        );

        let datagrams =
            handle_datagram(invite.as_bytes(), peer(), &edge_state, &edge_config()).await;
        assert_eq!(datagrams.len(), 2);
        assert!(datagram_text(&datagrams[0]).starts_with("SIP/2.0 100 Trying\r\n"));
        assert_eq!(datagrams[1].target, "gw1.example.com:5060");
        assert!(datagram_text(&datagrams[1])
            .starts_with("INVITE sip:13800138000@gw1.example.com:5060;transport=udp SIP/2.0\r\n"));

        let failure_response = format!(
            concat!(
                "SIP/2.0 503 Service Unavailable\r\n",
                "Via: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-vosrs\r\n",
                "From: <sip:1001@example.com>;tag=from-tag\r\n",
                "To: <sip:13800138000@example.com>;tag=gw1-tag\r\n",
                "Call-ID: {call_id}\r\n",
                "CSeq: 1 INVITE\r\n",
                "Content-Length: 0\r\n",
                "\r\n"
            ),
            call_id = call_id
        );

        let failover_datagrams = handle_datagram(
            failure_response.as_bytes(),
            "198.51.100.20:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        assert_eq!(failover_datagrams.len(), 1);
        assert_eq!(failover_datagrams[0].target, "gw2.example.com:5060");
        assert!(datagram_text(&failover_datagrams[0])
            .starts_with("INVITE sip:13800138000@gw2.example.com:5060;transport=udp SIP/2.0\r\n"));

        let call_guard = &edge_state.call_manager;
        let call = call_guard.get(&CallId::new(call_id)).unwrap();
        assert_eq!(call.state, CallState::Routing);
        assert_eq!(call.current_candidate_index, 1);
        assert_eq!(call.outbound_history.len(), 1);
        assert_eq!(
            call.outbound_history[0].remote_uri.to_string(),
            "sip:13800138000@gw1.example.com:5060;transport=udp"
        );
        assert_eq!(
            call.outbound.as_ref().unwrap().remote_uri.to_string(),
            "sip:13800138000@gw2.example.com:5060;transport=udp"
        );
    }

    #[tokio::test]
    async fn test_gateway_302_redirect_recursion() {
        let routes = RouteTable::new(vec![Route::new(
            "primary",
            "",
            200,
            RouteTarget::new("gw1", "gw1.example.com", Some(5060)),
        )]);
        let edge_state = EdgeState::new(CallManager::new(routes));
        let call_id = "redirect-test@example.com";

        let body = concat!(
            "v=0\r\n",
            "o=caller 1 1 IN IP4 192.0.2.10\r\n",
            "s=caller\r\n",
            "c=IN IP4 192.0.2.10\r\n",
            "t=0 0\r\n",
            "m=audio 49170 RTP/AVP 0\r\n",
            "a=rtpmap:0 PCMU/8000\r\n"
        );
        let invite = format!(
            concat!(
                "INVITE sip:13800138000@example.com SIP/2.0\r\n",
                "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-redirect-invite\r\n",
                "Max-Forwards: 70\r\n",
                "From: <sip:1001@example.com>;tag=from-tag\r\n",
                "To: <sip:13800138000@example.com>\r\n",
                "Call-ID: {call_id}\r\n",
                "CSeq: 1 INVITE\r\n",
                "Content-Type: application/sdp\r\n",
                "Content-Length: {}\r\n",
                "\r\n",
                "{}"
            ),
            body.len(),
            body,
            call_id = call_id
        );

        let datagrams =
            handle_datagram(invite.as_bytes(), peer(), &edge_state, &edge_config()).await;
        assert_eq!(datagrams.len(), 2);
        assert_eq!(datagrams[1].target, "gw1.example.com:5060");

        // GW1 responds with 302 Moved Temporarily directing to sip:13800138000@redirect-target.example.com:5060
        let redirect_response = format!(
            concat!(
                "SIP/2.0 302 Moved Temporarily\r\n",
                "Via: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-vosrs\r\n",
                "From: <sip:1001@example.com>;tag=from-tag\r\n",
                "To: <sip:13800138000@example.com>;tag=gw1-tag\r\n",
                "Call-ID: {call_id}\r\n",
                "CSeq: 1 INVITE\r\n",
                "Contact: <sip:13800138000@redirect-target.example.com:5060>\r\n",
                "Content-Length: 0\r\n",
                "\r\n"
            ),
            call_id = call_id
        );

        let redirect_datagrams = handle_datagram(
            redirect_response.as_bytes(),
            "198.51.100.20:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // Verify that 302 response was intercepted and resulted in redirect INVITE to redirect-target.example.com:5060
        assert_eq!(redirect_datagrams.len(), 1);
        assert_eq!(
            redirect_datagrams[0].target,
            "redirect-target.example.com:5060"
        );
        assert!(datagram_text(&redirect_datagrams[0])
            .starts_with("INVITE sip:13800138000@redirect-target.example.com:5060 SIP/2.0\r\n"));

        let call_guard = &edge_state.call_manager;
        let call = call_guard.get(&CallId::new(call_id)).unwrap();
        assert_eq!(call.state, CallState::Routing);
        assert_eq!(call.current_candidate_index, 1);
        assert_eq!(
            call.outbound.as_ref().unwrap().remote_uri.to_string(),
            "sip:13800138000@redirect-target.example.com:5060"
        );
    }

    #[tokio::test]
    async fn test_out_of_dialog_message_routing_to_registered_contact() {
        let edge_state = state_with_default_route();

        // 1. Register contact 1001
        let register = "REGISTER sip:example.com SIP/2.0\r\n\
             Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-reg\r\n\
             From: <sip:1001@example.com>;tag=from-tag\r\n\
             To: <sip:1001@example.com>\r\n\
             Call-ID: reg-message-01\r\n\
             CSeq: 1 REGISTER\r\n\
             Contact: <sip:1001@192.0.2.10:5070;transport=udp>;expires=60\r\n\
             Content-Length: 0\r\n\r\n";
        let _ = handle_datagram(
            register.as_bytes(),
            "192.0.2.10:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // 2. Send MESSAGE from 1002 to 1001
        let call_id = "msg-001";
        let message_req = format!(
            "MESSAGE sip:1001@example.com SIP/2.0\r\n\
             Via: SIP/2.0/UDP 192.0.2.20:5060;branch=z9hG4bK-msg-1\r\n\
             From: <sip:1002@example.com>;tag=from-tag\r\n\
             To: <sip:1001@example.com>\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 1 MESSAGE\r\n\
             Content-Type: text/plain\r\n\
             Content-Length: 5\r\n\r\n\
             hello"
        );

        let datagrams = handle_datagram(
            message_req.as_bytes(),
            "192.0.2.20:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // Verify MESSAGE is forwarded to 1001's registered contact
        assert_eq!(datagrams.len(), 1);
        assert_eq!(datagrams[0].target, "192.0.2.10:5060");
        let forwarded_msg = datagram_text(&datagrams[0]);
        assert!(
            forwarded_msg.starts_with("MESSAGE sip:1001@192.0.2.10:5070;transport=udp SIP/2.0\r\n")
        );
        assert!(forwarded_msg.contains("\r\n\r\nhello"));

        // Check transaction registered
        {
            assert!(edge_state.inbound_transactions.contains_key(call_id));
        }

        // 3. Receive 200 OK from 1001
        let ok_200 = format!(
            "SIP/2.0 200 OK\r\n\
             Via: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-msg-1\r\n\
             Via: SIP/2.0/UDP 192.0.2.20:5060;branch=z9hG4bK-msg-1\r\n\
             From: <sip:1002@example.com>;tag=from-tag\r\n\
             To: <sip:1001@example.com>;tag=to-tag\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 1 MESSAGE\r\n\
             Content-Length: 0\r\n\r\n"
        );

        let response_datagrams = handle_datagram(
            ok_200.as_bytes(),
            "192.0.2.10:5070".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // Verify 200 OK forwarded back to sender 1002
        assert_eq!(response_datagrams.len(), 1);
        assert_eq!(response_datagrams[0].target, "192.0.2.20:5060");
        let forwarded_ok = datagram_text(&response_datagrams[0]);
        assert!(forwarded_ok.starts_with("SIP/2.0 200 OK\r\n"));

        // Check transaction cleaned up
        {
            assert!(!edge_state.inbound_transactions.contains_key(call_id));
        }
    }

    #[tokio::test]
    async fn test_out_of_dialog_message_routing_to_gateway() {
        let edge_state = state_with_default_route();

        // Send MESSAGE targeting an unregistered destination (so it falls back to the default route gateway)
        let call_id = "msg-gw-01";
        let message_req = format!(
            "MESSAGE sip:13800138000@example.com SIP/2.0\r\n\
             Via: SIP/2.0/UDP 192.0.2.20:5060;branch=z9hG4bK-msg-gw\r\n\
             From: <sip:1002@example.com>;tag=from-tag\r\n\
             To: <sip:13800138000@example.com>\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 1 MESSAGE\r\n\
             Content-Type: text/plain\r\n\
             Content-Length: 5\r\n\r\n\
             hello"
        );

        let datagrams = handle_datagram(
            message_req.as_bytes(),
            "192.0.2.20:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // Verify MESSAGE is forwarded to default gateway (gw1.example.com:5060)
        assert_eq!(datagrams.len(), 1);
        assert_eq!(datagrams[0].target, "gw1.example.com:5060");
        let forwarded_msg = datagram_text(&datagrams[0]);
        assert!(forwarded_msg
            .starts_with("MESSAGE sip:13800138000@gw1.example.com:5060;transport=udp SIP/2.0\r\n"));
        assert!(forwarded_msg.contains("\r\n\r\nhello"));
    }

    #[tokio::test]
    async fn test_nat_traversal_registered_contact_override() {
        let edge_state = state_with_default_route();

        // 1. Register contact 1001 with private contact but public received_from socket
        let register = "REGISTER sip:example.com SIP/2.0\r\n\
             Via: SIP/2.0/UDP 192.0.2.10:5070;branch=z9hG4bK-regnat\r\n\
             From: <sip:1001@example.com>;tag=from-tag\r\n\
             To: <sip:1001@example.com>\r\n\
             Call-ID: reg-nat-01\r\n\
             CSeq: 1 REGISTER\r\n\
             Contact: <sip:1001@192.168.1.100:5060;transport=udp>;expires=60\r\n\
             Content-Length: 0\r\n\r\n";
        let _ = handle_datagram(
            register.as_bytes(),
            "192.0.2.10:5070".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // 2. Receive an inbound INVITE to 1001
        let call_id = "invite-nat-01";
        let body = sdp_body();
        let invite = format!(
            concat!(
                "INVITE sip:1001@example.com SIP/2.0\r\n",
                "Via: SIP/2.0/UDP 192.0.2.20:5060;branch=z9hG4bK-invite-nat\r\n",
                "Max-Forwards: 70\r\n",
                "From: <sip:1002@example.com>;tag=caller-tag\r\n",
                "To: <sip:1001@example.com>\r\n",
                "Call-ID: {call_id}\r\n",
                "CSeq: 1 INVITE\r\n",
                "Content-Type: application/sdp\r\n",
                "Content-Length: {}\r\n",
                "\r\n",
                "{}"
            ),
            body.len(),
            body,
            call_id = call_id
        );

        let datagrams = handle_datagram(
            invite.as_bytes(),
            "192.0.2.20:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // Verify INVITE is forwarded to the public NAT address of client 1001, NOT the private Contact IP!
        assert_eq!(datagrams.len(), 2);
        assert_eq!(datagrams[1].target, "192.0.2.10:5070");
        let forwarded_msg = datagram_text(&datagrams[1]);
        assert!(forwarded_msg
            .starts_with("INVITE sip:1001@192.168.1.100:5060;transport=udp SIP/2.0\r\n"));
    }

    #[tokio::test]
    async fn test_nat_traversal_in_dialog_callee_override() {
        let edge_state = state_with_default_route();

        // 1. Register contact 1001 with private Contact but public received_from socket
        let register = "REGISTER sip:example.com SIP/2.0\r\n\
             Via: SIP/2.0/UDP 198.51.100.20:5070;branch=z9hG4bK-regnat\r\n\
             From: <sip:1001@example.com>;tag=from-tag\r\n\
             To: <sip:1001@example.com>\r\n\
             Call-ID: reg-nat-01\r\n\
             CSeq: 1 REGISTER\r\n\
             Contact: <sip:1001@192.168.100.200:5060;transport=udp>;expires=60\r\n\
             Content-Length: 0\r\n\r\n";
        let _ = handle_datagram(
            register.as_bytes(),
            "198.51.100.20:5070".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // 2. Establish initial call: inbound INVITE from caller 1002 to registered contact 1001
        let call_id = "nat-indialog-01";
        let body = sdp_body();
        let invite = format!(
            concat!(
                "INVITE sip:1001@example.com SIP/2.0\r\n",
                "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-caller-1\r\n",
                "Max-Forwards: 70\r\n",
                "From: <sip:1002@example.com>;tag=caller-tag\r\n",
                "To: <sip:1001@example.com>\r\n",
                "Call-ID: {call_id}\r\n",
                "CSeq: 1 INVITE\r\n",
                "Content-Type: application/sdp\r\n",
                "Content-Length: {}\r\n",
                "\r\n",
                "{}"
            ),
            body.len(),
            body,
            call_id = call_id
        );

        let datagrams = handle_datagram(
            invite.as_bytes(),
            "192.0.2.10:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // Verify INVITE is forwarded to callee 1001 at public NAT address "198.51.100.20:5070"
        assert_eq!(datagrams.len(), 2);
        assert_eq!(datagrams[1].target, "198.51.100.20:5070");

        // 3. Callee 1001 responds 200 OK from public NAT address 198.51.100.20:5070
        let ok_body = sdp_body();
        let ok_200 = format!(
            concat!(
                "SIP/2.0 200 OK\r\n",
                "Via: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-vosrs\r\n",
                "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-caller-1\r\n",
                "From: <sip:1002@example.com>;tag=caller-tag\r\n",
                "To: <sip:1001@example.com>;tag=callee-tag\r\n",
                "Call-ID: {call_id}\r\n",
                "CSeq: 1 INVITE\r\n",
                "Contact: <sip:1001@192.168.100.200:5060>\r\n",
                "Content-Type: application/sdp\r\n",
                "Content-Length: {}\r\n",
                "\r\n",
                "{}"
            ),
            ok_body.len(),
            ok_body,
            call_id = call_id
        );

        let _ = handle_datagram(
            ok_200.as_bytes(),
            "198.51.100.20:5070".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // Verify outbound_peer NAT target and callee_behind_nat flag are registered
        {
            let tx = edge_state.inbound_transactions.get(call_id).unwrap();
            assert_eq!(tx.outbound_peer.as_deref(), Some("198.51.100.20:5070"));
            assert!(tx.callee_behind_nat);
        }

        // 4. Caller sends BYE to callee
        let bye = format!(
            "BYE sip:1001@192.168.100.200:5060 SIP/2.0\r\n\
             Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-caller-2\r\n\
             From: <sip:1002@example.com>;tag=caller-tag\r\n\
             To: <sip:1001@example.com>;tag=callee-tag\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 2 BYE\r\n\
             Content-Length: 0\r\n\r\n"
        );

        let bye_datagrams = handle_datagram(
            bye.as_bytes(),
            "192.0.2.10:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // Verify BYE is routed to the public source socket address of the callee (198.51.100.20:5070), NOT the private Contact IP!
        assert_eq!(bye_datagrams.len(), 2);
        assert_eq!(bye_datagrams[1].target, "198.51.100.20:5070");
        let forwarded_bye = datagram_text(&bye_datagrams[1]);
        assert!(forwarded_bye.starts_with("BYE sip:1001@192.168.100.200:5060 SIP/2.0\r\n"));
    }

    #[tokio::test]
    async fn test_nat_keepalive_background_loop() {
        let edge_state = Arc::new(state_with_default_route());
        let socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        edge_state.set_socket(Arc::clone(&socket));

        let local_addr = socket.local_addr().unwrap();

        // 1. Register a contact pointing to local receiver port so we can capture the keepalive datagram
        let register = format!(
            "REGISTER sip:example.com SIP/2.0\r\n\
             Via: SIP/2.0/UDP {addr};branch=z9hG4bK-regka\r\n\
             From: <sip:1001@example.com>;tag=from-tag\r\n\
             To: <sip:1001@example.com>\r\n\
             Call-ID: reg-ka-01\r\n\
             CSeq: 1 REGISTER\r\n\
             Contact: <sip:1001@192.168.1.100:5060;transport=udp>;expires=60\r\n\
             Content-Length: 0\r\n\r\n",
            addr = local_addr
        );
        let _ = handle_datagram(register.as_bytes(), local_addr, &edge_state, &edge_config()).await;

        // Discard the 200 OK registration response from the socket buffer
        let mut resp_buf = [0u8; 1000];
        let (resp_size, _) =
            tokio::time::timeout(Duration::from_millis(500), socket.recv_from(&mut resp_buf))
                .await
                .expect("timeout waiting for 200 OK registration response")
                .unwrap();
        assert!(std::str::from_utf8(&resp_buf[..resp_size])
            .unwrap()
            .starts_with("SIP/2.0 200 OK\r\n"));

        // 2. Start the NAT keepalive loop
        spawn_nat_keepalive_loop(Arc::clone(&edge_state), Arc::clone(&socket));

        // 3. Receive the NAT keepalive packet
        let mut buffer = [0u8; 100];
        let (size, src) =
            tokio::time::timeout(Duration::from_millis(500), socket.recv_from(&mut buffer))
                .await
                .expect("timeout waiting for keepalive probe")
                .unwrap();

        // Verify the keepalive probe matches single CRLF "\r\n"
        assert_eq!(&buffer[..size], b"\r\n");
        assert_eq!(src, local_addr);
    }

    #[tokio::test]
    async fn test_websocket_transport() {
        use futures::{SinkExt, StreamExt};
        use tokio_tungstenite::connect_async;
        use tokio_tungstenite::tungstenite::Message as WsMessage;

        let edge_state = Arc::new(state_with_default_route());

        // Start WS listener on random port
        let ws_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let ws_addr = ws_listener.local_addr().unwrap();

        let edge_state_clone = Arc::clone(&edge_state);
        tokio::spawn(async move {
            let (stream, peer) = ws_listener.accept().await.unwrap();
            let ws_stream = tokio_tungstenite::accept_async(stream).await.unwrap();
            let (tx, rx) = tokio::sync::mpsc::channel(100);
            edge_state_clone.register_tcp_connection(peer, tx.clone());

            let on_msg_state = Arc::clone(&edge_state_clone);
            handle_ws_connection(
                ws_stream,
                peer,
                tx,
                rx,
                move |msg_bytes: Vec<u8>,
                      peer_addr: SocketAddr,
                      connection_tx: tokio::sync::mpsc::Sender<Vec<u8>>| {
                    let state = Arc::clone(&on_msg_state);
                    async move {
                        let datagrams =
                            handle_datagram(&msg_bytes, peer_addr, &state, &edge_config()).await;
                        for d in datagrams {
                            let _ = connection_tx.send(d.bytes).await;
                        }
                    }
                },
            )
            .await;
        });

        // Connect client
        let (mut client_ws, _) = connect_async(format!("ws://{}", ws_addr)).await.unwrap();

        // Send REGISTER request over WS
        let register = concat!(
            "REGISTER sip:example.com SIP/2.0\r\n",
            "Via: SIP/2.0/WS 127.0.0.1:5062;branch=z9hG4bK-ws-reg\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:1001@example.com>\r\n",
            "Call-ID: ws-reg-001\r\n",
            "CSeq: 1 REGISTER\r\n",
            "Contact: <sip:1001@127.0.0.1:5062;transport=ws>;expires=60\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        );

        client_ws
            .send(WsMessage::Text(register.to_string()))
            .await
            .unwrap();

        // Receive response over WS
        let msg = tokio::time::timeout(Duration::from_millis(1000), client_ws.next())
            .await
            .expect("timeout waiting for WS response")
            .unwrap()
            .unwrap();

        let resp_text = match msg {
            WsMessage::Text(t) => t,
            _ => panic!("expected text frame"),
        };

        assert!(resp_text.starts_with("SIP/2.0 200 OK\r\n"));
        assert!(resp_text.contains("Call-ID: ws-reg-001\r\n"));
    }

    #[tokio::test]
    async fn test_media_hold_renegotiation() {
        let edge_state = state_with_default_route();

        // Establish media targets
        let ep = edge_state
            .media_relay
            .allocate_endpoint(&edge_config().media)
            .unwrap();

        // Register 0.0.0.0 (hold target)
        let hold_target: SocketAddr = "0.0.0.0:0".parse().unwrap();
        edge_state.media_relay.set_target_addr(ep.port, hold_target);

        // Verify that target_for_port returns 0.0.0.0
        assert_eq!(
            edge_state.media_relay.target_for_port(ep.port),
            Some(hold_target)
        );

        // Now simulate sending a media packet to target.
        let socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        edge_state.set_socket(Arc::clone(&socket));

        let raw_sdp = concat!(
            "v=0\r\n",
            "c=IN IP4 0.0.0.0\r\n",
            "m=audio 0 RTP/AVP 0 8\r\n",
            "a=sendonly\r\n"
        );
        let parsed_endpoint = media::parse_sdp_rtp_endpoint(raw_sdp.as_bytes()).unwrap();
        assert_eq!(parsed_endpoint.address, "0.0.0.0");
        assert_eq!(parsed_endpoint.port, 0);

        let rewritten =
            media::rewrite_sdp_body(raw_sdp.as_bytes(), RtpEndpoint::new("127.0.0.1", 40000))
                .unwrap();
        let rewritten_str = std::str::from_utf8(&rewritten).unwrap();
        assert!(rewritten_str.contains("a=sendonly\r\n"));
        assert!(rewritten_str.contains("c=IN IP4 127.0.0.1\r\n"));
        assert!(rewritten_str.contains("m=audio 40000 RTP/AVP 0 8\r\n"));
    }

    #[tokio::test]
    async fn test_prack_header_validation() {
        let edge_state = state_with_default_route();

        // 1. Establish transaction
        let call_id = "prack-val-001";
        let body = sdp_body();
        let invite = format!(
            concat!(
                "INVITE sip:13800138000@example.com SIP/2.0\r\n",
                "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-prval\r\n",
                "Max-Forwards: 70\r\n",
                "From: <sip:1001@example.com>;tag=from-tag\r\n",
                "To: <sip:13800138000@example.com>\r\n",
                "Call-ID: {call_id}\r\n",
                "CSeq: 1 INVITE\r\n",
                "Contact: <sip:1001@192.0.2.10>\r\n",
                "Content-Type: application/sdp\r\n",
                "Content-Length: {}\r\n",
                "\r\n",
                "{}"
            ),
            body.len(),
            body,
            call_id = call_id
        );

        let _ = handle_datagram(invite.as_bytes(), peer(), &edge_state, &edge_config()).await;

        // Gateway responds 180 Ringing with 100rel
        let ringing = format!(
            "SIP/2.0 180 Ringing\r\n\
             Via: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-vosrs\r\n\
             Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-prval\r\n\
             From: <sip:1001@example.com>;tag=from-tag\r\n\
             To: <sip:13800138000@example.com>;tag=gw-tag\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 1 INVITE\r\n\
             Require: 100rel\r\n\
             RSeq: 42\r\n\
             Content-Length: 0\r\n\r\n",
            call_id = call_id
        );
        let _ = handle_datagram(
            ringing.as_bytes(),
            "198.51.100.20:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // 2. Send PRACK with missing RAck header
        let prack_missing = format!(
            "PRACK sip:edge.example.com SIP/2.0\r\n\
             Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-prack-miss\r\n\
             From: <sip:1001@example.com>;tag=from-tag\r\n\
             To: <sip:13800138000@example.com>;tag=gw-tag\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 2 PRACK\r\n\
             Content-Length: 0\r\n\r\n",
            call_id = call_id
        );
        let datagrams = handle_datagram(
            prack_missing.as_bytes(),
            "192.0.2.10:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        assert_eq!(datagrams.len(), 1);
        let resp = datagram_text(&datagrams[0]);
        assert!(resp.starts_with("SIP/2.0 400 Bad Request - Invalid RAck\r\n"));

        // 3. Send PRACK with valid RAck header
        let prack_valid = format!(
            "PRACK sip:edge.example.com SIP/2.0\r\n\
             Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-prack-val\r\n\
             From: <sip:1001@example.com>;tag=from-tag\r\n\
             To: <sip:13800138000@example.com>;tag=gw-tag\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 3 PRACK\r\n\
             RAck: 42 1 INVITE\r\n\
             Content-Length: 0\r\n\r\n",
            call_id = call_id
        );
        let datagrams_ok = handle_datagram(
            prack_valid.as_bytes(),
            "192.0.2.10:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        assert_eq!(datagrams_ok.len(), 1);
        let resp_ok = datagram_text(&datagrams_ok[0]);
        assert!(resp_ok.starts_with("SIP/2.0 200 OK\r\n"));
    }

    #[tokio::test]
    async fn test_sbc_ip_acl() {
        let mut config = edge_config();
        config.sbc_allow_rules = vec!["192.0.2.0/24".to_string()];
        config.sbc_block_rules = vec!["192.0.2.100".to_string()];
        let edge_state = state_with_default_route_and_config(&config);

        let packet = b"OPTIONS sip:edge.example.com SIP/2.0\r\n\r\n";
        let d1 = handle_datagram(
            packet,
            "192.0.2.50:5060".parse().unwrap(),
            &edge_state,
            &config,
        )
        .await;
        assert!(!d1.is_empty());

        let d2 = handle_datagram(
            packet,
            "192.0.2.100:5060".parse().unwrap(),
            &edge_state,
            &config,
        )
        .await;
        assert!(d2.is_empty());

        let d3 = handle_datagram(
            packet,
            "10.0.0.1:5060".parse().unwrap(),
            &edge_state,
            &config,
        )
        .await;
        assert!(d3.is_empty());
    }

    #[tokio::test]
    async fn test_sbc_cps_rate_limiting() {
        let mut config = edge_config();
        config.sbc_rate_limit_capacity = 2.0;
        config.sbc_rate_limit_fill_rate = 0.0;
        let edge_state = state_with_default_route_and_config(&config);

        let packet = b"OPTIONS sip:edge.example.com SIP/2.0\r\n\r\n";
        let peer_addr = "192.0.2.50:5060".parse().unwrap();

        let d1 = handle_datagram(packet, peer_addr, &edge_state, &config).await;
        assert!(!d1.is_empty());
        assert!(datagram_text(&d1[0]).starts_with("SIP/2.0 200 OK\r\n"));

        let d2 = handle_datagram(packet, peer_addr, &edge_state, &config).await;
        assert!(!d2.is_empty());
        assert!(datagram_text(&d2[0]).starts_with("SIP/2.0 200 OK\r\n"));

        let d3 = handle_datagram(packet, peer_addr, &edge_state, &config).await;
        assert!(!d3.is_empty());
        assert!(datagram_text(&d3[0])
            .starts_with("SIP/2.0 503 Service Unavailable - Rate Limit Exceeded\r\n"));
    }

    #[tokio::test]
    async fn test_sbc_concurrency_limiting() {
        let mut config = edge_config();
        config.sbc_max_concurrency = 1;
        let edge_state = state_with_default_route_and_config(&config);

        let call_id_1 = "call-concurrent-1";
        let body = sdp_body();
        let invite_1 = format!(
            concat!(
                "INVITE sip:13800138000@example.com SIP/2.0\r\n",
                "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-conc1\r\n",
                "From: <sip:1001@example.com>;tag=from-tag1\r\n",
                "To: <sip:13800138000@example.com>\r\n",
                "Call-ID: {call_id}\r\n",
                "CSeq: 1 INVITE\r\n",
                "Content-Length: {}\r\n\r\n",
                "{}"
            ),
            body.len(),
            body,
            call_id = call_id_1
        );

        let d1 = handle_datagram(
            invite_1.as_bytes(),
            "192.0.2.10:5060".parse().unwrap(),
            &edge_state,
            &config,
        )
        .await;
        assert_eq!(d1.len(), 2);

        let ok_200 = format!(
            "SIP/2.0 200 OK\r\n\
             Via: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-vosrs\r\n\
             Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-conc1\r\n\
             From: <sip:1001@example.com>;tag=from-tag1\r\n\
             To: <sip:13800138000@example.com>;tag=gw-tag\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 1 INVITE\r\n\
             Content-Length: 0\r\n\r\n",
            call_id = call_id_1
        );
        let _ = handle_datagram(
            ok_200.as_bytes(),
            "198.51.100.20:5060".parse().unwrap(),
            &edge_state,
            &config,
        )
        .await;

        let call_id_2 = "call-concurrent-2";
        let invite_2 = format!(
            concat!(
                "INVITE sip:13800138000@example.com SIP/2.0\r\n",
                "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-conc2\r\n",
                "From: <sip:1001@example.com>;tag=from-tag2\r\n",
                "To: <sip:13800138000@example.com>\r\n",
                "Call-ID: {call_id}\r\n",
                "CSeq: 1 INVITE\r\n",
                "Content-Length: {}\r\n\r\n",
                "{}"
            ),
            body.len(),
            body,
            call_id = call_id_2
        );

        let d2 = handle_datagram(
            invite_2.as_bytes(),
            "192.0.2.10:5060".parse().unwrap(),
            &edge_state,
            &config,
        )
        .await;
        assert_eq!(d2.len(), 1);
        let resp = datagram_text(&d2[0]);
        assert!(resp.starts_with("SIP/2.0 486 Busy Here - Concurrency Limit Exceeded\r\n"));
    }

    #[tokio::test]
    async fn test_path_service_route_propagation() {
        let config = edge_config();
        let edge_state = state_with_default_route_and_config(&config);

        let register = concat!(
            "REGISTER sip:example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-regpath\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:1001@example.com>\r\n",
            "Call-ID: reg-path-01\r\n",
            "CSeq: 1 REGISTER\r\n",
            "Contact: <sip:1001@192.0.2.10:5070;transport=udp>;expires=60\r\n",
            "Path: <sip:proxy1.example.com;lr>, <sip:proxy2.example.com;lr>\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        );

        let d1 = handle_datagram(
            register.as_bytes(),
            "192.0.2.10:5060".parse().unwrap(),
            &edge_state,
            &config,
        )
        .await;
        assert_eq!(d1.len(), 1);
        let resp = datagram_text(&d1[0]);
        assert!(resp.starts_with("SIP/2.0 200 OK\r\n"));
        assert!(resp.contains("Service-Route: <sip:127.0.0.1:5060;lr>\r\n"));

        let call_id = "path-invite-01";
        let body = sdp_body();
        let invite = format!(
            concat!(
                "INVITE sip:1001@example.com SIP/2.0\r\n",
                "Via: SIP/2.0/UDP 192.0.2.200:5060;branch=z9hG4bK-invitepath\r\n",
                "Max-Forwards: 70\r\n",
                "From: <sip:1002@example.com>;tag=caller-tag\r\n",
                "To: <sip:1001@example.com>\r\n",
                "Call-ID: {call_id}\r\n",
                "CSeq: 1 INVITE\r\n",
                "Content-Length: {}\r\n\r\n",
                "{}"
            ),
            body.len(),
            body,
            call_id = call_id
        );

        let d2 = handle_datagram(
            invite.as_bytes(),
            "192.0.2.200:5060".parse().unwrap(),
            &edge_state,
            &config,
        )
        .await;
        assert_eq!(d2.len(), 2);
        let forwarded_invite = datagram_text(&d2[1]);
        assert!(forwarded_invite.contains("Route: <sip:proxy1.example.com;lr>\r\n"));
        assert!(forwarded_invite.contains("Route: <sip:proxy2.example.com;lr>\r\n"));
    }

    #[tokio::test]
    async fn test_record_route_and_route_propagation() {
        let edge_state = state_with_default_route();
        let call_id = "rr-test@example.com";

        let invite = format!(
            concat!(
                "INVITE sip:13800138000@example.com SIP/2.0\r\n",
                "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-rr-invite\r\n",
                "Max-Forwards: 70\r\n",
                "From: <sip:1001@example.com>;tag=from-tag\r\n",
                "To: <sip:13800138000@example.com>\r\n",
                "Call-ID: {call_id}\r\n",
                "CSeq: 1 INVITE\r\n",
                "Record-Route: <sip:proxy-inbound.example.com;lr>\r\n",
                "Content-Length: 0\r\n",
                "\r\n"
            ),
            call_id = call_id
        );

        let datagrams =
            handle_datagram(invite.as_bytes(), peer(), &edge_state, &edge_config()).await;
        assert_eq!(datagrams.len(), 2);
        assert_eq!(datagrams[1].target, "gw1.example.com:5060");
        let invite_out = datagram_text(&datagrams[1]);
        assert!(!invite_out.contains("Record-Route:"));

        let response_200 = format!(
            concat!(
                "SIP/2.0 200 OK\r\n",
                "Via: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-vosrs\r\n",
                "From: <sip:1001@example.com>;tag=from-tag\r\n",
                "To: <sip:13800138000@example.com>;tag=gw-tag\r\n",
                "Call-ID: {call_id}\r\n",
                "CSeq: 1 INVITE\r\n",
                "Record-Route: <sip:proxy-outbound.example.com;lr>\r\n",
                "Contact: <sip:gateway-direct@198.51.100.20:5070;transport=udp>\r\n",
                "Content-Length: 0\r\n",
                "\r\n"
            ),
            call_id = call_id
        );

        let answer_datagrams = handle_datagram(
            response_200.as_bytes(),
            "198.51.100.20:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        assert_eq!(answer_datagrams.len(), 1);
        let response_to_caller = datagram_text(&answer_datagrams[0]);
        assert!(response_to_caller.contains("Record-Route: <sip:proxy-inbound.example.com;lr>\r\n"));

        let ack = format!(
            concat!(
                "ACK sip:13800138000@example.com SIP/2.0\r\n",
                "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-rr-ack\r\n",
                "Max-Forwards: 70\r\n",
                "From: <sip:1001@example.com>;tag=from-tag\r\n",
                "To: <sip:13800138000@example.com>;tag=gw-tag\r\n",
                "Call-ID: {call_id}\r\n",
                "CSeq: 1 ACK\r\n",
                "Content-Length: 0\r\n",
                "\r\n"
            ),
            call_id = call_id
        );

        let ack_datagrams =
            handle_datagram(ack.as_bytes(), peer(), &edge_state, &edge_config()).await;
        assert_eq!(ack_datagrams.len(), 1);
        assert_eq!(ack_datagrams[0].target, "proxy-outbound.example.com:5060");
        let ack_out = datagram_text(&ack_datagrams[0]);
        assert!(ack_out
            .starts_with("ACK sip:gateway-direct@198.51.100.20:5070;transport=udp SIP/2.0\r\n"));
        assert!(ack_out.contains("Route: <sip:proxy-outbound.example.com;lr>\r\n"));
    }

    #[tokio::test]
    async fn test_client_transaction_retransmission() {
        let edge_state = Arc::new(state_with_default_route());
        let socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let local_port = socket.local_addr().unwrap().port();
        let target = format!("127.0.0.1:{}", local_port);

        let req_bytes = b"INVITE sip:gw@127.0.0.1:5060 SIP/2.0\r\n\
                          Via: SIP/2.0/UDP 127.0.0.1:5060;branch=z9hG4bK-tx-test\r\n\
                          Call-ID: tx-test@example.com\r\n\
                          CSeq: 1 INVITE\r\n\
                          Content-Length: 0\r\n\r\n";
        let req = parse_message(req_bytes).unwrap();
        let SipMessage::Request(req) = req else {
            panic!("expected request");
        };
        let key = ClientTransactionKey::from_request(&req).unwrap();

        spawn_client_transaction_retransmission(
            Arc::clone(&edge_state),
            Arc::clone(&socket),
            target.clone(),
            req_bytes.to_vec(),
            key.clone(),
            edge_config(),
        );

        tokio::time::sleep(Duration::from_millis(15)).await;

        let resp = parse_message(
            b"SIP/2.0 180 Ringing\r\n\
              Via: SIP/2.0/UDP 127.0.0.1:5060;branch=z9hG4bK-tx-test\r\n\
              Call-ID: tx-test@example.com\r\n\
              CSeq: 1 INVITE\r\n\
              Content-Length: 0\r\n\r\n",
        )
        .unwrap();
        let SipMessage::Response(resp) = resp else {
            panic!("expected response");
        };
        let resp_key = ClientTransactionKey::from_response(&resp).unwrap();

        edge_state.cancel_client_transaction(&resp_key);

        tokio::time::sleep(Duration::from_millis(5)).await;
        assert!(!edge_state
            .client_transactions
            .contains_key(&key));
    }

    #[tokio::test]
    async fn test_client_transaction_timeout_triggers_failover() {
        let routes = RouteTable::new(vec![
            Route::new(
                "primary",
                "".to_string(),
                100,
                RouteTarget::new("gw1".to_string(), "127.0.0.1".to_string(), Some(12345)),
            ),
            Route::new(
                "secondary",
                "".to_string(),
                200,
                RouteTarget::new("gw2".to_string(), "127.0.0.1".to_string(), Some(23456)),
            ),
        ]);
        let edge_state = Arc::new(EdgeState::new(CallManager::new(routes)));
        let socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let call_id = "timeout-failover-test@example.com";

        let invite = format!(
            concat!(
                "INVITE sip:13800138000@example.com SIP/2.0\r\n",
                "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-timeout\r\n",
                "Max-Forwards: 70\r\n",
                "From: <sip:1001@example.com>;tag=from-tag\r\n",
                "To: <sip:13800138000@example.com>\r\n",
                "Call-ID: {call_id}\r\n",
                "CSeq: 1 INVITE\r\n",
                "Content-Length: 0\r\n",
                "\r\n"
            ),
            call_id = call_id
        );

        let datagrams =
            handle_datagram(invite.as_bytes(), peer(), &edge_state, &edge_config()).await;
        assert_eq!(datagrams.len(), 2);
        assert_eq!(datagrams[1].target, "127.0.0.1:23456");

        let outbound_invite = parse_message(&datagrams[1].bytes).unwrap();
        let SipMessage::Request(outbound_req) = outbound_invite else {
            panic!("expected request");
        };
        let key = ClientTransactionKey::from_request(&outbound_req).unwrap();

        spawn_client_transaction_retransmission(
            Arc::clone(&edge_state),
            Arc::clone(&socket),
            "127.0.0.1:23456".to_string(),
            datagrams[1].bytes.clone(),
            key,
            edge_config(),
        );

        let mut success = false;
        for _ in 0..15 {
            tokio::time::sleep(Duration::from_millis(20)).await;
            let call_guard = &edge_state.call_manager;
            if let Some(call) = call_guard.get(&CallId::new(call_id)) {
                if call.current_candidate_index == 1 {
                    success = true;
                    break;
                }
            }
        }
        assert!(success, "failed to trigger failover within timeout");

        let call_guard = &edge_state.call_manager;
        let call = call_guard.get(&CallId::new(call_id)).unwrap();
        assert_eq!(call.state, CallState::Routing);
        assert_eq!(
            call.outbound.as_ref().unwrap().remote_uri.to_string(),
            "sip:13800138000@127.0.0.1:12345;transport=udp"
        );
    }

    #[test]
    fn test_rtcp_mos_calculation() {
        use super::media::RtcpQualitySnapshot;

        // Case 1: Perfect RTCP metrics (0 delay, 0 loss)
        let caller = RtcpQualitySnapshot {
            reports: 1,
            sender_reports: 0,
            receiver_reports: 1,
            report_blocks: 1,
            last_fraction_lost: Some(0),
            max_fraction_lost: Some(0),
            last_cumulative_lost: Some(0),
            max_cumulative_lost: Some(0),
            last_jitter: Some(0),
            max_jitter: Some(0),
            last_sender_report: Some(0),
            delay_since_last_sender_report: Some(0),
            last_rtt_ms: Some(0),
            max_rtt_ms: Some(0),
        };
        let gateway = RtcpQualitySnapshot {
            reports: 1,
            sender_reports: 0,
            receiver_reports: 1,
            report_blocks: 1,
            last_fraction_lost: Some(0),
            max_fraction_lost: Some(0),
            last_cumulative_lost: Some(0),
            max_cumulative_lost: Some(0),
            last_jitter: Some(0),
            max_jitter: Some(0),
            last_sender_report: Some(0),
            delay_since_last_sender_report: Some(0),
            last_rtt_ms: Some(0),
            max_rtt_ms: Some(0),
        };

        let metrics = super::calculate_mos_for_legs(Some(&caller), Some(&gateway));
        assert!(metrics.mos.is_some());
        let mos_val = metrics.mos.unwrap();
        // Perfect MOS should be close to 4.4
        assert!(mos_val > 4.35 && mos_val <= 4.5);

        // Case 2: Degraded metrics (high loss, some delay)
        let caller_degraded = RtcpQualitySnapshot {
            reports: 1,
            sender_reports: 0,
            receiver_reports: 1,
            report_blocks: 1,
            last_fraction_lost: Some(25), // 25/256 ≈ 9.7% loss
            max_fraction_lost: Some(25),
            last_cumulative_lost: Some(0),
            max_cumulative_lost: Some(0),
            last_jitter: Some(0),
            max_jitter: Some(0),
            last_sender_report: Some(0),
            delay_since_last_sender_report: Some(0),
            last_rtt_ms: Some(150),
            max_rtt_ms: Some(150),
        };
        let gateway_degraded = RtcpQualitySnapshot {
            reports: 1,
            sender_reports: 0,
            receiver_reports: 1,
            report_blocks: 1,
            last_fraction_lost: Some(5), // 5/256 ≈ 2% loss
            max_fraction_lost: Some(5),
            last_cumulative_lost: Some(0),
            max_cumulative_lost: Some(0),
            last_jitter: Some(0),
            max_jitter: Some(0),
            last_sender_report: Some(0),
            delay_since_last_sender_report: Some(0),
            last_rtt_ms: Some(50),
            max_rtt_ms: Some(50),
        };

        let metrics_degraded =
            super::calculate_mos_for_legs(Some(&caller_degraded), Some(&gateway_degraded));
        assert!(metrics_degraded.mos.is_some());
        let mos_val_degraded = metrics_degraded.mos.unwrap();
        // High loss and delay should degrade MOS significantly
        assert!(mos_val_degraded < 3.0);
    }

    #[tokio::test]
    async fn test_cdr_mos_persistence() {
        let edge_state = state_with_default_route();
        let call_id = "mos-test@example.com";

        // 1. Setup call
        let invite = format!(
            "INVITE sip:13800138000@example.com SIP/2.0\r\n\
             Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-mos-invite\r\n\
             Max-Forwards: 70\r\n\
             From: <sip:1001@example.com>;tag=from-tag\r\n\
             To: <sip:13800138000@example.com>\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 1 INVITE\r\n\
             Content-Length: 0\r\n\r\n",
            call_id = call_id
        );

        let _ = handle_datagram(invite.as_bytes(), peer(), &edge_state, &edge_config()).await;

        // 2. Answer call
        let response_200 = format!(
            "SIP/2.0 200 OK\r\n\
             Via: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-vosrs\r\n\
             From: <sip:1001@example.com>;tag=from-tag\r\n\
             To: <sip:13800138000@example.com>;tag=gw-tag\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 1 INVITE\r\n\
             Content-Length: 0\r\n\r\n",
            call_id = call_id
        );

        let _ = handle_datagram(
            response_200.as_bytes(),
            "198.51.100.20:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // Before sending BYE, set some mock metrics in the media relay
        {
            let mut transaction = edge_state.inbound_transactions.get_mut(call_id).unwrap();

            // Assign dummy caller and gateway endpoints to simulate media ports
            let caller_endpoint = RtpEndpoint::new("127.0.0.1", 45000);
            let gateway_endpoint = RtpEndpoint::new("127.0.0.1", 45002);
            transaction.caller_relay_rtp = Some(caller_endpoint.clone());
            transaction.gateway_relay_rtp = Some(gateway_endpoint.clone());

            // Write dummy RTCP reports into the media relay state
            let rtcp_caller = super::media::RtcpQualitySnapshot {
                max_fraction_lost: Some(1), // 1/256 ≈ 0.4%
                max_rtt_ms: Some(40),
                max_jitter: Some(16), // 16 / 8 = 2ms
                ..Default::default()
            };

            let rtcp_gateway = super::media::RtcpQualitySnapshot {
                max_fraction_lost: Some(1), // 1/256 ≈ 0.4%
                max_rtt_ms: Some(20),
                max_jitter: Some(8), // 8 / 8 = 1ms
                ..Default::default()
            };

            edge_state
                .media_relay
                .record_rtcp_reports_for_test(caller_endpoint.port, rtcp_caller);
            edge_state
                .media_relay
                .record_rtcp_reports_for_test(gateway_endpoint.port, rtcp_gateway);
        };

        // 3. Collect mock metrics and terminate via BYE
        let bye = format!(
            "BYE sip:13800138000@example.com SIP/2.0\r\n\
             Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-mos-bye\r\n\
             From: <sip:1001@example.com>;tag=from-tag\r\n\
             To: <sip:13800138000@example.com>;tag=gw-tag\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 2 BYE\r\n\
             Content-Length: 0\r\n\r\n",
            call_id = call_id
        );

        let datagrams = handle_datagram(bye.as_bytes(), peer(), &edge_state, &edge_config()).await;
        assert_eq!(datagrams.len(), 2); // BYE ok response and forward BYE

        // Verify that the CDR contains the expected metrics and MOS
        let call_guard = &edge_state.call_manager;
        let cdrs = call_guard.completed_cdrs();
        assert_eq!(cdrs.len(), 1);
        let cdr = &cdrs[0];

        assert!(cdr.mos.is_some());
        let mos_val = cdr.mos.unwrap();
        assert!(mos_val > 3.5 && mos_val < 4.4); // typical reasonable quality under slight loss/jitter
        assert!(cdr.caller_rtcp_loss_rate.is_some());
        assert!(cdr.gateway_rtcp_loss_rate.is_some());
    }

    #[tokio::test]
    async fn test_sip_info_dtmf_extraction() {
        let edge_state = state_with_default_route();
        let call_id = "info-dtmf-test@example.com";

        // 1. Setup call
        let invite = format!(
            "INVITE sip:13800138000@example.com SIP/2.0\r\n\
             Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-info-invite\r\n\
             Max-Forwards: 70\r\n\
             From: <sip:1001@example.com>;tag=from-tag\r\n\
             To: <sip:13800138000@example.com>\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 1 INVITE\r\n\
             Content-Length: 0\r\n\r\n",
            call_id = call_id
        );
        let _ = handle_datagram(invite.as_bytes(), peer(), &edge_state, &edge_config()).await;

        // 2. Answer call
        let response_200 = format!(
            "SIP/2.0 200 OK\r\n\
             Via: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-vosrs\r\n\
             From: <sip:1001@example.com>;tag=from-tag\r\n\
             To: <sip:13800138000@example.com>;tag=gw-tag\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 1 INVITE\r\n\
             Content-Length: 0\r\n\r\n",
            call_id = call_id
        );
        let _ = handle_datagram(
            response_200.as_bytes(),
            "198.51.100.20:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // 3. Send SIP INFO with application/dtmf-relay body (digit '7')
        let body_relay = "Signal= 7\r\nDuration= 160\r\n";
        let info_relay = format!(
            "INFO sip:13800138000@example.com SIP/2.0\r\n\
             Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-info-relay\r\n\
             Max-Forwards: 70\r\n\
             From: <sip:1001@example.com>;tag=from-tag\r\n\
             To: <sip:13800138000@example.com>;tag=gw-tag\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 2 INFO\r\n\
             Content-Type: application/dtmf-relay\r\n\
             Content-Length: {len}\r\n\r\n\
             {body}",
            call_id = call_id,
            len = body_relay.len(),
            body = body_relay
        );
        let _ = handle_datagram(info_relay.as_bytes(), peer(), &edge_state, &edge_config()).await;

        // 4. Send SIP INFO with application/dtmf body (digit '8')
        let body_dtmf = "8";
        let info_dtmf = format!(
            "INFO sip:13800138000@example.com SIP/2.0\r\n\
             Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-info-dtmf\r\n\
             Max-Forwards: 70\r\n\
             From: <sip:1001@example.com>;tag=from-tag\r\n\
             To: <sip:13800138000@example.com>;tag=gw-tag\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 3 INFO\r\n\
             Content-Type: application/dtmf\r\n\
             Content-Length: {len}\r\n\r\n\
             {body}",
            call_id = call_id,
            len = body_dtmf.len(),
            body = body_dtmf
        );
        let _ = handle_datagram(info_dtmf.as_bytes(), peer(), &edge_state, &edge_config()).await;

        // 5. Send BYE to terminate call
        let bye = format!(
            "BYE sip:13800138000@example.com SIP/2.0\r\n\
             Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-info-bye\r\n\
             From: <sip:1001@example.com>;tag=from-tag\r\n\
             To: <sip:13800138000@example.com>;tag=gw-tag\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 4 BYE\r\n\
             Content-Length: 0\r\n\r\n",
            call_id = call_id
        );
        let _ = handle_datagram(bye.as_bytes(), peer(), &edge_state, &edge_config()).await;

        // 6. Verify that the CDR contains the expected DTMF digits "78"
        let call_guard = &edge_state.call_manager;
        let cdrs = call_guard.completed_cdrs();
        assert_eq!(cdrs.len(), 1);
        let cdr = &cdrs[0];
        assert_eq!(cdr.dtmf_digits.as_deref(), Some("78"));
    }

    #[tokio::test]
    async fn test_graceful_shutdown_draining_invite_receives_503() {
        let edge_state = state_with_default_route();

        // 1. Enable draining
        edge_state.draining.store(true, Ordering::Relaxed);

        // 2. Send INVITE request
        let invite = "INVITE sip:13800138000@example.com SIP/2.0\r\n\
                      Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-invite-draining\r\n\
                      Max-Forwards: 70\r\n\
                      From: <sip:1001@example.com>;tag=from-tag\r\n\
                      To: <sip:13800138000@example.com>\r\n\
                      Call-ID: draining-invite@example.com\r\n\
                      CSeq: 1 INVITE\r\n\
                      Content-Length: 0\r\n\r\n";
        let datagrams =
            handle_datagram(invite.as_bytes(), peer(), &edge_state, &edge_config()).await;
        assert_eq!(datagrams.len(), 1);
        let resp = String::from_utf8_lossy(&datagrams[0].bytes);
        assert!(resp.starts_with("SIP/2.0 503 Service Unavailable\r\n"));
        assert!(resp.contains("Retry-After: 30\r\n"));

        // 3. Send OPTIONS request (should still be processed normally during drain)
        let options = "OPTIONS sip:edge@example.com SIP/2.0\r\n\
                       Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-options-draining\r\n\
                       Max-Forwards: 70\r\n\
                       From: <sip:1001@example.com>;tag=from-tag\r\n\
                       To: <sip:edge@example.com>\r\n\
                       Call-ID: draining-options@example.com\r\n\
                       CSeq: 1 OPTIONS\r\n\
                       Content-Length: 0\r\n\r\n";
        let datagrams =
            handle_datagram(options.as_bytes(), peer(), &edge_state, &edge_config()).await;
        assert_eq!(datagrams.len(), 1);
        let resp = String::from_utf8_lossy(&datagrams[0].bytes);
        assert!(resp.starts_with("SIP/2.0 200 OK\r\n"));
    }

    #[tokio::test]
    async fn test_mid_dialog_reinvite_sdp_rewrite() {
        let edge_state = state_with_default_route();
        let call_id = "mid-dialog-sdp-test@example.com";

        // 1. Setup call with initial SDP offer
        let offer_body = concat!(
            "v=0\r\n",
            "o=caller 1 1 IN IP4 192.0.2.10\r\n",
            "s=caller\r\n",
            "c=IN IP4 192.0.2.10\r\n",
            "t=0 0\r\n",
            "m=audio 49170 RTP/AVP 0\r\n",
            "a=rtpmap:0 PCMU/8000\r\n"
        );
        let invite = format!(
            "INVITE sip:13800138000@example.com SIP/2.0\r\n\
             Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-init-invite\r\n\
             Max-Forwards: 70\r\n\
             From: <sip:1001@example.com>;tag=from-tag\r\n\
             To: <sip:13800138000@example.com>\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 1 INVITE\r\n\
             Content-Type: application/sdp\r\n\
             Content-Length: {len}\r\n\r\n\
             {body}",
            call_id = call_id,
            len = offer_body.len(),
            body = offer_body
        );

        let invite_datagrams =
            handle_datagram(invite.as_bytes(), peer(), &edge_state, &edge_config()).await;
        assert_eq!(invite_datagrams.len(), 2);

        // 2. Answer with 200 OK containing gateway SDP answer
        let answer_body = concat!(
            "v=0\r\n",
            "o=gateway 1 1 IN IP4 198.51.100.20\r\n",
            "s=gateway\r\n",
            "c=IN IP4 198.51.100.20\r\n",
            "t=0 0\r\n",
            "m=audio 49172 RTP/AVP 0\r\n",
            "a=rtpmap:0 PCMU/8000\r\n"
        );
        let response_200 = format!(
            "SIP/2.0 200 OK\r\n\
             Via: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-vosrs\r\n\
             From: <sip:1001@example.com>;tag=from-tag\r\n\
             To: <sip:13800138000@example.com>;tag=gw-tag\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 1 INVITE\r\n\
             Content-Type: application/sdp\r\n\
             Content-Length: {len}\r\n\r\n\
             {body}",
            call_id = call_id,
            len = answer_body.len(),
            body = answer_body
        );

        let answer_datagrams = handle_datagram(
            response_200.as_bytes(),
            "198.51.100.20:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;
        assert_eq!(answer_datagrams.len(), 1);

        // Verify initial relay endpoints are set up in the transaction
        let (caller_relay, gw_relay) = {
            let transaction = edge_state.inbound_transactions.get(call_id).unwrap();
            (
                transaction.caller_relay_rtp.clone().unwrap(),
                transaction.gateway_relay_rtp.clone().unwrap(),
            )
        };

        // 3. Simulate mid-dialog Re-INVITE (Call Hold / SDP renegotiation) from caller
        let hold_body = concat!(
            "v=0\r\n",
            "o=caller 1 2 IN IP4 192.0.2.10\r\n",
            "s=caller\r\n",
            "c=IN IP4 0.0.0.0\r\n", // Call Hold IP
            "t=0 0\r\n",
            "m=audio 49170 RTP/AVP 0\r\n",
            "a=rtpmap:0 PCMU/8000\r\n",
            "a=sendonly\r\n"
        );
        let reinvite = format!(
            "INVITE sip:13800138000@example.com SIP/2.0\r\n\
             Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-reinvite\r\n\
             Max-Forwards: 70\r\n\
             From: <sip:1001@example.com>;tag=from-tag\r\n\
             To: <sip:13800138000@example.com>;tag=gw-tag\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 2 INVITE\r\n\
             Content-Type: application/sdp\r\n\
             Content-Length: {len}\r\n\r\n\
             {body}",
            call_id = call_id,
            len = hold_body.len(),
            body = hold_body
        );

        let reinvite_datagrams =
            handle_datagram(reinvite.as_bytes(), peer(), &edge_state, &edge_config()).await;
        assert_eq!(reinvite_datagrams.len(), 1);

        // Verify the outgoing Re-INVITE has rewritten SDP presenting gw_relay (reusing the same port!)
        let forwarded_reinvite = datagram_text(&reinvite_datagrams[0]);
        assert!(forwarded_reinvite.contains(&format!("m=audio {} RTP/AVP 0\r\n", gw_relay.port)));

        // Verify target for gw_relay is updated to caller's new IP (0.0.0.0:49170)
        assert_eq!(
            edge_state.media_relay.target_for_port(gw_relay.port),
            Some("0.0.0.0:49170".parse().unwrap())
        );

        // 4. Gateway responds with 200 OK (renegotiation answer)
        let hold_answer_body = concat!(
            "v=0\r\n",
            "o=gateway 1 2 IN IP4 198.51.100.20\r\n",
            "s=gateway\r\n",
            "c=IN IP4 198.51.100.20\r\n",
            "t=0 0\r\n",
            "m=audio 49172 RTP/AVP 0\r\n",
            "a=rtpmap:0 PCMU/8000\r\n",
            "a=recvonly\r\n"
        );
        let reinvite_resp_200 = format!(
            "SIP/2.0 200 OK\r\n\
             Via: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-reinvite\r\n\
             From: <sip:1001@example.com>;tag=from-tag\r\n\
             To: <sip:13800138000@example.com>;tag=gw-tag\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 2 INVITE\r\n\
             Content-Type: application/sdp\r\n\
             Content-Length: {len}\r\n\r\n\
             {body}",
            call_id = call_id,
            len = hold_answer_body.len(),
            body = hold_answer_body
        );

        let reinvite_resp_datagrams = handle_datagram(
            reinvite_resp_200.as_bytes(),
            "198.51.100.20:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;
        assert_eq!(reinvite_resp_datagrams.len(), 1);

        // Verify the outgoing response to caller has rewritten SDP presenting caller_relay (reusing the same port!)
        let forwarded_resp = datagram_text(&reinvite_resp_datagrams[0]);
        assert!(forwarded_resp.contains(&format!("m=audio {} RTP/AVP 0\r\n", caller_relay.port)));

        // Verify target for caller_relay is still gateway target (198.51.100.20:49172)
        assert_eq!(
            edge_state.media_relay.target_for_port(caller_relay.port),
            Some("198.51.100.20:49172".parse().unwrap())
        );
    }

    /// Verify that the outbound INVITE carries Session-Expires and Supported: timer headers.
    #[tokio::test]
    async fn test_session_timer_header_injected_in_invite() {
        let edge_state = state_with_gateway_uri("sip:198.51.100.20:5060");
        let call_id = "session-timer-header-test-001";

        let sdp_body = "v=0\r\no=- 0 0 IN IP4 192.0.2.10\r\ns=-\r\nc=IN IP4 192.0.2.10\r\nt=0 0\r\nm=audio 49170 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n";
        let invite = format!(
            "INVITE sip:13800138000@example.com SIP/2.0\r\nVia: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-st-hdr\r\nMax-Forwards: 70\r\nFrom: <sip:1001@example.com>;tag=from-tag\r\nTo: <sip:13800138000@example.com>\r\nCall-ID: {call_id}\r\nCSeq: 1 INVITE\r\nContact: <sip:1001@192.0.2.10>\r\nContent-Type: application/sdp\r\nContent-Length: {len}\r\n\r\n{sdp_body}",
            len = sdp_body.len()
        );

        let datagrams = handle_datagram(
            invite.as_bytes(),
            "192.0.2.10:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;
        // Should produce 2 datagrams: 100 Trying to caller + outbound INVITE to gateway
        assert_eq!(datagrams.len(), 2);

        let outbound_invite = datagram_text(&datagrams[1]);
        assert!(
            outbound_invite.contains("Session-Expires: 600;refresher=uac"),
            "outbound INVITE must carry Session-Expires header\n{outbound_invite}"
        );
        assert!(
            outbound_invite.contains("Supported: timer"),
            "outbound INVITE must carry Supported: timer header\n{outbound_invite}"
        );
        assert!(
            outbound_invite.contains("Min-SE: 90"),
            "outbound INVITE must carry Min-SE header\n{outbound_invite}"
        );
    }

    /// Verify that a 200 OK containing Session-Expires stores the value on the transaction.
    #[tokio::test]
    async fn test_session_expires_stored_from_200_ok() {
        let edge_state = state_with_gateway_uri("sip:198.51.100.20:5060");
        let call_id = "session-timer-store-test-001";
        let sdp_body = "v=0\r\no=- 0 0 IN IP4 192.0.2.10\r\ns=-\r\nc=IN IP4 192.0.2.10\r\nt=0 0\r\nm=audio 49170 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n";

        // Step 1: send INVITE to establish transaction
        let invite = format!(
            "INVITE sip:13800138000@example.com SIP/2.0\r\n\
             Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-se-store\r\n\
             Max-Forwards: 70\r\n\
             From: <sip:1001@example.com>;tag=from-tag\r\n\
             To: <sip:13800138000@example.com>\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 1 INVITE\r\n\
             Contact: <sip:1001@192.0.2.10>\r\n\
             Content-Type: application/sdp\r\n\
             Content-Length: {len}\r\n\r\n{sdp_body}",
            len = sdp_body.len()
        );
        handle_datagram(
            invite.as_bytes(),
            "192.0.2.10:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // Step 2: gateway returns 200 OK with Session-Expires
        let sdp_answer = "v=0\r\no=- 0 0 IN IP4 198.51.100.20\r\ns=-\r\nc=IN IP4 198.51.100.20\r\nt=0 0\r\nm=audio 49172 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n";
        let ok_200 = format!(
            "SIP/2.0 200 OK\r\n\
             Via: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-se-store\r\n\
             From: <sip:1001@example.com>;tag=from-tag\r\n\
             To: <sip:13800138000@example.com>;tag=gw-tag\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 1 INVITE\r\n\
             Session-Expires: 600;refresher=uac\r\n\
             Content-Type: application/sdp\r\n\
             Content-Length: {len}\r\n\r\n{sdp_answer}",
            len = sdp_answer.len()
        );
        handle_datagram(
            ok_200.as_bytes(),
            "198.51.100.20:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // Verify session timer was stored on the transaction
        let tx = edge_state.inbound_transactions.get(call_id).expect("transaction must exist");
        assert_eq!(
            tx.session_expires,
            Some(600),
            "session_expires must be stored"
        );
        assert_eq!(
            tx.session_refresher.as_deref(),
            Some("uac"),
            "refresher must be stored"
        );
        assert!(
            tx.last_session_refresh.is_some(),
            "last_session_refresh must be set"
        );
    }

    /// Verify that Re-INVITE resets the last_session_refresh timestamp.
    #[tokio::test]
    async fn test_session_refresh_resets_on_reinvite() {
        let edge_state = state_with_gateway_uri("sip:198.51.100.20:5060");
        let call_id = "session-timer-refresh-test-001";
        let sdp_body = "v=0\r\no=- 0 0 IN IP4 192.0.2.10\r\ns=-\r\nc=IN IP4 192.0.2.10\r\nt=0 0\r\nm=audio 49170 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n";

        // Establish the call
        let invite = format!(
            "INVITE sip:13800138000@example.com SIP/2.0\r\n\
             Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-se-refresh\r\n\
             Max-Forwards: 70\r\n\
             From: <sip:1001@example.com>;tag=from-tag\r\n\
             To: <sip:13800138000@example.com>\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 1 INVITE\r\n\
             Contact: <sip:1001@192.0.2.10>\r\n\
             Content-Type: application/sdp\r\n\
             Content-Length: {len}\r\n\r\n{sdp_body}",
            len = sdp_body.len()
        );
        handle_datagram(
            invite.as_bytes(),
            "192.0.2.10:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        let sdp_answer = "v=0\r\no=- 0 0 IN IP4 198.51.100.20\r\ns=-\r\nc=IN IP4 198.51.100.20\r\nt=0 0\r\nm=audio 49172 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n";
        let ok_200 = format!(
            "SIP/2.0 200 OK\r\n\
             Via: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-se-refresh\r\n\
             From: <sip:1001@example.com>;tag=from-tag\r\n\
             To: <sip:13800138000@example.com>;tag=gw-tag\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 1 INVITE\r\n\
             Session-Expires: 600;refresher=uac\r\n\
             Content-Type: application/sdp\r\n\
             Content-Length: {len}\r\n\r\n{sdp_answer}",
            len = sdp_answer.len()
        );
        handle_datagram(
            ok_200.as_bytes(),
            "198.51.100.20:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // Capture last_session_refresh time before Re-INVITE
        let before_reinvite = {
            let tx_guard = edge_state.inbound_transactions.get(call_id).unwrap(); tx_guard.last_session_refresh
        };

        // Small delay so the timestamp will differ
        tokio::time::sleep(Duration::from_millis(5)).await;

        // Send Re-INVITE (To has tag) — this acts as session refresh
        let reinvite_sdp = sdp_body;
        let reinvite = format!(
            "INVITE sip:13800138000@example.com SIP/2.0\r\n\
             Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-se-refresh-2\r\n\
             Max-Forwards: 70\r\n\
             From: <sip:1001@example.com>;tag=from-tag\r\n\
             To: <sip:13800138000@example.com>;tag=gw-tag\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 2 INVITE\r\n\
             Contact: <sip:1001@192.0.2.10>\r\n\
             Content-Type: application/sdp\r\n\
             Content-Length: {len}\r\n\r\n{reinvite_sdp}",
            len = reinvite_sdp.len()
        );
        handle_datagram(
            reinvite.as_bytes(),
            "192.0.2.10:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        let after_reinvite = {
            let tx_guard = edge_state.inbound_transactions.get(call_id).unwrap(); tx_guard.last_session_refresh
        };

        assert!(before_reinvite.is_some(), "initial timestamp must be set");
        assert!(
            after_reinvite.is_some(),
            "post-reinvite timestamp must be set"
        );
        assert!(
            after_reinvite.unwrap() >= before_reinvite.unwrap(),
            "last_session_refresh must be updated after Re-INVITE"
        );
    }

    // ── outbound::tests additions ────────────────────────────────────────────

    /// Verify the outbound INVITE contains both 'timer' and '100rel' in Supported.
    #[tokio::test]
    async fn test_invite_supported_includes_100rel() {
        let edge_state = state_with_gateway_uri("sip:198.51.100.20:5060");
        let sdp_body = "v=0\r\no=- 0 0 IN IP4 192.0.2.10\r\ns=-\r\nc=IN IP4 192.0.2.10\r\nt=0 0\r\nm=audio 49170 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n";
        let call_id = "prack-supported-test-001";
        let invite = format!(
            "INVITE sip:13800138000@example.com SIP/2.0\r\nVia: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-pr-sup\r\nMax-Forwards: 70\r\nFrom: <sip:1001@example.com>;tag=from-tag\r\nTo: <sip:13800138000@example.com>\r\nCall-ID: {call_id}\r\nCSeq: 1 INVITE\r\nContact: <sip:1001@192.0.2.10>\r\nContent-Type: application/sdp\r\nContent-Length: {len}\r\n\r\n{sdp_body}",
            len = sdp_body.len()
        );
        let datagrams = handle_datagram(
            invite.as_bytes(),
            "192.0.2.10:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;
        assert_eq!(datagrams.len(), 2);
        let outbound = datagram_text(&datagrams[1]);
        assert!(
            outbound.contains("Supported: timer,100rel"),
            "outbound INVITE must advertise both timer and 100rel\n{outbound}"
        );
    }

    /// Verify that a 180 with Require: 100rel causes sip-edge to:
    ///   - emit a PRACK toward the gateway
    ///   - forward the 180 (with rewritten RSeq) toward the caller
    #[tokio::test]
    async fn test_180_with_100rel_triggers_prack_to_gateway() {
        let edge_state = state_with_gateway_uri("sip:198.51.100.20:5060");
        let call_id = "prack-180-test-001";
        let sdp_body = "v=0\r\no=- 0 0 IN IP4 192.0.2.10\r\ns=-\r\nc=IN IP4 192.0.2.10\r\nt=0 0\r\nm=audio 49170 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n";

        // 1. Establish inbound INVITE to create transaction
        let invite = format!(
            "INVITE sip:13800138000@example.com SIP/2.0\r\nVia: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-pr-180\r\nMax-Forwards: 70\r\nFrom: <sip:1001@example.com>;tag=from-tag\r\nTo: <sip:13800138000@example.com>\r\nCall-ID: {call_id}\r\nCSeq: 1 INVITE\r\nContact: <sip:1001@192.0.2.10>\r\nContent-Type: application/sdp\r\nContent-Length: {len}\r\n\r\n{sdp_body}",
            len = sdp_body.len()
        );
        handle_datagram(
            invite.as_bytes(),
            "192.0.2.10:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // 2. Gateway sends 180 Ringing with Require: 100rel and RSeq: 42
        let ringing_180 = format!(
            "SIP/2.0 180 Ringing\r\nVia: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-pr-180\r\nFrom: <sip:1001@example.com>;tag=from-tag\r\nTo: <sip:13800138000@example.com>;tag=gw-tag\r\nCall-ID: {call_id}\r\nCSeq: 1 INVITE\r\nRequire: 100rel\r\nRSeq: 42\r\nContent-Length: 0\r\n\r\n"
        );
        let datagrams = handle_datagram(
            ringing_180.as_bytes(),
            "198.51.100.20:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // Should produce exactly 2 datagrams:
        //   [0] PRACK → gateway
        //   [1] 180 Ringing (rewritten RSeq: 1) → caller
        assert_eq!(datagrams.len(), 2, "expected PRACK + forwarded 180");

        let prack = datagram_text(&datagrams[0]);
        assert!(
            prack.starts_with("PRACK "),
            "first datagram must be PRACK\n{prack}"
        );
        assert!(
            prack.contains("RAck: 42 1 INVITE"),
            "PRACK must echo gateway RSeq in RAck\n{prack}"
        );

        let forwarded_180 = datagram_text(&datagrams[1]);
        assert!(
            forwarded_180.contains("180 Ringing"),
            "second datagram must be the 180\n{forwarded_180}"
        );
        assert!(
            forwarded_180.contains("Require: 100rel"),
            "forwarded 180 must keep Require: 100rel\n{forwarded_180}"
        );
        assert!(
            forwarded_180.contains("RSeq: 1"),
            "RSeq must be rewritten to 1 by sip-edge\n{forwarded_180}"
        );

        // Verify transaction state was updated
        let tx = edge_state.inbound_transactions.get(call_id).unwrap();
        assert!(
            tx.gateway_100rel,
            "gateway_100rel must be set after receiving Require: 100rel"
        );
        assert_eq!(tx.prack_rseq, 1, "prack_rseq counter must be 1");
    }

    /// Verify that a PRACK from the caller receives 200 OK and is not forwarded.
    #[tokio::test]
    async fn test_prack_from_caller_receives_200_ok_only() {
        let edge_state = state_with_gateway_uri("sip:198.51.100.20:5060");
        let call_id = "prack-ack-test-001";
        let sdp_body = "v=0\r\no=- 0 0 IN IP4 192.0.2.10\r\ns=-\r\nc=IN IP4 192.0.2.10\r\nt=0 0\r\nm=audio 49170 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n";

        // Establish transaction and trigger 100rel so prack_rseq > 0
        let invite = format!(
            "INVITE sip:13800138000@example.com SIP/2.0\r\nVia: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-prack-ack\r\nMax-Forwards: 70\r\nFrom: <sip:1001@example.com>;tag=from-tag\r\nTo: <sip:13800138000@example.com>\r\nCall-ID: {call_id}\r\nCSeq: 1 INVITE\r\nContact: <sip:1001@192.0.2.10>\r\nContent-Type: application/sdp\r\nContent-Length: {len}\r\n\r\n{sdp_body}",
            len = sdp_body.len()
        );
        handle_datagram(
            invite.as_bytes(),
            "192.0.2.10:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;
        handle_datagram(
            format!("SIP/2.0 180 Ringing\r\nVia: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-prack-ack\r\nFrom: <sip:1001@example.com>;tag=from-tag\r\nTo: <sip:13800138000@example.com>;tag=gw-tag\r\nCall-ID: {call_id}\r\nCSeq: 1 INVITE\r\nRequire: 100rel\r\nRSeq: 1\r\nContent-Length: 0\r\n\r\n").as_bytes(),
            "198.51.100.20:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        ).await;

        // Caller sends PRACK
        let caller_prack = format!(
            "PRACK sip:edge.example.com SIP/2.0\r\nVia: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-caller-prack\r\nMax-Forwards: 70\r\nFrom: <sip:1001@example.com>;tag=from-tag\r\nTo: <sip:13800138000@example.com>;tag=gw-tag\r\nCall-ID: {call_id}\r\nCSeq: 2 PRACK\r\nRAck: 1 1 INVITE\r\nContent-Length: 0\r\n\r\n"
        );
        let datagrams = handle_datagram(
            caller_prack.as_bytes(),
            "192.0.2.10:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // Must produce exactly 1 datagram: 200 OK to caller (no forwarding to gateway)
        assert_eq!(
            datagrams.len(),
            1,
            "PRACK must only produce 200 OK — not forwarded\n{:?}",
            datagrams.iter().map(datagram_text).collect::<Vec<_>>()
        );
        let resp = datagram_text(&datagrams[0]);
        assert!(resp.starts_with("SIP/2.0 200 OK"), "must be 200 OK\n{resp}");
    }

    // ── Early Media (183 Session Progress + SDP) ────────────────────────────

    /// Verify that a 183 with SDP:
    ///   - allocates relay ports for early media
    ///   - rewrites the SDP endpoint to the relay IP
    ///   - forwards the 183 to the caller
    /// And that the subsequent 200 OK still finalises media correctly.
    #[tokio::test]
    async fn test_early_media_183_sdp_allocates_relay_and_forwards() {
        let edge_state = state_with_default_route();
        let call_id = "early-media-test-001";
        let offer_sdp = "v=0\r\no=- 0 0 IN IP4 192.0.2.10\r\ns=-\r\nc=IN IP4 192.0.2.10\r\nt=0 0\r\nm=audio 49170 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n";

        // Step 1: caller sends INVITE
        let invite = format!(
            "INVITE sip:13800138000@example.com SIP/2.0\r\nVia: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-em-001\r\nMax-Forwards: 70\r\nFrom: <sip:1001@example.com>;tag=from-tag\r\nTo: <sip:13800138000@example.com>\r\nCall-ID: {call_id}\r\nCSeq: 1 INVITE\r\nContact: <sip:1001@192.0.2.10>\r\nContent-Type: application/sdp\r\nContent-Length: {len}\r\n\r\n{offer_sdp}",
            len = offer_sdp.len()
        );
        let invite_datagrams = handle_datagram(
            invite.as_bytes(),
            "192.0.2.10:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;
        assert_eq!(
            invite_datagrams.len(),
            2,
            "INVITE must produce 100 Trying + outbound INVITE"
        );

        // Extract the relay port allocated for the gateway-facing side
        let outbound_invite_body = datagram_text(&invite_datagrams[1]);
        let gateway_relay_port: u16 = {
            outbound_invite_body
                .lines()
                .find(|l| l.starts_with("m=audio"))
                .and_then(|l| l.split_whitespace().nth(1)?.parse().ok())
                .expect("outbound INVITE must have m=audio with relay port")
        };

        // Step 2: gateway sends 183 Session Progress with SDP (early media)
        let early_sdp = "v=0\r\no=- 0 0 IN IP4 198.51.100.20\r\ns=-\r\nc=IN IP4 198.51.100.20\r\nt=0 0\r\nm=audio 49200 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n";
        let session_progress_183 = format!(
            "SIP/2.0 183 Session Progress\r\nVia: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-em-001\r\nFrom: <sip:1001@example.com>;tag=from-tag\r\nTo: <sip:13800138000@example.com>;tag=gw-tag\r\nCall-ID: {call_id}\r\nCSeq: 1 INVITE\r\nContact: <sip:13800138000@gw1.example.com:5060>\r\nContent-Type: application/sdp\r\nContent-Length: {len}\r\n\r\n{early_sdp}",
            len = early_sdp.len()
        );
        let datagrams_183 = handle_datagram(
            session_progress_183.as_bytes(),
            "203.0.113.20:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // 183 with SDP must be forwarded to caller with rewritten media endpoint
        assert_eq!(datagrams_183.len(), 1, "183 must be forwarded to caller");
        let forwarded_183 = datagram_text(&datagrams_183[0]);
        assert!(
            forwarded_183.contains("183 Session Progress"),
            "forwarded message must be 183\n{forwarded_183}"
        );
        assert!(
            forwarded_183.contains("203.0.113.10"),
            "SDP must be rewritten to relay IP\n{forwarded_183}"
        );

        // The relay port facing the caller (caller_relay_rtp) must now be set
        let caller_relay_port: u16 = {
            forwarded_183
                .lines()
                .find(|l| l.starts_with("m=audio"))
                .and_then(|l| l.split_whitespace().nth(1)?.parse().ok())
                .expect("forwarded 183 must have m=audio with caller relay port")
        };
        assert_ne!(
            caller_relay_port, 49200,
            "SDP port must be rewritten away from gateway port"
        );
        assert_ne!(
            caller_relay_port, gateway_relay_port,
            "caller relay port must differ from gateway relay port"
        );

        // The caller-facing relay port target must point to the gateway's early media endpoint
        // (caller-relay → gateway direction)
        assert_eq!(
            edge_state.media_relay.target_for_port(caller_relay_port),
            Some("198.51.100.20:49200".parse().unwrap()),
            "caller relay must target the gateway early media port"
        );

        // The gateway relay (allocated during INVITE) target must point to the caller
        // (gateway-relay → caller direction, set during INVITE SDP rewrite)
        assert_eq!(
            edge_state.media_relay.target_for_port(gateway_relay_port),
            Some("192.0.2.10:49170".parse().unwrap()),
            "gateway relay must still target the caller's RTP port"
        );

        // Step 3: verify final 200 OK is still handled correctly after early media
        let final_sdp = "v=0\r\no=- 0 0 IN IP4 198.51.100.20\r\ns=-\r\nc=IN IP4 198.51.100.20\r\nt=0 0\r\nm=audio 49202 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n";
        let ok_200 = format!(
            "SIP/2.0 200 OK\r\nVia: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-em-001\r\nFrom: <sip:1001@example.com>;tag=from-tag\r\nTo: <sip:13800138000@example.com>;tag=gw-tag\r\nCall-ID: {call_id}\r\nCSeq: 1 INVITE\r\nContact: <sip:13800138000@gw1.example.com:5060>\r\nContent-Type: application/sdp\r\nContent-Length: {len}\r\n\r\n{final_sdp}",
            len = final_sdp.len()
        );
        let datagrams_200 = handle_datagram(
            ok_200.as_bytes(),
            "203.0.113.20:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        assert_eq!(datagrams_200.len(), 1, "200 OK must be forwarded to caller");
        let forwarded_200 = datagram_text(&datagrams_200[0]);
        assert!(
            forwarded_200.starts_with("SIP/2.0 200 OK"),
            "must be 200 OK\n{forwarded_200}"
        );
        assert!(
            forwarded_200.contains("203.0.113.10"),
            "200 OK SDP must also use relay IP\n{forwarded_200}"
        );

        // Gateway relay target must be updated to final SDP port — check the caller-relay port
        // (The caller-relay → gateway direction is what gets updated when the final SDP arrives)
        assert_eq!(
            edge_state.media_relay.target_for_port(caller_relay_port),
            Some("198.51.100.20:49202".parse().unwrap()),
            "caller relay target must be updated to final media port from 200 OK"
        );
    }

    /// Verify that a 183 WITHOUT SDP is forwarded as-is (no relay allocation needed).
    #[tokio::test]
    async fn test_183_without_sdp_forwarded_unchanged() {
        let edge_state = state_with_default_route();
        let call_id = "early-media-no-sdp-001";
        let offer_sdp = "v=0\r\no=- 0 0 IN IP4 192.0.2.10\r\ns=-\r\nc=IN IP4 192.0.2.10\r\nt=0 0\r\nm=audio 49170 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n";

        let invite = format!(
            "INVITE sip:13800138000@example.com SIP/2.0\r\nVia: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-em-nossdp\r\nMax-Forwards: 70\r\nFrom: <sip:1001@example.com>;tag=from-tag\r\nTo: <sip:13800138000@example.com>\r\nCall-ID: {call_id}\r\nCSeq: 1 INVITE\r\nContact: <sip:1001@192.0.2.10>\r\nContent-Type: application/sdp\r\nContent-Length: {len}\r\n\r\n{offer_sdp}",
            len = offer_sdp.len()
        );
        handle_datagram(
            invite.as_bytes(),
            "192.0.2.10:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // 183 without SDP body
        let session_progress_183 = format!(
            "SIP/2.0 183 Session Progress\r\nVia: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-em-nossdp\r\nFrom: <sip:1001@example.com>;tag=from-tag\r\nTo: <sip:13800138000@example.com>;tag=gw-tag\r\nCall-ID: {call_id}\r\nCSeq: 1 INVITE\r\nContent-Length: 0\r\n\r\n"
        );
        let datagrams = handle_datagram(
            session_progress_183.as_bytes(),
            "203.0.113.20:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // 183 without SDP = forward straight to caller
        assert_eq!(datagrams.len(), 1, "183 without SDP must be forwarded");
        let forwarded = datagram_text(&datagrams[0]);
        assert!(
            forwarded.contains("183 Session Progress"),
            "must be 183\n{forwarded}"
        );

        // No caller_relay_rtp should be set (no SDP to allocate relay for)
        let tx = edge_state.inbound_transactions.get(call_id).expect("transaction must exist");
        assert!(
            tx.caller_relay_rtp.is_none(),
            "no relay should be set without SDP in 183"
        );
    }

    #[tokio::test]
    async fn test_refer_local_transfer_lifecycle() {
        let edge_state = state_with_default_route();
        let call_id = "refer-lifecycle-001";
        let offer_sdp = "v=0\r\no=- 0 0 IN IP4 192.0.2.10\r\ns=-\r\nc=IN IP4 192.0.2.10\r\nt=0 0\r\nm=audio 49170 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n";

        // 1. Inbound INVITE
        let invite = format!(
            "INVITE sip:13800138000@example.com SIP/2.0\r\nVia: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-inv-001\r\nMax-Forwards: 70\r\nFrom: <sip:1001@example.com>;tag=from-tag\r\nTo: <sip:13800138000@example.com>\r\nCall-ID: {call_id}\r\nCSeq: 1 INVITE\r\nContact: <sip:1001@192.0.2.10>\r\nContent-Type: application/sdp\r\nContent-Length: {len}\r\n\r\n{offer_sdp}",
            len = offer_sdp.len()
        );
        handle_datagram(
            invite.as_bytes(),
            "192.0.2.10:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // 2. Gateway responds 200 OK
        let answer_sdp = "v=0\r\no=- 0 0 IN IP4 198.51.100.20\r\ns=-\r\nc=IN IP4 198.51.100.20\r\nt=0 0\r\nm=audio 49200 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n";
        let ok_200 = format!(
            "SIP/2.0 200 OK\r\nVia: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-inv-001\r\nFrom: <sip:1001@example.com>;tag=from-tag\r\nTo: <sip:13800138000@example.com>;tag=gw-tag\r\nCall-ID: {call_id}\r\nCSeq: 1 INVITE\r\nContact: <sip:13800138000@gw1.example.com:5060>\r\nContent-Type: application/sdp\r\nContent-Length: {len}\r\n\r\n{answer_sdp}",
            len = answer_sdp.len()
        );
        handle_datagram(
            ok_200.as_bytes(),
            "203.0.113.20:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // 3. Caller sends REFER to transfer gateway B to target C (sip:1002@example.com)
        let refer = format!(
            "REFER sip:edge.example.com SIP/2.0\r\n\
             Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-ref-001\r\n\
             Max-Forwards: 70\r\n\
             From: <sip:1001@example.com>;tag=from-tag\r\n\
             To: <sip:13800138000@example.com>;tag=gw-tag\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 2 REFER\r\n\
             Refer-To: <sip:1002@example.com>\r\n\
             Content-Length: 0\r\n\r\n"
        );
        let datagrams = handle_datagram(
            refer.as_bytes(),
            "192.0.2.10:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // Should return 3 datagrams:
        // [0] 202 Accepted to referrer
        // [1] NOTIFY (100 Trying) to referrer
        // [2] INVITE to target C
        assert_eq!(datagrams.len(), 3, "expected 202 + NOTIFY + INVITE");

        let response_202 = datagram_text(&datagrams[0]);
        assert!(response_202.starts_with("SIP/2.0 202 Accepted\r\n"));

        let notify_trying = datagram_text(&datagrams[1]);
        assert!(notify_trying.starts_with("NOTIFY sip:1001@example.com SIP/2.0\r\n"));
        assert!(notify_trying.contains("SIP/2.0 100 Trying\r\n"));

        let invite_c = datagram_text(&datagrams[2]);
        assert!(
            invite_c.starts_with("INVITE sip:1002@gw1.example.com:5060;transport=udp SIP/2.0\r\n")
        );

        // Extract transfer call id
        let transfer_call_id = {
            invite_c
                .lines()
                .find(|l| l.starts_with("Call-ID:"))
                .unwrap()
                .split_whitespace()
                .nth(1)
                .unwrap()
                .to_string()
        };

        // 4. Target C responds 180 Ringing
        let ringing_180 = format!(
            "SIP/2.0 180 Ringing\r\n\
             Via: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-transfer-refer-lifecycle-001-52\r\n\
             From: <sip:13800138000@example.com>;tag=gw-tag\r\n\
             To: <sip:1002@example.com>;tag=c-tag\r\n\
             Call-ID: {transfer_call_id}\r\n\
             CSeq: 1 INVITE\r\n\
             Content-Length: 0\r\n\r\n"
        );
        let ringing_datagrams = handle_datagram(
            ringing_180.as_bytes(),
            "203.0.113.20:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;
        assert_eq!(ringing_datagrams.len(), 1);
        let notify_ringing = datagram_text(&ringing_datagrams[0]);
        assert!(notify_ringing.contains("SIP/2.0 180 Ringing\r\n"));

        // 5. Target C responds 200 OK with SDP
        let target_sdp = "v=0\r\no=- 0 0 IN IP4 198.51.100.30\r\ns=-\r\nc=IN IP4 198.51.100.30\r\nt=0 0\r\nm=audio 49300 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n";
        let answer_200 = format!(
            "SIP/2.0 200 OK\r\n\
             Via: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-transfer-refer-lifecycle-001-52\r\n\
             From: <sip:13800138000@example.com>;tag=gw-tag\r\n\
             To: <sip:1002@example.com>;tag=c-tag\r\n\
             Call-ID: {transfer_call_id}\r\n\
             CSeq: 1 INVITE\r\n\
             Contact: <sip:1002@198.51.100.30:5060>\r\n\
             Content-Type: application/sdp\r\n\
             Content-Length: {len}\r\n\r\n{target_sdp}",
            len = target_sdp.len()
        );
        let ok_datagrams = handle_datagram(
            answer_200.as_bytes(),
            "203.0.113.20:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // Should return 2 datagrams:
        // [0] NOTIFY (200 OK) to referrer
        // [1] BYE to referrer
        assert_eq!(ok_datagrams.len(), 2);
        let notify_ok = datagram_text(&ok_datagrams[0]);
        assert!(notify_ok.contains("SIP/2.0 200 OK\r\n"));
        assert!(notify_ok.contains("Subscription-State: terminated;reason=noresource\r\n"));

        let bye = datagram_text(&ok_datagrams[1]);
        assert!(bye.starts_with("BYE "));

        // Verify media bridging
        let tx = edge_state.inbound_transactions.get(call_id).unwrap();

        // Check if gateway's relay port target is updated to C's media IP/port (198.51.100.30:49300)
        let gw_relay = tx.gateway_relay_rtp.as_ref().unwrap();
        assert_eq!(
            edge_state.media_relay.target_for_port(gw_relay.port),
            Some("198.51.100.30:49300".parse().unwrap())
        );

        // Check if target C's relay port target is updated to the transferee's remote media endpoint (198.51.100.20:49200)
        let c_port = invite_c
            .lines()
            .find(|l| l.starts_with("m=audio"))
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap()
            .parse::<u16>()
            .unwrap();
        assert_eq!(
            edge_state.media_relay.target_for_port(c_port),
            Some("198.51.100.20:49200".parse().unwrap())
        );
    }

    #[tokio::test]
    async fn test_refer_transfer_failure_rollback() {
        let edge_state = state_with_default_route();
        let call_id = "refer-rollback-001";
        let offer_sdp = "v=0\r\no=- 0 0 IN IP4 192.0.2.10\r\ns=-\r\nc=IN IP4 192.0.2.10\r\nt=0 0\r\nm=audio 49170 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n";

        // 1. Inbound INVITE
        let invite = format!(
            "INVITE sip:13800138000@example.com SIP/2.0\r\nVia: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-inv-002\r\nMax-Forwards: 70\r\nFrom: <sip:1001@example.com>;tag=from-tag\r\nTo: <sip:13800138000@example.com>\r\nCall-ID: {call_id}\r\nCSeq: 1 INVITE\r\nContact: <sip:1001@192.0.2.10>\r\nContent-Type: application/sdp\r\nContent-Length: {len}\r\n\r\n{offer_sdp}",
            len = offer_sdp.len()
        );
        handle_datagram(
            invite.as_bytes(),
            "192.0.2.10:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // 2. Gateway responds 200 OK
        let answer_sdp = "v=0\r\no=- 0 0 IN IP4 198.51.100.20\r\ns=-\r\nc=IN IP4 198.51.100.20\r\nt=0 0\r\nm=audio 49200 RTP/AVP 0\r\na=rtpmap:0 PCMU/8000\r\n";
        let ok_200 = format!(
            "SIP/2.0 200 OK\r\nVia: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-inv-002\r\nFrom: <sip:1001@example.com>;tag=from-tag\r\nTo: <sip:13800138000@example.com>;tag=gw-tag\r\nCall-ID: {call_id}\r\nCSeq: 1 INVITE\r\nContact: <sip:13800138000@gw1.example.com:5060>\r\nContent-Type: application/sdp\r\nContent-Length: {len}\r\n\r\n{answer_sdp}",
            len = answer_sdp.len()
        );
        handle_datagram(
            ok_200.as_bytes(),
            "203.0.113.20:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // Verify initial media pairing and target config
        {
            let tx = edge_state.inbound_transactions.get(call_id).unwrap();
            let caller_relay = tx.caller_relay_rtp.as_ref().unwrap();
            let gw_relay = tx.gateway_relay_rtp.as_ref().unwrap();
            assert_eq!(
                edge_state.media_relay.peer_port_for(caller_relay.port),
                Some(gw_relay.port)
            );
            assert_eq!(
                edge_state.media_relay.target_for_port(caller_relay.port),
                Some("198.51.100.20:49200".parse().unwrap())
            );
            assert_eq!(
                edge_state.media_relay.target_for_port(gw_relay.port),
                Some("192.0.2.10:49170".parse().unwrap())
            );
        }

        // 3. Caller sends REFER to transfer gateway B to target C (sip:1002@example.com)
        let refer = format!(
            "REFER sip:edge.example.com SIP/2.0\r\n\
             Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-ref-002\r\n\
             Max-Forwards: 70\r\n\
             From: <sip:1001@example.com>;tag=from-tag\r\n\
             To: <sip:13800138000@example.com>;tag=gw-tag\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 2 REFER\r\n\
             Refer-To: <sip:1002@example.com>\r\n\
             Content-Length: 0\r\n\r\n"
        );
        let datagrams = handle_datagram(
            refer.as_bytes(),
            "192.0.2.10:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;
        assert_eq!(datagrams.len(), 3);
        let invite_c = datagram_text(&datagrams[2]);
        let transfer_call_id = invite_c
            .lines()
            .find(|l| l.starts_with("Call-ID:"))
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap()
            .to_string();

        // 4. Target C responds with a failure (e.g. 486 Busy Here)
        let busy_486 = format!(
            "SIP/2.0 486 Busy Here\r\n\
             Via: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-transfer-refer-rollback-001-52\r\n\
             From: <sip:13800138000@example.com>;tag=gw-tag\r\n\
             To: <sip:1002@example.com>;tag=c-tag\r\n\
             Call-ID: {transfer_call_id}\r\n\
             CSeq: 1 INVITE\r\n\
             Content-Length: 0\r\n\r\n"
        );
        let err_datagrams = handle_datagram(
            busy_486.as_bytes(),
            "203.0.113.20:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // Should return 1 datagram: NOTIFY (486 Busy Here) to referrer with Subscription-State: terminated
        assert_eq!(err_datagrams.len(), 1);
        let notify_err = datagram_text(&err_datagrams[0]);
        assert!(notify_err.contains("SIP/2.0 486 Busy Here\r\n"));
        assert!(notify_err.contains("Subscription-State: terminated;reason=noresource\r\n"));

        // Verify media rollback occurred and original targets were restored
        {
            let tx = edge_state.inbound_transactions.get(call_id).unwrap();
            let caller_relay = tx.caller_relay_rtp.as_ref().unwrap();
            let gw_relay = tx.gateway_relay_rtp.as_ref().unwrap();
            assert_eq!(
                edge_state.media_relay.peer_port_for(caller_relay.port),
                Some(gw_relay.port)
            );
            assert_eq!(
                edge_state.media_relay.target_for_port(caller_relay.port),
                Some("198.51.100.20:49200".parse().unwrap())
            );
            assert_eq!(
                edge_state.media_relay.target_for_port(gw_relay.port),
                Some("192.0.2.10:49170".parse().unwrap())
            );
        }
    }

    #[tokio::test]
    async fn test_tcp_stream_framing() {
        use crate::transport::read_frame;

        // Case 1: Complete single message
        let mut buf = b"SIP/2.0 200 OK\r\nContent-Length: 5\r\n\r\nhello".to_vec();
        let frame = read_frame(&mut buf).unwrap();
        assert_eq!(
            frame,
            b"SIP/2.0 200 OK\r\nContent-Length: 5\r\n\r\nhello".to_vec()
        );
        assert!(buf.is_empty());

        // Case 2: Compact length header l
        let mut buf = b"SIP/2.0 200 OK\r\nl: 5\r\n\r\nhello".to_vec();
        let frame = read_frame(&mut buf).unwrap();
        assert_eq!(frame, b"SIP/2.0 200 OK\r\nl: 5\r\n\r\nhello".to_vec());
        assert!(buf.is_empty());

        // Case 3: Partial message (header not complete)
        let mut buf = b"SIP/2.0 200 OK\r\nContent-L".to_vec();
        assert!(read_frame(&mut buf).is_none());
        assert_eq!(buf.len(), 25);

        // Case 4: Header complete but body incomplete
        let mut buf = b"SIP/2.0 200 OK\r\nContent-Length: 10\r\n\r\nbody".to_vec();
        assert!(read_frame(&mut buf).is_none());
        assert_eq!(buf.len(), 42);

        // Feed rest of body
        buf.extend_from_slice(b" rest1");
        let frame = read_frame(&mut buf).unwrap();
        assert_eq!(
            frame,
            b"SIP/2.0 200 OK\r\nContent-Length: 10\r\n\r\nbody rest1".to_vec()
        );
        assert!(buf.is_empty());

        // Case 5: Multiple concatenated messages
        let mut buf = b"SIP/2.0 200 OK\r\nContent-Length: 3\r\n\r\n123SIP/2.0 200 OK\r\nContent-Length: 3\r\n\r\n456".to_vec();
        let frame1 = read_frame(&mut buf).unwrap();
        assert_eq!(
            frame1,
            b"SIP/2.0 200 OK\r\nContent-Length: 3\r\n\r\n123".to_vec()
        );
        let frame2 = read_frame(&mut buf).unwrap();
        assert_eq!(
            frame2,
            b"SIP/2.0 200 OK\r\nContent-Length: 3\r\n\r\n456".to_vec()
        );
        assert!(buf.is_empty());
    }

    #[tokio::test]
    async fn test_tcp_tls_transport_dispatch_and_reuse() {
        use tokio::io::AsyncReadExt;

        let edge_state = Arc::new(state_with_default_route());
        let edge_config = edge_config();

        // 1. Setup local TCP listener
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let listen_addr = listener.local_addr().unwrap();

        // Spawn local test TCP server task
        let (server_tx, mut server_rx) = tokio::sync::mpsc::channel(10);
        tokio::spawn(async move {
            if let Ok((mut stream, _)) = listener.accept().await {
                let mut buf = vec![0u8; 1024];
                while let Ok(n) = stream.read(&mut buf).await {
                    if n == 0 {
                        break;
                    }
                    let _ = server_tx.send(buf[..n].to_vec()).await;
                }
            }
        });

        // Send a mock SIP request targeting this server using TCP transport (Via has SIP/2.0/TCP)
        let request_bytes = format!(
            "INVITE sip:1002@example.com SIP/2.0\r\n\
             Via: SIP/2.0/TCP {listen_addr};branch=z9hG4bK-tcp-001\r\n\
             Content-Length: 0\r\n\r\n"
        )
        .into_bytes();

        let datagram = PendingDatagram::new(listen_addr.to_string(), request_bytes.clone());
        let dummy_udp = UdpSocket::bind("127.0.0.1:0").await.unwrap();

        let res = edge_state
            .send_sip_datagram(datagram, &dummy_udp, &edge_config)
            .await;
        assert!(res.is_ok());

        // Verify message received by the server
        let received = tokio::time::timeout(Duration::from_millis(500), server_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(received, request_bytes);

        // Check that the connection was registered in the connection pool
        {
            assert!(edge_state.tcp_connections.contains_key(&listen_addr));
        }

        // Send a second message to check connection reuse
        let request_bytes2 = format!(
            "INVITE sip:1002@example.com SIP/2.0\r\n\
             Via: SIP/2.0/TCP {listen_addr};branch=z9hG4bK-tcp-002\r\n\
             Content-Length: 0\r\n\r\n"
        )
        .into_bytes();

        let datagram2 = PendingDatagram::new(listen_addr.to_string(), request_bytes2.clone());
        let res2 = edge_state
            .send_sip_datagram(datagram2, &dummy_udp, &edge_config)
            .await;
        assert!(res2.is_ok());

        let received2 = tokio::time::timeout(Duration::from_millis(500), server_rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(received2, request_bytes2);
    }

    #[tokio::test]
    async fn test_dynamic_nonce_verification() {
        let auth = AuthConfig::new(
            "vos-rs",
            "test-nonce",
            HashMap::from([("1001".to_string(), "secret".to_string())]),
        );

        let nonce = auth.generate_dynamic_nonce();
        assert!(auth.verify_dynamic_nonce(&nonce, 300));

        let tampered = format!("{}-wrongsig", nonce.split_once('-').unwrap().0);
        assert!(!auth.verify_dynamic_nonce(&tampered, 300));

        // Use a deterministic older timestamp to test age expiration
        let past_ts = std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            - 10;
        let past_sig = format!(
            "{:x}",
            md5::compute(format!("{}:{}", past_ts, auth.secret_key).as_bytes())
        );
        let past_nonce = format!("{}-{}", past_ts, past_sig);

        assert!(auth.verify_dynamic_nonce(&past_nonce, 15));
        assert!(!auth.verify_dynamic_nonce(&past_nonce, 5));
    }

    #[tokio::test]
    async fn test_nonce_anti_replay_protection() {
        let auth = AuthConfig::new(
            "vos-rs",
            "test-nonce",
            HashMap::from([("1001".to_string(), "secret".to_string())]),
        );

        let cache = std::sync::Mutex::new(HashMap::new());

        let raw_invite = concat!(
            "INVITE sip:13800138000@edge.example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-replay\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:13800138000@example.com>\r\n",
            "Call-ID: replay-test@example.com\r\n",
            "CSeq: 1 INVITE\r\n",
            "Content-Length: 0\r\n\r\n"
        );

        let SipMessage::Request(request) = parse_message(raw_invite.as_bytes()).unwrap() else {
            panic!("expected request");
        };

        let response = crate::auth::digest_response(
            "1001",
            "secret",
            "vos-rs",
            "test-nonce",
            "INVITE",
            "sip:13800138000@edge.example.com",
            Some(("auth", "00000001", "abcdef")),
        );

        let auth_hdr = format!(
            "Digest username=\"1001\", realm=\"vos-rs\", nonce=\"test-nonce\", uri=\"sip:13800138000@edge.example.com\", response=\"{response}\", algorithm=MD5, qop=auth, nc=00000001, cnonce=\"abcdef\""
        );

        let mut request_correct = request.clone();
        request_correct.headers.insert(
            sip_core::HeaderName::new("proxy-authorization").unwrap(),
            sip_core::HeaderValue::new(&auth_hdr),
        );

        // First attempt succeeds
        assert_eq!(
            auth.verify_request(&request_correct, None, Some(&cache))
                .await,
            crate::AuthDecision::Authorized {
                username: "1001".to_string()
            }
        );

        // Replay attempt fails (returns Challenge due to replay block)
        assert_eq!(
            auth.verify_request(&request_correct, None, Some(&cache))
                .await,
            crate::AuthDecision::Challenge
        );
    }

    #[tokio::test]
    async fn test_invite_proxy_auth_flow() {
        let edge_state = state_with_default_route();
        let config = edge_config_with_auth();

        let raw_invite = concat!(
            "INVITE sip:13800138000@edge.example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-invite-1\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:13800138000@example.com>\r\n",
            "Call-ID: invite-auth-test@example.com\r\n",
            "CSeq: 1 INVITE\r\n",
            "Content-Length: 0\r\n\r\n"
        );

        // 1. Initial INVITE should be challenged with 407 Proxy Authentication Required
        let datagrams = handle_datagram(raw_invite.as_bytes(), peer(), &edge_state, &config).await;

        assert_eq!(datagrams.len(), 1);
        let challenge_resp = datagram_text(&datagrams[0]);
        assert!(challenge_resp.starts_with("SIP/2.0 407 Proxy Authentication Required\r\n"));
        assert!(challenge_resp.contains("Proxy-Authenticate: Digest"));

        // Extract nonce from Proxy-Authenticate
        let nonce = challenge_resp
            .lines()
            .find(|l| l.starts_with("Proxy-Authenticate:"))
            .unwrap()
            .split("nonce=\"")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap();

        // 2. Build second INVITE with valid Proxy-Authorization
        let response = crate::auth::digest_response(
            "1001",
            "secret",
            "vos-rs",
            nonce,
            "INVITE",
            "sip:13800138000@edge.example.com",
            Some(("auth", "00000002", "cnonce123")),
        );

        let auth_hdr = format!(
            "Digest username=\"1001\", realm=\"vos-rs\", nonce=\"{nonce}\", uri=\"sip:13800138000@edge.example.com\", response=\"{response}\", algorithm=MD5, qop=auth, nc=00000002, cnonce=\"cnonce123\""
        );

        let raw_invite_auth = format!(
            "INVITE sip:13800138000@edge.example.com SIP/2.0\r\n\
             Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-invite-2\r\n\
             From: <sip:1001@example.com>;tag=from-tag\r\n\
             To: <sip:13800138000@edge.example.com>\r\n\
             Call-ID: invite-auth-test@example.com\r\n\
             CSeq: 2 INVITE\r\n\
             Proxy-Authorization: {auth_hdr}\r\n\
             Content-Length: 0\r\n\r\n"
        );

        let datagrams_auth =
            handle_datagram(raw_invite_auth.as_bytes(), peer(), &edge_state, &config).await;

        // Second INVITE should bypass challenge and be routed (returning 100 Trying and forwarding INVITE)
        assert_eq!(datagrams_auth.len(), 2);
        let resp_100 = datagram_text(&datagrams_auth[0]);
        assert!(resp_100.starts_with("SIP/2.0 100 Trying\r\n"));

        let forwarded_invite = datagram_text(&datagrams_auth[1]);
        assert!(forwarded_invite
            .starts_with("INVITE sip:13800138000@gw1.example.com:5060;transport=udp SIP/2.0\r\n"));
    }

    #[tokio::test]
    async fn test_invite_gateway_bypass_auth() {
        let edge_state = state_with_default_route();

        // Add peer IP to test gateways bypass list
        edge_state
            .test_gateways
            .lock()
            .unwrap()
            .push("192.0.2.10".to_string());

        let config = edge_config_with_auth();

        let raw_invite = concat!(
            "INVITE sip:13800138000@edge.example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-invite-gw\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:13800138000@example.com>\r\n",
            "Call-ID: invite-gw-bypass-test@example.com\r\n",
            "CSeq: 1 INVITE\r\n",
            "Content-Length: 0\r\n\r\n"
        );

        // INVITE from gateway IP should bypass challenge completely and be routed
        let datagrams = handle_datagram(raw_invite.as_bytes(), peer(), &edge_state, &config).await;

        assert_eq!(datagrams.len(), 2);
        let resp_100 = datagram_text(&datagrams[0]);
        assert!(resp_100.starts_with("SIP/2.0 100 Trying\r\n"));
    }

    #[tokio::test]
    async fn test_session_timer_response_forwarding() {
        let raw_resp = concat!(
            "SIP/2.0 200 OK\r\n",
            "Via: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-vosrs\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:13800138000@example.com>;tag=gw-tag\r\n",
            "Call-ID: test-session-expires-forwarding@example.com\r\n",
            "CSeq: 1 INVITE\r\n",
            "Session-Expires: 600;refresher=uac\r\n",
            "Min-SE: 90\r\n",
            "Supported: timer\r\n",
            "Content-Length: 0\r\n\r\n"
        );

        let SipMessage::Response(resp) = parse_message(raw_resp.as_bytes()).unwrap() else {
            panic!("expected response");
        };

        let vias = vec!["SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-inbound".to_string()];
        let route_set = vec![];
        let forwarded =
            response::forward_response_to_inbound_with_body(&resp, &vias, &route_set, &[]);
        let forwarded_str = String::from_utf8(forwarded).unwrap();

        assert!(forwarded_str.contains("Session-Expires: 600;refresher=uac\r\n"));
        assert!(forwarded_str.contains("Min-SE: 90\r\n"));
        assert!(forwarded_str.contains("Supported: timer\r\n"));
    }

    #[tokio::test]
    async fn test_active_session_refresh_triggering() {
        let edge_state = Arc::new(state_with_default_route());
        let call_id = "test-active-refresh-trigger@example.com";

        // Setup a tracked established call
        let raw_invite = concat!(
            "INVITE sip:13800138000@example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-refresh-invite\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:13800138000@example.com>\r\n",
            "Call-ID: test-active-refresh-trigger@example.com\r\n",
            "CSeq: 1 INVITE\r\n",
            "Content-Length: 0\r\n\r\n"
        );

        // 1. Receive INVITE
        let _ = handle_datagram(raw_invite.as_bytes(), peer(), &edge_state, &edge_config()).await;

        // 2. Setup refresher = Some("uac") and last_session_refresh = 10 seconds ago
        {
            // DashMap get_mut returns RefMut
            let mut tx = edge_state.inbound_transactions.get_mut(call_id).unwrap();
            tx.session_expires = Some(10); // Expires in 10s, refresh at 5s
            tx.session_refresher = Some("uac".to_string());
            tx.last_session_refresh =
                Some(std::time::Instant::now() - std::time::Duration::from_secs(6));
            tx.callee_contact =
                Some(SipUri::from_str("sip:13800138000@gw-real-ip.com:5060").unwrap());
        }

        // 3. Setup a mock socket to capture outbound refresh UPDATE packet
        let tokio_socket = Arc::new(tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let port = tokio_socket.local_addr().unwrap().port();

        // Spawn watchdog with short interval
        let mut config = edge_config();
        config.advertised_addr = format!("127.0.0.1:{}", port);

        spawn_session_timer_watchdog(Arc::clone(&edge_state), Arc::clone(&tokio_socket), config);

        // Wait a bit for watchdog loop to tick
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;

        // Verify that the call's last_session_refresh was reset (throttled)
        {
            let tx = edge_state.inbound_transactions.get(call_id).unwrap();
            let elapsed = tx.last_session_refresh.unwrap().elapsed().as_secs();
            assert!(elapsed < 2);
        }
    }

    #[tokio::test]
    async fn test_self_refresh_response_drop() {
        let edge_state = state_with_default_route();
        let call_id = "test-self-refresh-response-drop@example.com";

        // Setup a tracked established call
        let raw_invite = concat!(
            "INVITE sip:13800138000@example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-drop-invite\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:13800138000@example.com>\r\n",
            "Call-ID: test-self-refresh-response-drop@example.com\r\n",
            "CSeq: 1 INVITE\r\n",
            "Content-Length: 0\r\n\r\n"
        );

        let _ = handle_datagram(raw_invite.as_bytes(), peer(), &edge_state, &edge_config()).await;

        // Setup mock session timer state
        {
            // DashMap get_mut returns RefMut
            let mut tx = edge_state.inbound_transactions.get_mut(call_id).unwrap();
            tx.session_expires = Some(600);
            tx.session_refresher = Some("uac".to_string());
            tx.last_session_refresh =
                Some(std::time::Instant::now() - std::time::Duration::from_secs(400));
        }

        // Send a 200 OK response corresponding to our self-generated refresh request (Via contains branch=z9hG4bK-refresh-)
        let raw_200 = format!(
            "SIP/2.0 200 OK\r\n\
             Via: SIP/2.0/UDP 127.0.0.1:5060;branch=z9hG4bK-refresh-true-2\r\n\
             From: <sip:1001@example.com>;tag=from-tag\r\n\
             To: <sip:13800138000@example.com>;tag=gw-tag\r\n\
             Call-ID: {call_id}\r\n\
             CSeq: 2 UPDATE\r\n\
             Content-Length: 0\r\n\r\n",
            call_id = call_id
        );
        let datagrams = handle_datagram(
            raw_200.as_bytes(),
            "203.0.113.20:5060".parse().unwrap(),
            &edge_state,
            &edge_config(),
        )
        .await;

        // The response must be consumed (empty datagram list returned)
        assert!(datagrams.is_empty());

        // The last_session_refresh must be reset to now (elapsed < 2 seconds)
        {
            let tx = edge_state.inbound_transactions.get(call_id).unwrap();
            let elapsed = tx.last_session_refresh.unwrap().elapsed().as_secs();
            assert!(elapsed < 2);
        }
    }

    // ── Topology Hiding (Scheme 6) ────────────────────────────────────────────

    /// Verifies that outbound INVITEs carry a new (external) Call-ID distinct
    /// from the inbound (internal) one — the core of topology hiding.
    #[tokio::test]
    async fn test_topology_hiding_call_id_rewritten_on_outbound_invite() {
        let routes = RouteTable::new(vec![Route::new(
            "gw1",
            "".to_string(),
            100,
            RouteTarget::new("gw1".to_string(), "203.0.113.20".to_string(), Some(5060)),
        )]);
        let edge_state = Arc::new(EdgeState::new(CallManager::new(routes)));
        let internal_call_id = "internal-call-id-topo-test@example.com";
        let invite = format!(
            concat!(
                "INVITE sip:13800138000@example.com SIP/2.0\r\n",
                "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-topo\r\n",
                "Max-Forwards: 70\r\n",
                "From: <sip:1001@example.com>;tag=from-tag\r\n",
                "To: <sip:13800138000@example.com>\r\n",
                "Call-ID: {call_id}\r\n",
                "CSeq: 1 INVITE\r\n",
                "Content-Length: 0\r\n",
                "\r\n"
            ),
            call_id = internal_call_id
        );

        let datagrams =
            handle_datagram(invite.as_bytes(), peer(), &edge_state, &edge_config()).await;
        assert_eq!(datagrams.len(), 2);
        let outbound_text = datagram_text(&datagrams[1]);

        assert!(
            !outbound_text.contains(internal_call_id),
            "outbound INVITE should not expose the internal Call-ID, got:\n{}",
            outbound_text
        );
        let call_id_count = outbound_text
            .lines()
            .filter(|l| l.to_ascii_lowercase().starts_with("call-id:"))
            .count();
        assert_eq!(
            call_id_count, 1,
            "outbound INVITE must have exactly one Call-ID header"
        );

        let external_call_id = outbound_text
            .lines()
            .find(|l| l.to_ascii_lowercase().starts_with("call-id:"))
            .and_then(|l| l.split_once(':').map(|(_, v)| v.trim().to_string()))
            .expect("Call-ID header not found in outbound INVITE");

        assert_ne!(external_call_id, internal_call_id);
        assert_eq!(
            edge_state
                .get_internal_call_id(&external_call_id)
                .as_deref(),
            Some(internal_call_id)
        );
        assert_eq!(
            edge_state.get_external_call_id(internal_call_id).as_deref(),
            Some(external_call_id.as_str())
        );
    }

    /// Verifies that a 200 OK from the gateway (with external Call-ID) is
    /// forwarded to the caller using the original internal Call-ID.
    #[tokio::test]
    async fn test_topology_hiding_gateway_200_forwarded_with_internal_call_id() {
        let routes = RouteTable::new(vec![Route::new(
            "gw1",
            "".to_string(),
            100,
            RouteTarget::new("gw1".to_string(), "203.0.113.20".to_string(), Some(5060)),
        )]);
        let edge_state = Arc::new(EdgeState::new(CallManager::new(routes)));
        let internal_call_id = "topo-gw-200-test@example.com";

        let invite = format!(
            concat!(
                "INVITE sip:13800138000@example.com SIP/2.0\r\n",
                "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-topo-gw\r\n",
                "Max-Forwards: 70\r\n",
                "From: <sip:1001@example.com>;tag=caller-tag\r\n",
                "To: <sip:13800138000@example.com>\r\n",
                "Call-ID: {cid}\r\n",
                "CSeq: 1 INVITE\r\n",
                "Content-Length: 0\r\n",
                "\r\n"
            ),
            cid = internal_call_id
        );
        let dg = handle_datagram(invite.as_bytes(), peer(), &edge_state, &edge_config()).await;
        assert_eq!(dg.len(), 2);

        let out_text = datagram_text(&dg[1]);
        let external_call_id = out_text
            .lines()
            .find(|l| l.to_ascii_lowercase().starts_with("call-id:"))
            .and_then(|l| l.split_once(':').map(|(_, v)| v.trim().to_string()))
            .expect("outbound INVITE has no Call-ID");

        let gw_200 = format!(
            concat!(
                "SIP/2.0 200 OK\r\n",
                "Via: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-vosrs\r\n",
                "From: <sip:1001@example.com>;tag=caller-tag\r\n",
                "To: <sip:13800138000@example.com>;tag=gw-tag\r\n",
                "Call-ID: {cid}\r\n",
                "CSeq: 1 INVITE\r\n",
                "Content-Length: 0\r\n",
                "\r\n"
            ),
            cid = external_call_id
        );
        let gw_peer: SocketAddr = "203.0.113.20:5060".parse().unwrap();
        let resp_dg =
            handle_datagram(gw_200.as_bytes(), gw_peer, &edge_state, &edge_config()).await;

        assert_eq!(resp_dg.len(), 1, "200 OK should be forwarded to the caller");
        let forwarded = datagram_text(&resp_dg[0]);

        assert!(
            forwarded.contains(internal_call_id),
            "forwarded 200 OK should contain the internal Call-ID, got:\n{}",
            forwarded
        );
        assert!(
            !forwarded.contains(external_call_id.as_str()),
            "forwarded 200 OK must not expose the external Call-ID, got:\n{}",
            forwarded
        );
    }
}
