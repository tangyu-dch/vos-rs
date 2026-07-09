use crate::handle_datagram;
use crate::nats_cdr::NatsCdrPublisher;
use crate::net::create_tls_connector;
use crate::sbc;
use crate::sip::registrar::RegistrationStore;
use crate::{
    config::EdgeConfig,
    media::{MediaConfig, MediaRelayState},
    net::{handle_stream_connection, SipStream, Transport},
    sip::{
        dialog, transaction, ClientTransactionKey, DialogValidationError,
        RequestTransactionKey,
    },
};
use call_core::{CallManager, GatewayHealthTracker};
use cdr_core::PostgresCdrStore;
use dashmap::DashMap;
use rustls_pki_types::ServerName;
use sdp_core::RtpEndpoint;
use sip_core::{parse_message, Method, SipRequest, SipUri};
use std::{collections::HashMap, net::SocketAddr, str::FromStr, sync::Arc, time::Instant};
use tokio::net::{TcpStream, UdpSocket};
use tracing::{debug, error, warn};

#[derive(Debug, Clone)]
pub(crate) struct PendingDatagram {
    pub target: String,
    pub bytes: Vec<u8>,
}

impl PendingDatagram {
    pub fn new(target: impl Into<String>, bytes: Vec<u8>) -> Self {
        Self {
            target: target.into(),
            bytes,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct CdrSinks {
    pub(crate) postgres: Option<PostgresCdrStore>,
    pub(crate) nats: Option<NatsCdrPublisher>,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub(crate) struct ReferSubscription {
    pub(crate) refer_to: String,
    pub(crate) from_header: String,
    pub(crate) to_header: String,
    pub(crate) notify_cseq: u32,
    pub(crate) transfer_call_id: String,
    pub(crate) referrer_peer: String,
    pub(crate) refer_cseq: u32,
    pub(crate) target_relay_port: Option<u16>,
    pub(crate) transferee_relay_port: Option<u16>,
}

#[derive(Debug, Clone)]
pub(crate) struct InboundTransaction {
    pub(crate) peer: String,
    pub(crate) outbound_peer: Option<String>,
    pub(crate) vias: Vec<String>,
    pub(crate) outbound_uri: SipUri,
    pub(crate) inbound_from_tag: Option<String>,
    pub(crate) inbound_to_tag: Option<String>,
    pub(crate) last_inbound_cseq: Option<u32>,
    pub(crate) last_outbound_cseq: Option<u32>,
    pub(crate) caller_rtp: Option<RtpEndpoint>,
    pub(crate) gateway_relay_rtp: Option<RtpEndpoint>,
    pub(crate) gateway_rtp: Option<RtpEndpoint>,
    pub(crate) caller_relay_rtp: Option<RtpEndpoint>,
    pub(crate) original_request: Option<Arc<SipRequest>>,
    pub(crate) inbound_route_set: Vec<String>,
    pub(crate) outbound_route_set: Vec<String>,
    pub(crate) caller_contact: Option<SipUri>,
    pub(crate) callee_contact: Option<SipUri>,
    /// Negotiated Session-Expires value in seconds (from 200 OK)
    pub(crate) session_expires: Option<u32>,
    /// Who is responsible for refresh: "uac" (caller) or "uas" (gateway)
    pub(crate) session_refresher: Option<String>,
    /// Timestamp of the last session refresh (Re-INVITE / UPDATE received)
    pub(crate) last_session_refresh: Option<Instant>,
    /// RSeq counter for 100rel provisional responses sent toward the caller
    pub(crate) prack_rseq: u32,
    /// Whether the gateway negotiated 100rel (Require: 100rel seen in a 1xx)
    pub(crate) gateway_100rel: bool,
    /// Information about any active REFER transfer subscription handled locally
    pub(crate) refer_subscription: Option<ReferSubscription>,
    pub(crate) transfer_from_header: Option<String>,
    pub(crate) transfer_to_header: Option<String>,
    pub(crate) transfer_call_id: Option<String>,
    pub(crate) transfer_contact: Option<SipUri>,
    pub(crate) transfer_peer: Option<String>,
    pub(crate) transferee_is_caller: bool,
    pub(crate) callee_behind_nat: bool,
}

impl InboundTransaction {
    pub(crate) fn validate_in_dialog_request(
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

    pub(crate) fn dialog_leg_for_peer(&self, peer: SocketAddr) -> Option<DialogLeg> {
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
pub(crate) enum DialogLeg {
    Caller,
    Gateway,
}

pub(crate) fn extract_uri_from_contact(contact: &str) -> Option<SipUri> {
    let contact = contact.trim();
    let uri_str = if let Some(start) = contact.find('<') {
        let end = contact.find('>')?;
        &contact[start + 1..end]
    } else {
        contact.split(';').next()?
    };
    SipUri::from_str(uri_str.trim()).ok()
}

pub(crate) fn sip_uri_from_peer(peer: &str) -> SipUri {
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

pub(crate) fn parse_target_addr_from_route(route: &str) -> Option<String> {
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

#[derive(Debug)]
pub(crate) struct EdgeState {
    pub(crate) call_manager: std::sync::Arc<CallManager>,
    pub(crate) gateway_health: std::sync::Mutex<GatewayHealthTracker>,
    pub(crate) inbound_transactions: dashmap::DashMap<String, InboundTransaction>,
    pub(crate) media_relay: MediaRelayState,
    pub(crate) registrar: tokio::sync::Mutex<RegistrationStore>,
    pub(crate) db_store: Option<PostgresCdrStore>,
    pub(crate) client_transactions:
        dashmap::DashMap<ClientTransactionKey, tokio::sync::oneshot::Sender<()>>,
    pub(crate) draining: std::sync::atomic::AtomicBool,
    pub(crate) refer_transfers: dashmap::DashMap<String, String>,
    pub(crate) bridged_transfers: dashmap::DashMap<String, String>,
    pub(crate) server_transactions: dashmap::DashMap<
        RequestTransactionKey,
        tokio::sync::mpsc::Sender<transaction::ServerTransactionEvent>,
    >,
    pub(crate) socket: std::sync::OnceLock<Arc<UdpSocket>>,
    pub(crate) test_request_cache: dashmap::DashMap<RequestTransactionKey, Vec<PendingDatagram>>,
    pub(crate) nonce_replay_cache: DashMap<String, u64>,
    pub(crate) tcp_connections: dashmap::DashMap<SocketAddr, tokio::sync::mpsc::Sender<Vec<u8>>>,
    pub(crate) sbc_engine: sbc::SbcEngine,
    pub(crate) external_to_internal_call_ids: dashmap::DashMap<String, String>,
    pub(crate) internal_to_external_call_ids: dashmap::DashMap<String, String>,
    pub(crate) gateway_cache: std::sync::RwLock<HashMap<String, (bool, std::time::Instant)>>,
    #[cfg(test)]
    pub(crate) test_gateways: std::sync::Mutex<Vec<String>>,
}

impl EdgeState {
    #[cfg(test)]
    #[cfg(test)]
    pub(crate) fn new(call_manager: CallManager) -> Self {
        Self::with_media_relay_and_db(call_manager, MediaRelayState::new(), None)
    }

    pub(crate) fn with_media_relay_and_db(
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
            socket: std::sync::OnceLock::new(),
            test_request_cache: dashmap::DashMap::new(),
            nonce_replay_cache: DashMap::new(),
            tcp_connections: dashmap::DashMap::new(),
            sbc_engine,
            external_to_internal_call_ids: dashmap::DashMap::new(),
            internal_to_external_call_ids: dashmap::DashMap::new(),
            gateway_cache: std::sync::RwLock::new(HashMap::new()),
            #[cfg(test)]
            test_gateways: std::sync::Mutex::new(Vec::new()),
        }
    }

    #[cfg(test)]
    #[cfg(test)]
    pub(crate) fn with_config(call_manager: CallManager, config: &EdgeConfig) -> Self {
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
            socket: std::sync::OnceLock::new(),
            test_request_cache: dashmap::DashMap::new(),
            nonce_replay_cache: DashMap::new(),
            tcp_connections: dashmap::DashMap::new(),
            sbc_engine,
            external_to_internal_call_ids: dashmap::DashMap::new(),
            internal_to_external_call_ids: dashmap::DashMap::new(),
            gateway_cache: std::sync::RwLock::new(HashMap::new()),
            test_gateways: std::sync::Mutex::new(Vec::new()),
        }
    }

    pub(crate) fn get_internal_call_id(&self, external_call_id: &str) -> Option<String> {
        self.external_to_internal_call_ids
            .get(external_call_id)
            .map(|r| r.clone())
    }

    pub(crate) fn get_external_call_id(&self, internal_call_id: &str) -> Option<String> {
        self.internal_to_external_call_ids
            .get(internal_call_id)
            .map(|r| r.clone())
    }

    pub(crate) fn register_call_id_mapping(&self, internal_call_id: &str, external_call_id: &str) {
        self.internal_to_external_call_ids
            .insert(internal_call_id.to_string(), external_call_id.to_string());
        self.external_to_internal_call_ids
            .insert(external_call_id.to_string(), internal_call_id.to_string());
    }

    pub(crate) fn set_socket(&self, socket: Arc<UdpSocket>) {
        let _ = self.socket.set(socket);
    }

    pub(crate) fn get_socket(&self) -> Option<Arc<UdpSocket>> {
        self.socket.get().cloned()
    }

    pub(crate) fn register_tcp_connection(
        &self,
        addr: SocketAddr,
        tx: tokio::sync::mpsc::Sender<Vec<u8>>,
    ) {
        self.tcp_connections.insert(addr, tx);
    }

    pub(crate) fn get_tcp_connection(
        &self,
        addr: &SocketAddr,
    ) -> Option<tokio::sync::mpsc::Sender<Vec<u8>>> {
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

    pub(crate) async fn send_sip_datagram(
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

    pub(crate) fn register_server_transaction(
        &self,
        key: RequestTransactionKey,
        tx: tokio::sync::mpsc::Sender<transaction::ServerTransactionEvent>,
    ) {
        self.server_transactions.insert(key, tx);
    }

    pub(crate) fn get_server_transaction(
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

    pub(crate) async fn is_peer_gateway(&self, peer: SocketAddr) -> bool {
        let peer_ip = peer.ip().to_string();

        #[cfg(test)]
        {
            if self.test_gateways.lock().unwrap().contains(&peer_ip) {
                return true;
            }
        }

        // Check cached result first (TTL 60s)
        {
            let cache = self.gateway_cache.read().unwrap();
            if let Some(entry) = cache.get(&peer_ip) {
                if entry.1.elapsed().as_secs() < 60 {
                    return entry.0;
                }
            }
        }

        let result = if let Some(ref db) = self.db_store {
            if let Ok(gateways) = db.load_gateways().await {
                gateways
                    .iter()
                    .any(|(_, host, _, _, _, _, _, _)| host == &peer_ip)
            } else {
                false
            }
        } else {
            false
        };

        // Update cache
        {
            let mut cache = self.gateway_cache.write().unwrap();
            cache.insert(peer_ip, (result, std::time::Instant::now()));
        }

        result
    }

    pub(crate) fn cancel_client_transaction(&self, key: &ClientTransactionKey) {
        if let Some((_, cancel_tx)) = self.client_transactions.remove(key) {
            let _ = cancel_tx.send(());
        }
    }

    pub(crate) fn remember_inbound_invite(
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
                original_request: Some(Arc::new(request.clone())),
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

    pub(crate) fn remember_gateway_media(
        &self,
        call_id: &str,
        gateway_rtp: Option<RtpEndpoint>,
        caller_relay_rtp: RtpEndpoint,
        media_config: &MediaConfig,
    ) {
        let gateway_relay_port = self
            .inbound_transactions
            .get(call_id)
            .and_then(|t| t.gateway_relay_rtp.as_ref().map(|ep| ep.port));

        if let Some(mut transaction) = self.inbound_transactions.get_mut(call_id) {
            transaction.gateway_rtp = gateway_rtp;
            if let Some(gw_port) = gateway_relay_port {
                self.media_relay.pair_ports(gw_port, caller_relay_rtp.port);
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

    pub(crate) fn remember_inbound_to_tag(&self, call_id: &str, response: &sip_core::SipResponse) {
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

    pub(crate) fn clear_media_targets(&self, transaction: &InboundTransaction) {
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
