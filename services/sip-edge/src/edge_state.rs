use crate::cluster::{flow_key, ClusterEgress, FlowRecord};
use crate::handle_datagram;
use crate::media::relay::MediaRelayMetrics;
use crate::net::create_tls_connector;
use crate::sbc;
use crate::sip::registrar::RegistrationStore;
use crate::{
    config::EdgeConfig,
    media::{MediaConfig, MediaRelayState},
    net::{handle_stream_connection, SipStream, Transport},
    sip::{
        dialog, transaction, ClientTransactionKey, DialogValidationError, InviteAckKey,
        RequestTransactionKey,
    },
};
use call_core::{CallId, CallManager, GatewayHealthTracker};
use cdr_core::PostgresCdrStore;
use dashmap::DashMap;
use rustls_pki_types::ServerName;
use sdp_core::RtpEndpoint;
use sip_core::{parse_message, Method, SipRequest, SipUri};
use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    str::FromStr,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::net::{TcpStream, UdpSocket};
use tracing::{debug, error, info, warn};

#[derive(Debug, Clone)]
pub(crate) struct PendingDatagram {
    pub target: String,
    pub bytes: Vec<u8>,
    invite_response: Option<InviteResponseMetadata>,
}

impl PendingDatagram {
    pub fn new(target: impl Into<String>, bytes: Vec<u8>) -> Self {
        Self {
            target: target.into(),
            bytes,
            invite_response: None,
        }
    }

    pub(crate) fn with_invite_response_order(
        mut self,
        order: Arc<tokio::sync::Mutex<InviteResponseOrder>>,
        cseq: Option<u32>,
        status_code: u16,
    ) -> Self {
        self.invite_response = Some(InviteResponseMetadata {
            order,
            cseq,
            status_code,
        });
        self
    }
}

#[derive(Debug, Clone)]
struct InviteResponseMetadata {
    order: Arc<tokio::sync::Mutex<InviteResponseOrder>>,
    cseq: Option<u32>,
    status_code: u16,
}

#[derive(Debug, Clone)]
struct CachedRegistrationLookup {
    contact: Option<crate::sip::registrar::RegistrationContact>,
    expires_at: Instant,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) struct IvrMenu {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) welcome_prompt: String,
    pub(crate) timeout_secs: i32,
    pub(crate) actions: HashMap<String, IvrAction>,
}

#[derive(Debug, Clone)]
pub(crate) struct IvrAction {
    pub(crate) action_type: String,
    pub(crate) action_target: String,
    pub(crate) waiting_prompt: Option<String>,
    pub(crate) webhook_method: Option<String>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) struct OutboundRegState {
    pub(crate) gateway_id: String,
    pub(crate) host: String,
    pub(crate) port: Option<u16>,
    pub(crate) transport: String,
    pub(crate) username: String,
    pub(crate) password: String,
    pub(crate) call_id: String,
    pub(crate) cseq: u32,
    pub(crate) from_tag: String,
    pub(crate) expires: u32,
    pub(crate) last_reg_sent: Option<std::time::Instant>,
    pub(crate) last_reg_success: Option<std::time::Instant>,
    pub(crate) challenge: Option<HashMap<String, String>>,
}

#[derive(Debug, Default)]
pub(crate) struct GatewayIdentityCache {
    exact_endpoints: HashMap<SocketAddr, Option<String>>,
    unique_ips: HashMap<IpAddr, Option<String>>,
    trunk_targets: HashMap<String, String>,
}

impl GatewayIdentityCache {
    fn replace(&mut self, gateways: impl IntoIterator<Item = (String, u16, String)>) {
        self.exact_endpoints.clear();
        self.unique_ips.clear();
        self.trunk_targets.clear();
        for (host, port, trunk_id) in gateways {
            self.trunk_targets
                .insert(trunk_id.clone(), format_gateway_target(&host, port));
            let Some(ip) = parse_normalized_ip(&host) else {
                continue;
            };
            merge_gateway_identity(
                &mut self.exact_endpoints,
                SocketAddr::new(ip, port),
                &trunk_id,
            );
            merge_gateway_identity(&mut self.unique_ips, ip, &trunk_id);
        }
    }

    fn identify(&self, peer: SocketAddr) -> Option<String> {
        let peer = normalize_socket_addr(peer);
        self.exact_endpoints
            .get(&peer)
            .cloned()
            .flatten()
            .or_else(|| self.unique_ips.get(&peer.ip()).cloned().flatten())
    }
}

fn format_gateway_target(host: &str, port: u16) -> String {
    let host = host.trim();
    if host.starts_with('[') || !host.contains(':') {
        format!("{host}:{port}")
    } else {
        format!("[{host}]:{port}")
    }
}

fn merge_gateway_identity<K>(identities: &mut HashMap<K, Option<String>>, key: K, trunk_id: &str)
where
    K: std::hash::Hash + Eq,
{
    identities
        .entry(key)
        .and_modify(|current| {
            if current.as_deref() != Some(trunk_id) {
                *current = None;
            }
        })
        .or_insert_with(|| Some(trunk_id.to_string()));
}

fn parse_normalized_ip(host: &str) -> Option<IpAddr> {
    let host = host.trim();
    let host = host
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(host);
    host.parse().ok().map(normalize_ip)
}

fn normalize_socket_addr(address: SocketAddr) -> SocketAddr {
    SocketAddr::new(normalize_ip(address.ip()), address.port())
}

fn normalize_ip(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(ipv6) => ipv6
            .to_ipv4_mapped()
            .map(IpAddr::V4)
            .unwrap_or(IpAddr::V6(ipv6)),
        ipv4 => ipv4,
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct RedisBalanceCheck {
    pub(crate) has_balance: bool,
    pub(crate) balance: f64,
    pub(crate) billing_interval_secs: u32,
    pub(crate) price_per_interval: f64,
}

type RedisBillingPipelineResult = (
    Option<f64>,
    Vec<Option<u32>>,
    Vec<Option<f64>>,
    Vec<Option<f64>>,
);

const POSITIVE_REGISTRATION_CACHE_TTL: Duration = Duration::from_secs(5);
const NEGATIVE_REGISTRATION_CACHE_TTL: Duration = Duration::from_secs(1);
const MAX_REGISTRATION_CACHE_ENTRIES: usize = 10_000;

#[derive(Debug, Clone, Default)]
pub(crate) struct CdrSinks {
    pub(crate) postgres: Option<PostgresCdrStore>,
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
    pub(crate) active_forks: Vec<(String, String)>,
    pub(crate) max_duration_secs: Option<u32>,
    pub(crate) established_at: Option<std::time::Instant>,
    /// Serializes INVITE responses for this dialog and rejects late provisional responses.
    pub(crate) invite_response_order: Arc<tokio::sync::Mutex<InviteResponseOrder>>,
}

#[derive(Debug, Default)]
pub(crate) struct InviteResponseOrder {
    pub(crate) cseq: Option<u32>,
    pub(crate) final_response_seen: bool,
    pub(crate) final_response_send_started: bool,
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
                std::net::IpAddr::V4(ip) => ip.to_string().into(),
                std::net::IpAddr::V6(ip) => format!("[{ip}]").into(),
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
                .to_string()
                .into(),
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

#[derive(Debug, Clone)]
pub(crate) struct AccessIpRule {
    pub(crate) trunk_id: String,
    pub(crate) network: sbc::IpNet,
    pub(crate) source_port: Option<u16>,
    pub(crate) transport: String,
}

#[derive(Clone)]
pub(crate) struct ParkedCall {
    pub(crate) invite_request: sip_core::SipRequest,
    pub(crate) peer_addr: std::net::SocketAddr,
    pub(crate) caller_relay_port: u16,
    pub(crate) created_at: std::time::Instant,
}

pub(crate) struct EdgeState {
    pub(crate) call_manager: std::sync::Arc<CallManager>,
    pub(crate) gateway_health: GatewayHealthTracker,
    pub(crate) inbound_transactions: dashmap::DashMap<String, InboundTransaction>,
    pub(crate) media_relay: MediaRelayState,
    pub(crate) registrar: tokio::sync::RwLock<RegistrationStore>,
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
    pub(crate) sbc_rate_limit_enabled: bool,
    pub(crate) external_to_internal_call_ids: dashmap::DashMap<String, String>,
    pub(crate) internal_to_external_call_ids: dashmap::DashMap<String, String>,
    pub(crate) gateway_cache: std::sync::RwLock<GatewayIdentityCache>,
    pub(crate) access_trunk_auth_modes: std::sync::RwLock<HashMap<String, String>>,
    pub(crate) access_username_to_trunk_id: std::sync::RwLock<HashMap<String, String>>,
    pub(crate) trunk_billing_accounts: std::sync::RwLock<HashMap<String, String>>,
    pub(crate) did_destinations: std::sync::RwLock<HashMap<String, cdr_core::DidDestination>>,
    pub(crate) extension_groups: std::sync::RwLock<HashMap<String, Vec<String>>>,
    pub(crate) ivr_menus: std::sync::RwLock<HashMap<String, IvrMenu>>,
    pub(crate) outbound_registrations: dashmap::DashMap<String, OutboundRegState>,
    access_ip_rules: std::sync::RwLock<Vec<AccessIpRule>>,
    registered_access_users: std::sync::RwLock<Vec<String>>,
    /// 按用户名跟踪活跃并发通话数，O(1) 替代 O(n) iter 扫描
    pub(crate) user_concurrency: dashmap::DashMap<String, u32>,
    pub(crate) anti_fraud_rules: std::sync::RwLock<Vec<cdr_core::AntiFraudRule>>,
    pub(crate) media_metrics_log: bool,
    pub(crate) billing_settlement_enabled: bool,
    pub(crate) parked_calls: std::sync::Arc<dashmap::DashMap<String, ParkedCall>>,
    pub(crate) nats_client: std::sync::OnceLock<async_nats::Client>,
    pub(crate) gateway_health_persistence_enabled: bool,
    /// Active gateway OPTIONS probes keyed by their SIP Call-ID.
    pub(crate) gateway_probes: dashmap::DashMap<String, String>,
    /// Redis 自动重连连接，用于集群状态与呼叫热路径缓存。
    pub(crate) redis_conn: std::sync::OnceLock<redis::aio::ConnectionManager>,
    registration_sync: std::sync::OnceLock<crate::cluster::RegistrationSyncSender>,
    cluster_egress: std::sync::OnceLock<ClusterEgress>,
    registration_lookup_cache: dashmap::DashMap<String, CachedRegistrationLookup>,
    registration_lookup_locks: dashmap::DashMap<String, Arc<tokio::sync::Mutex<()>>>,
    #[cfg(test)]
    pub(crate) test_gateways: std::sync::Mutex<Vec<String>>,
    pub(crate) sipflow_enabled: std::sync::atomic::AtomicBool,
    pub(crate) sipflow_whitelist: std::sync::RwLock<String>,
    pub(crate) sipflow_retention_days: std::sync::atomic::AtomicI32,
    pub(crate) sip_flow_tx: std::sync::OnceLock<tokio::sync::mpsc::Sender<cdr_core::SipFlowRecord>>,
    pub(crate) call_caller_addrs: dashmap::DashMap<String, std::net::SocketAddr>,
    pub(crate) matched_call_ids: dashmap::DashMap<String, std::time::Instant>,
    pub(crate) self_weak: std::sync::OnceLock<std::sync::Weak<EdgeState>>,
}

impl EdgeState {
    #[cfg(test)]
    #[cfg(test)]
    pub(crate) fn new(call_manager: CallManager) -> Self {
        Self::with_media_relay_and_db(
            call_manager,
            MediaRelayState::new(),
            None,
            &EdgeConfig::default(),
        )
    }

    pub(crate) fn with_media_relay_and_db(
        call_manager: CallManager,
        media_relay: MediaRelayState,
        db_store: Option<PostgresCdrStore>,
        config: &EdgeConfig,
    ) -> Self {
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
            gateway_health: GatewayHealthTracker::new(call_core::HealthThresholds::default()),
            inbound_transactions: dashmap::DashMap::new(),
            media_relay,
            registrar: tokio::sync::RwLock::new(RegistrationStore::new()),
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
            sbc_rate_limit_enabled: config.sbc_rate_limit_enabled,
            external_to_internal_call_ids: dashmap::DashMap::new(),
            internal_to_external_call_ids: dashmap::DashMap::new(),
            gateway_cache: std::sync::RwLock::new(GatewayIdentityCache::default()),
            access_trunk_auth_modes: std::sync::RwLock::new(HashMap::new()),
            access_username_to_trunk_id: std::sync::RwLock::new(HashMap::new()),
            trunk_billing_accounts: std::sync::RwLock::new(HashMap::new()),
            did_destinations: std::sync::RwLock::new(HashMap::new()),
            extension_groups: std::sync::RwLock::new(HashMap::new()),
            ivr_menus: std::sync::RwLock::new(HashMap::new()),
            outbound_registrations: dashmap::DashMap::new(),
            access_ip_rules: std::sync::RwLock::new(Vec::new()),
            registered_access_users: std::sync::RwLock::new(Vec::new()),
            user_concurrency: dashmap::DashMap::new(),
            anti_fraud_rules: std::sync::RwLock::new(Vec::new()),
            media_metrics_log: config.media_metrics_log,
            billing_settlement_enabled: config.billing_settlement_enabled,
            gateway_health_persistence_enabled: config.gateway_health_checks_enabled,
            gateway_probes: dashmap::DashMap::new(),
            redis_conn: std::sync::OnceLock::new(),
            registration_sync: std::sync::OnceLock::new(),
            cluster_egress: std::sync::OnceLock::new(),
            registration_lookup_cache: dashmap::DashMap::new(),
            registration_lookup_locks: dashmap::DashMap::new(),
            parked_calls: std::sync::Arc::new(dashmap::DashMap::new()),
            nats_client: std::sync::OnceLock::new(),
            #[cfg(test)]
            test_gateways: std::sync::Mutex::new(Vec::new()),
            self_weak: std::sync::OnceLock::new(),
            sipflow_enabled: std::sync::atomic::AtomicBool::new(config.sipflow_enabled),
            sipflow_whitelist: std::sync::RwLock::new(config.sipflow_whitelist.clone()),
            sipflow_retention_days: std::sync::atomic::AtomicI32::new(
                config.sipflow_retention_days,
            ),
            sip_flow_tx: std::sync::OnceLock::new(),
            call_caller_addrs: dashmap::DashMap::new(),
            matched_call_ids: dashmap::DashMap::new(),
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
            gateway_health: GatewayHealthTracker::default(),
            inbound_transactions: dashmap::DashMap::new(),
            media_relay: MediaRelayState::new(),
            registrar: tokio::sync::RwLock::new(RegistrationStore::new()),
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
            sbc_rate_limit_enabled: config.sbc_rate_limit_enabled,
            external_to_internal_call_ids: dashmap::DashMap::new(),
            internal_to_external_call_ids: dashmap::DashMap::new(),
            gateway_cache: std::sync::RwLock::new(GatewayIdentityCache::default()),
            access_trunk_auth_modes: std::sync::RwLock::new(HashMap::new()),
            access_username_to_trunk_id: std::sync::RwLock::new(HashMap::new()),
            trunk_billing_accounts: std::sync::RwLock::new(HashMap::new()),
            did_destinations: std::sync::RwLock::new(HashMap::new()),
            extension_groups: std::sync::RwLock::new(HashMap::new()),
            ivr_menus: std::sync::RwLock::new(HashMap::new()),
            outbound_registrations: dashmap::DashMap::new(),
            access_ip_rules: std::sync::RwLock::new(Vec::new()),
            registered_access_users: std::sync::RwLock::new(Vec::new()),
            user_concurrency: dashmap::DashMap::new(),
            anti_fraud_rules: std::sync::RwLock::new(Vec::new()),
            media_metrics_log: config.media_metrics_log,
            billing_settlement_enabled: config.billing_settlement_enabled,
            gateway_health_persistence_enabled: config.gateway_health_checks_enabled,
            gateway_probes: dashmap::DashMap::new(),
            redis_conn: std::sync::OnceLock::new(),
            registration_sync: std::sync::OnceLock::new(),
            cluster_egress: std::sync::OnceLock::new(),
            registration_lookup_cache: dashmap::DashMap::new(),
            registration_lookup_locks: dashmap::DashMap::new(),
            parked_calls: std::sync::Arc::new(dashmap::DashMap::new()),
            nats_client: std::sync::OnceLock::new(),
            test_gateways: std::sync::Mutex::new(Vec::new()),
            self_weak: std::sync::OnceLock::new(),
            sipflow_enabled: std::sync::atomic::AtomicBool::new(config.sipflow_enabled),
            sipflow_whitelist: std::sync::RwLock::new(config.sipflow_whitelist.clone()),
            sipflow_retention_days: std::sync::atomic::AtomicI32::new(
                config.sipflow_retention_days,
            ),
            sip_flow_tx: std::sync::OnceLock::new(),
            call_caller_addrs: dashmap::DashMap::new(),
            matched_call_ids: dashmap::DashMap::new(),
        }
    }

    /// 设置 Redis 连接（仅在启动阶段调用一次）。
    pub(crate) fn set_redis(&self, conn: redis::aio::ConnectionManager) {
        let _ = self.redis_conn.set(conn);
    }

    /// 获取 Redis 连接管理器的克隆，各请求可并发发送命令并共享重连状态。
    pub(crate) fn redis_connection(&self) -> Option<redis::aio::ConnectionManager> {
        self.redis_conn.get().cloned()
    }

    pub(crate) fn set_nats(&self, conn: async_nats::Client) {
        let _ = self.nats_client.set(conn);
    }

    pub(crate) fn nats_connection(&self) -> Option<async_nats::Client> {
        self.nats_client.get().cloned()
    }

    pub(crate) fn set_registration_sync(&self, sender: crate::cluster::RegistrationSyncSender) {
        let _ = self.registration_sync.set(sender);
    }

    pub(crate) fn registration_sync(&self) -> Option<crate::cluster::RegistrationSyncSender> {
        self.registration_sync.get().cloned()
    }

    pub(crate) fn set_cluster_egress(&self, egress: ClusterEgress) {
        let _ = self.cluster_egress.set(egress);
    }

    async fn forward_to_flow_owner(
        &self,
        target: SocketAddr,
        bytes: Vec<u8>,
    ) -> Result<bool, std::io::Error> {
        let Some(egress) = self.cluster_egress.get() else {
            return Ok(false);
        };
        let Some(mut redis) = self.redis_connection() else {
            return Ok(false);
        };
        let payload: Option<String> = redis::cmd("GET")
            .arg(flow_key(target))
            .query_async(&mut redis)
            .await
            .map_err(std::io::Error::other)?;
        let Some(flow) = payload
            .as_deref()
            .and_then(|value| serde_json::from_str::<FlowRecord>(value).ok())
        else {
            return Ok(false);
        };
        if flow.owner_node_id == egress.node_id {
            return Ok(false);
        }
        egress
            .publish(&flow.owner_node_id, target, bytes)
            .await
            .map_err(std::io::Error::other)?;
        Ok(true)
    }

    /// 从 Redis 读取 SIP 鉴权凭据，不回退查询 PostgreSQL。
    pub(crate) async fn redis_auth_password(
        &self,
        username: &str,
        is_trunk: bool,
    ) -> Option<String> {
        let mut connection = self.redis_connection()?;
        let hash_key = if is_trunk {
            "vos_rs:auth:trunks"
        } else {
            "vos_rs:auth:extensions"
        };
        redis::cmd("HGET")
            .arg(hash_key)
            .arg(username)
            .query_async(&mut connection)
            .await
            .ok()
            .flatten()
    }

    /// 使用 Redis 凭据执行 SIP Digest 鉴权，不访问 PostgreSQL。
    pub(crate) async fn verify_sip_auth(
        &self,
        auth: &crate::sip::AuthConfig,
        request: &SipRequest,
        is_trunk: bool,
    ) -> crate::sip::AuthDecision {
        let username = auth.authorization_username(request);
        let password = if let Some(username) = username.as_deref() {
            self.redis_auth_password(username, is_trunk)
                .await
                .or_else(|| auth.configured_password(username))
        } else {
            None
        };
        let is_bypass = std::env::var("VOS_RS_AUTH_BYPASS")
            .ok()
            .as_deref()
            == Some("true");
        let auth_required = if !auth.is_enabled() || is_bypass {
            false
        } else {
            auth.is_enabled() || self.redis_connection().is_some()
        };
        auth.verify_request_with_password(
            request,
            password,
            auth_required,
            Some(&self.nonce_replay_cache),
        )
    }

    /// 从 Redis 一次读取账户余额与最长前缀费率。
    pub(crate) async fn redis_balance_check(
        &self,
        username: &str,
        callee: &str,
    ) -> Option<RedisBalanceCheck> {
        let mut connection = self.redis_connection()?;
        let prefixes = (0..=callee.len())
            .rev()
            .filter(|index| callee.is_char_boundary(*index))
            .map(|index| &callee[..index])
            .collect::<Vec<_>>();
        let mut pipeline = redis::pipe();
        pipeline
            .cmd("HGET")
            .arg("vos_rs:billing:balances")
            .arg(username)
            .cmd("HMGET")
            .arg("vos_rs:billing:intervals");
        for prefix in &prefixes {
            pipeline.arg(prefix);
        }
        pipeline.cmd("HMGET").arg("vos_rs:billing:prices");
        for prefix in &prefixes {
            pipeline.arg(prefix);
        }
        pipeline.cmd("HMGET").arg("vos_rs:billing:rates");
        for prefix in &prefixes {
            pipeline.arg(prefix);
        }
        let (balance, intervals, prices, legacy_rates): RedisBillingPipelineResult =
            pipeline.query_async(&mut connection).await.ok()?;
        let balance = balance.unwrap_or(0.0);
        let pulse = intervals
            .into_iter()
            .zip(prices)
            .find_map(|(interval, price)| interval.zip(price));
        let (billing_interval_secs, price_per_interval) =
            pulse.unwrap_or_else(|| (60, legacy_rates.into_iter().flatten().next().unwrap_or(0.0)));
        Some(RedisBalanceCheck {
            has_balance: balance >= price_per_interval || price_per_interval == 0.0,
            balance,
            billing_interval_secs,
            price_per_interval,
        })
    }

    /// 查找注册绑定的 Contact 地址。先查本地注册表，未命中再查 Redis。
    ///
    /// SIP 请求热路径不访问 PostgreSQL，避免数据库池等待拖慢所有呼叫。
    pub(crate) async fn lookup_contact(
        &self,
        uri: &SipUri,
    ) -> Option<crate::sip::registrar::RegistrationContact> {
        let aor = crate::sip::registrar::canonical_aor(uri).ok()?;
        if let Some(cached) = self.cached_registration_lookup(&aor) {
            return cached;
        }

        let lookup_lock = self
            .registration_lookup_locks
            .entry(aor.clone())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone();
        let _guard = lookup_lock.lock().await;
        if let Some(cached) = self.cached_registration_lookup(&aor) {
            return cached;
        }

        let now = std::time::SystemTime::now();
        let mut result = self
            .registrar
            .read()
            .await
            .lookup_contact(uri, now, None)
            .await;

        if result.is_none() {
            if let Some(mut conn) = self.redis_connection() {
                let redis_key = format!("vos_rs:reg:{aor}");
                let res: Result<String, redis::RedisError> = redis::cmd("GET")
                    .arg(&redis_key)
                    .query_async(&mut conn)
                    .await;
                if let Ok(json_str) = res {
                    if let Ok(contacts) = serde_json::from_str::<
                        Vec<crate::sip::registrar::RegistrationContact>,
                    >(&json_str)
                    {
                        result = contacts.into_iter().find(|contact| contact.expires > 0);
                    }
                }
            }
        }

        let ttl = if result.is_some() {
            POSITIVE_REGISTRATION_CACHE_TTL
        } else {
            NEGATIVE_REGISTRATION_CACHE_TTL
        };
        self.registration_lookup_cache.insert(
            aor,
            CachedRegistrationLookup {
                contact: result.clone(),
                expires_at: Instant::now() + ttl,
            },
        );
        self.prune_registration_lookup_cache();
        result
    }

    /// 使用号码库存解析被叫后查找注册 Contact。
    pub(crate) async fn lookup_destination_contact(
        &self,
        uri: &SipUri,
    ) -> Option<crate::sip::registrar::RegistrationContact> {
        let resolved = self.resolve_number_destination(uri);
        self.lookup_contact(&resolved).await
    }

    pub(crate) fn replace_did_destinations(&self, dids: HashMap<String, cdr_core::DidDestination>) {
        if let Ok(mut current) = self.did_destinations.write() {
            *current = dids;
        } else {
            tracing::error!("DID 目标路由缓存锁已损坏，忽略本次刷新");
        }
    }

    pub(crate) fn resolve_number_destination(&self, uri: &SipUri) -> SipUri {
        let Some(number) = uri.user.as_deref() else {
            return uri.clone();
        };
        let target_id = self.did_destinations.read().ok().and_then(|dids| {
            dids.get(number)
                .filter(|did| did.enabled && did.target_type == "extension")
                .map(|did| did.target_id.clone())
        });
        let Some(target_id) = target_id else {
            return uri.clone();
        };
        let mut resolved = uri.clone();
        resolved.user = Some(target_id.into());
        resolved
    }

    /// Returns the enabled DID rule for a real number.
    pub(crate) fn did_destination(&self, number: &str) -> Option<cdr_core::DidDestination> {
        self.did_destinations
            .read()
            .ok()
            .and_then(|destinations| destinations.get(number).filter(|did| did.enabled).cloned())
    }

    fn cached_registration_lookup(
        &self,
        aor: &str,
    ) -> Option<Option<crate::sip::registrar::RegistrationContact>> {
        let cached = self.registration_lookup_cache.get(aor)?;
        if cached.expires_at > Instant::now() {
            return Some(cached.contact.clone());
        }
        drop(cached);
        self.registration_lookup_cache.remove(aor);
        None
    }

    fn prune_registration_lookup_cache(&self) {
        if self.registration_lookup_cache.len() <= MAX_REGISTRATION_CACHE_ENTRIES {
            return;
        }
        let now = Instant::now();
        self.registration_lookup_cache
            .retain(|_, cached| cached.expires_at > now);
        self.registration_lookup_locks
            .retain(|aor, _| self.registration_lookup_cache.contains_key(aor));
    }

    pub(crate) fn invalidate_registration_lookup(&self, aor: &str) {
        self.registration_lookup_cache.remove(aor);
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

    /// 获取指定用户当前活跃并发通话数（O(1)）
    pub(crate) fn user_concurrent_count(&self, username: &str) -> u32 {
        self.user_concurrency.get(username).map_or(0, |c| *c)
    }

    /// INVITE 成功写入 inbound_transactions 后，递增该用户的并发计数
    pub(crate) fn increment_user_concurrency(&self, username: &str) {
        self.user_concurrency
            .entry(username.to_string())
            .and_modify(|c| *c += 1)
            .or_insert(1);
    }

    /// BYE/CANCEL/超时清理时递减用户并发计数（防止下溢）
    pub(crate) fn decrement_user_concurrency(&self, username: &str) {
        // remove_if 在同一分片锁内完成递减和删除，避免“先释放锁再 remove”
        // 导致并发 INVITE 刚加上的计数被误删。
        if let dashmap::mapref::entry::Entry::Occupied(mut entry) =
            self.user_concurrency.entry(username.to_string())
        {
            if *entry.get() <= 1 {
                entry.remove();
            } else {
                *entry.get_mut() -= 1;
            }
        }
    }

    /// 从 SIP 请求的 From 头中提取用户名
    pub(crate) fn username_from_request(request: &SipRequest) -> Option<String> {
        let from = request.headers.get("from")?;
        let s = from.as_str();
        let start = s.find("sip:").map(|i| i + 4)?;
        let end = s[start..].find('@')?;
        Some(s[start..start + end].to_string())
    }

    /// 从 SIP 请求的 From 头中提取域名作为租户标识
    pub(crate) fn domain_from_request(request: &SipRequest) -> Option<String> {
        let from = request.headers.get("from")?;
        let s = from.as_str();
        let start = s.find("sip:").map(|i| i + 4)?;
        let rest = &s[start..];
        let at_pos = rest.find('@')?;
        let domain_part = &rest[at_pos + 1..];
        let end_pos = domain_part
            .find(|c: char| !c.is_alphanumeric() && c != '.' && c != '-')
            .unwrap_or(domain_part.len());
        Some(domain_part[..end_pos].to_string())
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

    pub(crate) fn send_sip_datagram<'a>(
        self: &'a Arc<Self>,
        datagram: PendingDatagram,
        fallback_socket: &'a UdpSocket,
        edge_config: &'a EdgeConfig,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), std::io::Error>> + Send + 'a>>
    {
        Box::pin(async move {
            // The guard intentionally covers the actual socket/channel write. Merely serializing
            // response construction still allows a suspended provisional-response task to send
            // after a final response under high scheduler pressure.
            let _invite_response_guard = match datagram.invite_response.as_ref() {
                Some(metadata) => {
                    let mut order = metadata.order.lock().await;
                    if order.cseq != metadata.cseq {
                        if order
                            .cseq
                            .zip(metadata.cseq)
                            .is_some_and(|(current, pending)| pending < current)
                        {
                            debug!(
                                cseq = ?metadata.cseq,
                                current_cseq = ?order.cseq,
                                status = metadata.status_code,
                                "dropping response from an older INVITE transaction"
                            );
                            return Ok(());
                        }
                        order.cseq = metadata.cseq;
                        order.final_response_seen = metadata.status_code >= 200;
                        order.final_response_send_started = false;
                    }
                    if metadata.status_code < 200 && order.final_response_send_started {
                        debug!(
                            cseq = ?metadata.cseq,
                            status = metadata.status_code,
                            "dropping late provisional INVITE response before network send"
                        );
                        return Ok(());
                    }
                    if metadata.status_code >= 200 {
                        order.final_response_send_started = true;
                    }
                    Some(order)
                }
                None => None,
            };

            let target_addr: SocketAddr = match datagram.target.parse() {
                Ok(addr) => addr,
                Err(_) => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "invalid target address",
                    ));
                }
            };

            if self
                .sipflow_enabled
                .load(std::sync::atomic::Ordering::Relaxed)
            {
                self.capture_sip_packet(&datagram.bytes, "out", target_addr);
            }

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

            if matches!(
                transport,
                Transport::Tcp | Transport::Tls | Transport::Ws | Transport::Wss
            ) && self
                .forward_to_flow_owner(target_addr, datagram.bytes.clone())
                .await?
            {
                return Ok(());
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
                                    let fut: std::pin::Pin<
                                        Box<dyn std::future::Future<Output = ()> + Send>,
                                    > = Box::pin(async move {
                                        let datagrams =
                                            handle_datagram(&msg_bytes, peer_addr, &state, &config)
                                                .await;
                                        for d in datagrams {
                                            let _ = connection_tx.send(d.bytes).await;
                                        }
                                    });
                                    fut
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
                            .map_err(|e| {
                                std::io::Error::new(std::io::ErrorKind::InvalidInput, e)
                            })?;
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
                                            let fut: std::pin::Pin<
                                                Box<dyn std::future::Future<Output = ()> + Send>,
                                            > = Box::pin(async move {
                                                let datagrams = handle_datagram(
                                                    &msg_bytes, peer_addr, &state, &config,
                                                )
                                                .await;
                                                for d in datagrams {
                                                    let _ = connection_tx.send(d.bytes).await;
                                                }
                                            });
                                            fut
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
        })
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

    pub(crate) fn take_invite_ack_transaction(
        &self,
        key: &InviteAckKey,
    ) -> Option<tokio::sync::mpsc::Sender<transaction::ServerTransactionEvent>> {
        let transaction_key = self.server_transactions.iter().find_map(|entry| {
            (entry.key().invite_ack_key().as_ref() == Some(key)).then(|| entry.key().clone())
        })?;
        self.server_transactions
            .remove(&transaction_key)
            .map(|(_, tx)| tx)
            .filter(|tx| !tx.is_closed())
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

    pub(crate) async fn identify_egress_trunk(&self, peer: SocketAddr) -> Option<String> {
        #[cfg(test)]
        {
            let peer_ip = normalize_ip(peer.ip()).to_string();
            if self
                .test_gateways
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .contains(&peer_ip)
            {
                return Some("test-gateway".to_string());
            }
        }
        self.gateway_cache
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .identify(peer)
    }

    /// Replaces the egress identity cache with configured SIP endpoints.
    pub(crate) fn replace_gateway_endpoint_cache(
        &self,
        gateways: impl IntoIterator<Item = (String, u16, String)>,
    ) {
        let mut cache = self
            .gateway_cache
            .write()
            .unwrap_or_else(|error| error.into_inner());
        cache.replace(gateways);
    }

    pub(crate) fn gateway_target(&self, trunk_id: &str) -> Option<String> {
        self.gateway_cache
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .trunk_targets
            .get(trunk_id)
            .cloned()
    }

    #[cfg(test)]
    pub(crate) fn replace_gateway_cache(
        &self,
        gateways: impl IntoIterator<Item = (String, String)>,
    ) {
        self.replace_gateway_endpoint_cache(
            gateways
                .into_iter()
                .map(|(host, trunk_id)| (host, 5060, trunk_id)),
        );
    }

    pub(crate) fn access_trunk_auth_mode(&self, trunk_id: &str) -> String {
        self.access_trunk_auth_modes
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .get(trunk_id)
            .cloned()
            .unwrap_or_else(|| "none".to_string())
    }

    pub(crate) fn resolve_access_username_to_trunk(&self, username: &str) -> Option<String> {
        self.access_username_to_trunk_id
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .get(username)
            .cloned()
    }

    pub(crate) fn resolve_trunk_billing_account(&self, trunk_id: &str) -> Option<String> {
        self.trunk_billing_accounts
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .get(trunk_id)
            .cloned()
    }

    pub(crate) fn replace_access_sources(
        &self,
        rules: Vec<AccessIpRule>,
        registered_users: Vec<String>,
    ) {
        *self
            .access_ip_rules
            .write()
            .unwrap_or_else(|error| error.into_inner()) = rules;
        *self
            .registered_access_users
            .write()
            .unwrap_or_else(|error| error.into_inner()) = registered_users;
    }

    /// Returns a unique IP-authenticated access trunk. Overlap is rejected at runtime too.
    pub(crate) fn identify_access_trunk(
        &self,
        peer: SocketAddr,
        transport: &str,
    ) -> Result<Option<String>, ()> {
        let matches = self
            .access_ip_rules
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .iter()
            .filter(|rule| rule.transport.eq_ignore_ascii_case(transport))
            .filter(|rule| rule.source_port.is_none_or(|port| port == peer.port()))
            .filter(|rule| rule.network.contains(&peer.ip()))
            .map(|rule| rule.trunk_id.clone())
            .collect::<std::collections::HashSet<_>>();
        match matches.len() {
            0 => Ok(None),
            1 => Ok(matches.into_iter().next()),
            _ => Err(()),
        }
    }

    pub(crate) fn is_registered_access_username(&self, username: &str) -> bool {
        self.registered_access_users
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .iter()
            .any(|configured| configured == username)
    }

    pub(crate) fn cancel_client_transaction(&self, key: &ClientTransactionKey) {
        if let Some((_, cancel_tx)) = self.client_transactions.remove(key) {
            let _ = cancel_tx.send(());
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn remember_inbound_invite(
        &self,
        request: &SipRequest,
        peer: SocketAddr,
        outbound_uri: SipUri,
        caller_rtp: Option<RtpEndpoint>,
        gateway_relay_rtp: Option<RtpEndpoint>,
        callee_behind_nat: bool,
        max_duration_secs: Option<u32>,
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
                active_forks: Vec::new(),
                max_duration_secs,
                established_at: None,
                invite_response_order: Arc::new(tokio::sync::Mutex::new(
                    InviteResponseOrder::default(),
                )),
            },
        );

        // 记录该用户新增一路活跃并发通话
        if let Some(username) = Self::username_from_request(request) {
            self.increment_user_concurrency(&username);
        }
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
                        self.call_manager.set_recording_path(
                            &CallId::new(call_id),
                            format!("local:{}", path.display()),
                        );
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
        let metrics_log_enabled = self.media_metrics_log;
        if let Some(endpoint) = &transaction.gateway_relay_rtp {
            self.media_relay.clear_monitors(endpoint.port);
            let metrics = self.media_relay.metrics_for_port(endpoint.port);
            log_media_target_metrics("gateway", endpoint.port, metrics, metrics_log_enabled);
            self.media_relay.clear_target(endpoint.port);

            // 如果是参会成员，清理出会议室
            let mgr = self.media_relay.conference_manager.clone();
            let port = endpoint.port;
            tokio::spawn(async move {
                mgr.leave_conference(port).await;
            });
        }
        if let Some(endpoint) = &transaction.caller_relay_rtp {
            self.media_relay.clear_monitors(endpoint.port);
            let metrics = self.media_relay.metrics_for_port(endpoint.port);
            log_media_target_metrics("caller", endpoint.port, metrics, metrics_log_enabled);
            self.media_relay.clear_target(endpoint.port);

            // 如果是参会成员，清理出会议室
            let mgr = self.media_relay.conference_manager.clone();
            let port = endpoint.port;
            tokio::spawn(async move {
                mgr.leave_conference(port).await;
            });
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
            recording_dropped_packets = totals.recording_dropped_packets,
            recording_errors = totals.recording_errors,
            dtmf_events = totals.dtmf_events,
            "RTP relay metrics totals"
        );
    }
}

fn log_media_target_metrics(
    leg: &'static str,
    port: u16,
    metrics: MediaRelayMetrics,
    info_enabled: bool,
) {
    if info_enabled {
        info!(
            leg,
            port,
            received_packets = metrics.received_packets,
            forwarded_packets = metrics.forwarded_packets,
            dropped_invalid_packets = metrics.dropped_invalid_packets,
            dropped_no_target_packets = metrics.dropped_no_target_packets,
            send_errors = metrics.send_errors,
            learned_source_updates = metrics.learned_source_updates,
            rtcp_quality = ?metrics.rtcp_quality,
            recorded_packets = metrics.recorded_packets,
            recording_dropped_packets = metrics.recording_dropped_packets,
            recording_errors = metrics.recording_errors,
            dtmf_events = metrics.dtmf_events,
            "clearing RTP relay target"
        );
    } else {
        debug!(
            leg,
            port,
            received_packets = metrics.received_packets,
            forwarded_packets = metrics.forwarded_packets,
            dropped_invalid_packets = metrics.dropped_invalid_packets,
            dropped_no_target_packets = metrics.dropped_no_target_packets,
            send_errors = metrics.send_errors,
            learned_source_updates = metrics.learned_source_updates,
            rtcp_quality = ?metrics.rtcp_quality,
            recorded_packets = metrics.recorded_packets,
            recording_dropped_packets = metrics.recording_dropped_packets,
            recording_errors = metrics.recording_errors,
            dtmf_events = metrics.dtmf_events,
            "clearing RTP relay target"
        );
    }
}

#[cfg(test)]
mod gateway_identity_tests {
    use super::GatewayIdentityCache;

    fn identify(cache: &GatewayIdentityCache, peer: &str) -> Option<String> {
        cache.identify(peer.parse().expect("valid test peer"))
    }

    #[test]
    fn exact_endpoint_distinguishes_trunks_on_the_same_ip() {
        let mut cache = GatewayIdentityCache::default();
        cache.replace([
            ("127.0.0.1".to_string(), 5170, "carrier-a".to_string()),
            ("127.0.0.1".to_string(), 5171, "carrier-b".to_string()),
        ]);

        assert_eq!(
            identify(&cache, "127.0.0.1:5170").as_deref(),
            Some("carrier-a")
        );
        assert_eq!(
            identify(&cache, "127.0.0.1:5171").as_deref(),
            Some("carrier-b")
        );
        assert_eq!(identify(&cache, "127.0.0.1:5199"), None);
    }

    #[test]
    fn ip_fallback_only_accepts_a_unique_trunk() {
        let mut cache = GatewayIdentityCache::default();
        cache.replace([
            ("192.0.2.10".to_string(), 5060, "carrier-a".to_string()),
            ("192.0.2.10".to_string(), 5070, "carrier-a".to_string()),
        ]);
        assert_eq!(
            identify(&cache, "192.0.2.10:5090").as_deref(),
            Some("carrier-a")
        );

        cache.replace([
            ("192.0.2.10".to_string(), 5060, "carrier-a".to_string()),
            ("192.0.2.10".to_string(), 5070, "carrier-b".to_string()),
        ]);
        assert_eq!(identify(&cache, "192.0.2.10:5090"), None);
    }

    #[test]
    fn ipv4_mapped_ipv6_peers_match_ipv4_configuration() {
        let mut cache = GatewayIdentityCache::default();
        cache.replace([("192.0.2.10".to_string(), 5060, "carrier-a".to_string())]);

        assert_eq!(
            identify(&cache, "[::ffff:192.0.2.10]:5060").as_deref(),
            Some("carrier-a")
        );
    }

    #[test]
    fn duplicate_endpoint_owned_by_multiple_trunks_is_ambiguous() {
        let mut cache = GatewayIdentityCache::default();
        cache.replace([
            ("192.0.2.10".to_string(), 5060, "carrier-a".to_string()),
            ("192.0.2.10".to_string(), 5060, "carrier-b".to_string()),
        ]);

        assert_eq!(identify(&cache, "192.0.2.10:5060"), None);
    }
}
