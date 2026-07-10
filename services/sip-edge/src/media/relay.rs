use cdr_core::DtmfEventRecord;
use dashmap::DashMap;
use rtp_core::{RtcpPacket, RtpPacketView, SrtpError};
use sdp_core::RtpEndpoint;
use std::{
    collections::{HashMap, HashSet},
    net::SocketAddr,
    sync::{Arc, Mutex},
    path::PathBuf,
    io,
};
use tokio::{net::UdpSocket, task::JoinHandle};
use tracing::{debug, warn};

// Import custom media modules
pub use crate::media::config::{MediaConfig, DEFAULT_RTP_PORT_MIN};
use crate::media::config::{recording_worker_count, recording_queue_capacity};
pub use crate::media::metrics::{MediaRelayMetrics, RtcpQualitySnapshot, RtpReceiveStats};
pub use crate::media::crypto::MediaCryptoSession;
pub use crate::media::dtmf::DtmfState;
pub use crate::media::recording::{RecordingPool, RecordingLeg, MediaError};
pub use crate::media::utils::{unix_timestamp_millis, compact_ntp_middle_32_now};
use crate::media::sdp::socket_addr_for_endpoint;


pub const MAX_RTP_DATAGRAM_SIZE: usize = 65_535;

#[derive(Debug, Clone)]
pub(crate) struct PendingSrtpConfig {
    pub(crate) suite: String,
    pub(crate) key_params: String,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SourceBinding {
    pub(crate) address: SocketAddr,
    pub(crate) last_seen_unix_ms: u128,
}

pub struct MediaRelayState {
    pub(crate) targets: Arc<DashMap<u16, SocketAddr>>,
    pub(crate) peer_ports: Arc<DashMap<u16, u16>>,
    pub(crate) metrics: Arc<DashMap<u16, MediaRelayMetrics>>,
    pub(crate) recordings: Arc<DashMap<u16, RecordingLeg>>,
    pub(crate) recording_pool: Arc<RecordingPool>,
    pub(crate) dtmf_states: Arc<DashMap<u16, DtmfState>>,
    pub(crate) active_loops: Arc<DashMap<u16, Vec<tokio::sync::oneshot::Sender<()>>>>,
    pub(crate) crypto_sessions: Arc<DashMap<u16, Arc<tokio::sync::Mutex<MediaCryptoSession>>>>,
    pub(crate) pending_srtp: Arc<DashMap<u16, PendingSrtpConfig>>,
    pub(crate) rtp_stats: Arc<DashMap<u16, RtpReceiveStats>>,
    pub(crate) source_bindings: Arc<DashMap<u16, SourceBinding>>,
    pub(crate) state: Arc<Mutex<MediaRelayStateInner>>,
}

impl Clone for MediaRelayState {
    fn clone(&self) -> Self {
        Self {
            targets: Arc::clone(&self.targets),
            peer_ports: Arc::clone(&self.peer_ports),
            metrics: Arc::clone(&self.metrics),
            recordings: Arc::clone(&self.recordings),
            recording_pool: Arc::clone(&self.recording_pool),
            dtmf_states: Arc::clone(&self.dtmf_states),
            active_loops: Arc::clone(&self.active_loops),
            crypto_sessions: Arc::clone(&self.crypto_sessions),
            pending_srtp: Arc::clone(&self.pending_srtp),
            rtp_stats: Arc::clone(&self.rtp_stats),
            source_bindings: Arc::clone(&self.source_bindings),
            state: Arc::clone(&self.state),
        }
    }
}

impl std::fmt::Debug for MediaRelayState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MediaRelayState")
            .field("targets_len", &self.targets.len())
            .field("peer_ports_len", &self.peer_ports.len())
            .field("metrics_len", &self.metrics.len())
            .field("recordings_len", &self.recordings.len())
            .field("recording_workers", &self.recording_pool.worker_count())
            .finish()
    }
}

#[derive(Debug)]
pub(crate) struct MediaRelayStateInner {
    pub(crate) next_port: u16,
    pub(crate) leased_rtp_ports: HashSet<u16>,
    pub(crate) recording_dirs: HashSet<PathBuf>,
    pub(crate) dtmf_accumulators: HashMap<String, String>,
    pub(crate) dtmf_event_log: HashMap<String, Vec<DtmfEventRecord>>,
}

// Metrics structures moved to external metrics.rs

// Structural components moved to external modules within media/

impl MediaRelayState {
    pub fn new() -> Self {
        Self {
            targets: Arc::new(DashMap::new()),
            peer_ports: Arc::new(DashMap::new()),
            metrics: Arc::new(DashMap::new()),
            recordings: Arc::new(DashMap::new()),
            recording_pool: Arc::new(RecordingPool::new(
                recording_worker_count(),
                recording_queue_capacity(),
            )),
            dtmf_states: Arc::new(DashMap::new()),
            active_loops: Arc::new(DashMap::new()),
            crypto_sessions: Arc::new(DashMap::new()),
            pending_srtp: Arc::new(DashMap::new()),
            rtp_stats: Arc::new(DashMap::new()),
            source_bindings: Arc::new(DashMap::new()),
            state: Arc::new(Mutex::new(MediaRelayStateInner {
                next_port: DEFAULT_RTP_PORT_MIN,
                leased_rtp_ports: HashSet::new(),
                recording_dirs: HashSet::new(),
                dtmf_accumulators: HashMap::new(),
                dtmf_event_log: HashMap::new(),
            })),
        }
    }

    pub fn allocate_endpoint(&self, config: &MediaConfig) -> Result<RtpEndpoint, MediaError> {
        let mut inner = self.state.lock().expect("media relay lock poisoned");
        if inner.next_port < config.port_min || inner.next_port > config.port_max {
            inner.next_port = config.port_min;
        }

        let available_ports = ((config.port_max - config.port_min) / 2) + 1;
        for _ in 0..available_ports {
            let port = inner.next_port;
            inner.next_port = next_rtp_port(port, config);

            if inner.leased_rtp_ports.insert(port) {
                // Try to bind sockets dynamically for RTP and RTCP
                let rtp_addr = SocketAddr::from(([0, 0, 0, 0], port));
                let rtp_std = match std::net::UdpSocket::bind(rtp_addr) {
                    Ok(s) => s,
                    Err(e) => {
                        warn!(port, error = %e, "failed to bind RTP socket");
                        inner.leased_rtp_ports.remove(&port);
                        continue;
                    }
                };

                let rtcp_port = rtcp_port_for(port).unwrap_or(port + 1);
                let rtcp_addr = SocketAddr::from(([0, 0, 0, 0], rtcp_port));
                let rtcp_std = match std::net::UdpSocket::bind(rtcp_addr) {
                    Ok(s) => s,
                    Err(_) => {
                        inner.leased_rtp_ports.remove(&port);
                        continue;
                    }
                };

                // Convert to tokio UdpSockets
                rtp_std
                    .set_nonblocking(true)
                    .map_err(|e| MediaError::Io(e.to_string()))?;
                rtcp_std
                    .set_nonblocking(true)
                    .map_err(|e| MediaError::Io(e.to_string()))?;

                let rtp_socket = tokio::net::UdpSocket::from_std(rtp_std)
                    .map_err(|e| MediaError::Io(e.to_string()))?;
                let rtcp_socket = tokio::net::UdpSocket::from_std(rtcp_std)
                    .map_err(|e| MediaError::Io(e.to_string()))?;

                // Spawn loops
                let (rtp_tx, rtp_rx) = tokio::sync::oneshot::channel();
                let (rtcp_tx, rtcp_rx) = tokio::sync::oneshot::channel();

                let relay_clone1 = Self {
                    targets: Arc::clone(&self.targets),
                    peer_ports: Arc::clone(&self.peer_ports),
                    metrics: Arc::clone(&self.metrics),
                    recordings: Arc::clone(&self.recordings),
                    recording_pool: Arc::clone(&self.recording_pool),
                    dtmf_states: Arc::clone(&self.dtmf_states),
                    active_loops: Arc::clone(&self.active_loops),
                    crypto_sessions: Arc::clone(&self.crypto_sessions),
                    pending_srtp: Arc::clone(&self.pending_srtp),
                    rtp_stats: Arc::clone(&self.rtp_stats),
                    source_bindings: Arc::clone(&self.source_bindings),
                    state: Arc::clone(&self.state),
                };
                let rtp_learning = config.symmetric_rtp_learning;
                tokio::spawn(relay_media_port(
                    rtp_socket,
                    port,
                    relay_clone1,
                    rtp_learning,
                    config.anti_spoofing,
                    config.source_relearn_after_secs,
                    MediaPacketKind::Rtp,
                    rtp_rx,
                ));

                let relay_clone2 = Self {
                    targets: Arc::clone(&self.targets),
                    peer_ports: Arc::clone(&self.peer_ports),
                    metrics: Arc::clone(&self.metrics),
                    recordings: Arc::clone(&self.recordings),
                    recording_pool: Arc::clone(&self.recording_pool),
                    dtmf_states: Arc::clone(&self.dtmf_states),
                    active_loops: Arc::clone(&self.active_loops),
                    crypto_sessions: Arc::clone(&self.crypto_sessions),
                    pending_srtp: Arc::clone(&self.pending_srtp),
                    rtp_stats: Arc::clone(&self.rtp_stats),
                    source_bindings: Arc::clone(&self.source_bindings),
                    state: Arc::clone(&self.state),
                };
                tokio::spawn(relay_media_port(
                    rtcp_socket,
                    rtcp_port,
                    relay_clone2,
                    rtp_learning,
                    config.anti_spoofing,
                    config.source_relearn_after_secs,
                    MediaPacketKind::Rtcp,
                    rtcp_rx,
                ));

                self.active_loops.insert(port, vec![rtp_tx, rtcp_tx]);

                debug!(port, rtcp_port, "allocated media relay endpoint");
                return Ok(RtpEndpoint::new(config.advertised_addr.clone(), port));
            }
        }

        Err(MediaError::PortRangeExhausted {
            port_min: config.port_min,
            port_max: config.port_max,
        })
    }

    pub fn set_target(
        &self,
        relay_endpoint: &RtpEndpoint,
        target_endpoint: &RtpEndpoint,
    ) -> Result<(), MediaError> {
        let target = socket_addr_for_endpoint(target_endpoint)?;
        self.set_target_addr(relay_endpoint.port, target);
        if let (Some(relay_rtcp_port), Some(target_rtcp_port)) = (
            rtcp_port_for(relay_endpoint.port),
            rtcp_port_for(target_endpoint.port),
        ) {
            let mut target_rtcp_endpoint = target_endpoint.clone();
            target_rtcp_endpoint.port = target_rtcp_port;
            let target_rtcp = socket_addr_for_endpoint(&target_rtcp_endpoint)?;
            self.set_target_addr(relay_rtcp_port, target_rtcp);
        }
        Ok(())
    }

    pub fn set_target_addr(&self, relay_port: u16, target: SocketAddr) {
        self.targets.insert(relay_port, target);
    }

    /// Registers the SDES-SRTP context for one local RTP direction.
    #[allow(dead_code)]
    pub(crate) fn register_srtp_session(
        &self,
        relay_port: u16,
        suite: &str,
        key_params: &str,
        ssrc: u32,
    ) -> Result<(), SrtpError> {
        let session = MediaCryptoSession::from_sdes(suite, key_params, ssrc)?;
        self.crypto_sessions
            .insert(relay_port, Arc::new(tokio::sync::Mutex::new(session)));
        Ok(())
    }

    pub(crate) fn register_srtp_offer(&self, relay_port: u16, suite: &str, key_params: &str) {
        self.pending_srtp.insert(
            relay_port,
            PendingSrtpConfig {
                suite: suite.to_string(),
                key_params: key_params.to_string(),
            },
        );
    }

    pub(crate) fn clear_srtp_session(&self, relay_port: u16) {
        self.crypto_sessions.remove(&relay_port);
        self.pending_srtp.remove(&relay_port);
        if let Some(peer_port) = self.peer_ports.get(&relay_port).map(|value| *value) {
            self.crypto_sessions.remove(&peer_port);
            self.pending_srtp.remove(&peer_port);
        }
    }

    pub fn clear_target(&self, relay_port: u16) {
        let rtp_port = rtp_port_for(relay_port).unwrap_or(relay_port);
        let peer_port = self.peer_ports.get(&rtp_port).map(|v| *v);
        self.targets.remove(&rtp_port);
        self.metrics.remove(&rtp_port);
        self.rtp_stats.remove(&rtp_port);
        self.source_bindings.remove(&rtp_port);
        self.peer_ports.remove(&rtp_port);
        self.recordings.remove(&rtp_port);
        self.clear_srtp_session(rtp_port);
        self.dtmf_states.remove(&rtp_port);
        {
            let mut state = self.state.lock().expect("media relay lock poisoned");
            state.leased_rtp_ports.remove(&rtp_port);
        }
        if let Some(peer_port) = peer_port {
            self.targets.remove(&peer_port);
            self.metrics.remove(&peer_port);
            self.rtp_stats.remove(&peer_port);
            self.source_bindings.remove(&peer_port);
            self.peer_ports.remove(&peer_port);
            self.recordings.remove(&peer_port);
            self.dtmf_states.remove(&peer_port);
        }
        if let Some(rtcp_port) = rtcp_port_for(rtp_port) {
            self.targets.remove(&rtcp_port);
            self.metrics.remove(&rtcp_port);
            let rtcp_peer = self.peer_ports.get(&rtcp_port).map(|v| *v);
            self.peer_ports.remove(&rtcp_port);
            if let Some(rtcp_peer_port) = rtcp_peer {
                self.peer_ports.remove(&rtcp_peer_port);
            }
        }
        if let Some((_, senders)) = self.active_loops.remove(&rtp_port) {
            for sender in senders {
                let _ = sender.send(());
            }
        }
    }

    // DTMF processing methods moved to dtmf.rs

    pub fn target_for_port(&self, relay_port: u16) -> Option<SocketAddr> {
        self.targets.get(&relay_port).map(|v| *v)
    }

    pub fn metrics_for_port(&self, relay_port: u16) -> MediaRelayMetrics {
        self.metrics
            .get(&relay_port)
            .map(|v| *v)
            .unwrap_or_default()
    }

    #[cfg(test)]
    pub fn record_rtcp_reports_for_test(&self, relay_port: u16, quality: RtcpQualitySnapshot) {
        self.record_metric(relay_port, |metrics| {
            metrics.rtcp_quality = quality;
        });
    }

    pub fn metrics_totals(&self) -> MediaRelayMetrics {
        let mut totals = self.metrics.iter().map(|entry| *entry.value()).fold(
            MediaRelayMetrics::default(),
            |mut totals, metrics| {
                totals.received_packets += metrics.received_packets;
                totals.forwarded_packets += metrics.forwarded_packets;
                totals.dropped_invalid_packets += metrics.dropped_invalid_packets;
                totals.dropped_no_target_packets += metrics.dropped_no_target_packets;
                totals.send_errors += metrics.send_errors;
                totals.learned_source_updates += metrics.learned_source_updates;
                totals.dropped_spoofed_packets += metrics.dropped_spoofed_packets;
                totals.rtcp_quality.merge(metrics.rtcp_quality);
                totals.rtcp_window.merge(metrics.rtcp_window);
                totals.rtcp_quality_alerts += metrics.rtcp_quality_alerts;
                totals.rtcp_quality_degraded |= metrics.rtcp_quality_degraded;
                totals.recorded_packets += metrics.recorded_packets;
                totals.recording_dropped_packets += metrics.recording_dropped_packets;
                totals.recording_errors += metrics.recording_errors;
                totals.dtmf_events += metrics.dtmf_events;
                totals
            },
        );
        totals.recording_queue_depth = self.recording_pool.queued_commands() as u64;
        totals.recording_queue_capacity = self.recording_pool.total_capacity() as u64;
        totals.recording_workers = self.recording_pool.worker_count() as u64;
        totals
    }

    // Recording and storage management methods moved to recording.rs

    pub fn pair_ports(&self, first_port: u16, second_port: u16) {
        self.peer_ports.insert(first_port, second_port);
        self.peer_ports.insert(second_port, first_port);
        if let (Some(first_rtcp_port), Some(second_rtcp_port)) =
            (rtcp_port_for(first_port), rtcp_port_for(second_port))
        {
            self.peer_ports.insert(first_rtcp_port, second_rtcp_port);
            self.peer_ports.insert(second_rtcp_port, first_rtcp_port);
        }
    }

    pub fn peer_port_for(&self, relay_port: u16) -> Option<u16> {
        self.peer_ports.get(&relay_port).map(|v| *v)
    }

    fn learn_symmetric_source(
        &self,
        relay_port: u16,
        source: SocketAddr,
    ) -> Option<SymmetricSourceUpdate> {
        let peer_port = *self.peer_ports.get(&relay_port)?;
        let previous_target = self.targets.insert(peer_port, source);
        if previous_target == Some(source) {
            return None;
        }

        self.metrics
            .entry(relay_port)
            .or_default()
            .learned_source_updates += 1;

        Some(SymmetricSourceUpdate {
            source_port: relay_port,
            target_port: peer_port,
            previous_target,
            learned_target: source,
        })
    }

    fn accept_rtp_source(
        &self,
        relay_port: u16,
        source: SocketAddr,
        anti_spoofing: bool,
        relearn_after_secs: u64,
    ) -> bool {
        if !anti_spoofing {
            return true;
        }

        let now = unix_timestamp_millis();
        let mut binding = self
            .source_bindings
            .entry(relay_port)
            .or_insert(SourceBinding {
                address: source,
                last_seen_unix_ms: now,
            });
        if binding.address == source {
            binding.last_seen_unix_ms = now;
            return true;
        }

        let elapsed = now.saturating_sub(binding.last_seen_unix_ms);
        if elapsed >= u128::from(relearn_after_secs) * 1_000 {
            binding.address = source;
            binding.last_seen_unix_ms = now;
            return true;
        }

        self.record_metric(relay_port, |metrics| metrics.dropped_spoofed_packets += 1);
        false
    }

    fn record_metric(&self, relay_port: u16, update: impl FnOnce(&mut MediaRelayMetrics)) {
        update(self.metrics.entry(relay_port).or_default().value_mut());
    }

    fn record_rtcp_reports(&self, relay_port: u16, summary: &MediaPacketSummary) {
        if summary.rtcp_quality.reports == 0 {
            return;
        }

        self.record_metric(relay_port, |metrics| {
            metrics.rtcp_quality.merge(summary.rtcp_quality);
            metrics.rtcp_window.observe(summary.rtcp_quality);
            let degraded = metrics.rtcp_window.is_degraded();
            if degraded && !metrics.rtcp_quality_degraded {
                metrics.rtcp_quality_alerts += 1;
                warn!(
                    relay_port,
                    mos_x100 = ?metrics.rtcp_window.mos_x100,
                    r_factor_x100 = ?metrics.rtcp_window.r_factor_x100,
                    average_loss_x10000 = ?metrics
                        .rtcp_window
                        .average_fraction_lost
                        .map(|value| u32::from(value) * 10_000 / 255),
                    "RTCP quality degraded"
                );
            }
            metrics.rtcp_quality_degraded = degraded;
        });
    }

    // record_rtp_packet and process_dtmf_packet moved to external files dtmf.rs and recording.rs
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SymmetricSourceUpdate {
    source_port: u16,
    target_port: u16,
    previous_target: Option<SocketAddr>,
    learned_target: SocketAddr,
}

// Recording components and MediaError moved to external media/recording.rs

// SDP parsing and rewriting helper methods moved to sdp.rs

#[allow(dead_code)]
pub async fn spawn_rtp_relay_listeners(
    config: &MediaConfig,
    relay: MediaRelayState,
) -> io::Result<Vec<JoinHandle<()>>> {
    let mut sockets = Vec::new();

    for port in (config.port_min..=config.port_max).step_by(2) {
        let rtp_socket = UdpSocket::bind(SocketAddr::from(([0, 0, 0, 0], port))).await?;
        sockets.push((rtp_socket, port, MediaPacketKind::Rtp));

        if let Some(rtcp_port) = rtcp_port_for(port) {
            let rtcp_socket = UdpSocket::bind(SocketAddr::from(([0, 0, 0, 0], rtcp_port))).await?;
            sockets.push((rtcp_socket, rtcp_port, MediaPacketKind::Rtcp));
        }
    }

    let mut handles = Vec::new();

    for (socket, port, packet_kind) in sockets {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let relay_clone = MediaRelayState {
            targets: Arc::clone(&relay.targets),
            peer_ports: Arc::clone(&relay.peer_ports),
            metrics: Arc::clone(&relay.metrics),
            recordings: Arc::clone(&relay.recordings),
            recording_pool: Arc::clone(&relay.recording_pool),
            dtmf_states: Arc::clone(&relay.dtmf_states),
            active_loops: Arc::clone(&relay.active_loops),
            crypto_sessions: Arc::clone(&relay.crypto_sessions),
            pending_srtp: Arc::clone(&relay.pending_srtp),
            rtp_stats: Arc::clone(&relay.rtp_stats),
            source_bindings: Arc::clone(&relay.source_bindings),
            state: Arc::clone(&relay.state),
        };
        let handle = tokio::spawn(relay_media_port(
            socket,
            port,
            relay_clone,
            config.symmetric_rtp_learning,
            config.anti_spoofing,
            config.source_relearn_after_secs,
            packet_kind,
            rx,
        ));
        handles.push(handle);

        let rtp_port = rtp_port_for(port).unwrap_or(port);
        relay.active_loops.entry(rtp_port).or_default().push(tx);
    }

    Ok(handles)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MediaPacketKind {
    Rtp,
    Rtcp,
}

impl MediaPacketKind {
    fn label(self) -> &'static str {
        match self {
            Self::Rtp => "RTP",
            Self::Rtcp => "RTCP",
        }
    }

    fn inspect<'a>(self, packet: &'a [u8]) -> Result<MediaPacketSummary<'a>, rtp_core::RtpError> {
        match self {
            Self::Rtp => RtpPacketView::parse(packet).map(|rtp_packet| MediaPacketSummary {
                rtp_packet: Some(rtp_packet),
                ..MediaPacketSummary::default()
            }),
            Self::Rtcp => {
                let packets = RtcpPacket::parse_compound(packet)?;
                rtcp_summary(&packets)
            }
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct MediaPacketSummary<'a> {
    rtp_packet: Option<RtpPacketView<'a>>,
    rtcp_quality: RtcpQualitySnapshot,
}

fn rtcp_summary(packets: &[RtcpPacket]) -> Result<MediaPacketSummary<'static>, rtp_core::RtpError> {
    let mut rtcp_quality = RtcpQualitySnapshot::default();
    let arrival_ntp_middle_32 = compact_ntp_middle_32_now();

    for packet in packets {
        if let Some(report) = packet.sender_report()? {
            rtcp_quality.reports += 1;
            rtcp_quality.sender_reports += 1;
            for block in &report.report_blocks {
                rtcp_quality.record_report_block(block, arrival_ntp_middle_32);
            }
            continue;
        }

        if let Some(report) = packet.receiver_report()? {
            rtcp_quality.reports += 1;
            rtcp_quality.receiver_reports += 1;
            for block in &report.report_blocks {
                rtcp_quality.record_report_block(block, arrival_ntp_middle_32);
            }
        }
    }

    Ok(MediaPacketSummary {
        rtp_packet: None,
        rtcp_quality,
    })
}

// NTP and RTT helpers moved to utils.rs

#[allow(clippy::too_many_arguments)]
async fn relay_media_port(
    socket: UdpSocket,
    local_port: u16,
    relay: MediaRelayState,
    symmetric_rtp_learning: bool,
    anti_spoofing: bool,
    source_relearn_after_secs: u64,
    packet_kind: MediaPacketKind,
    mut shutdown_rx: tokio::sync::oneshot::Receiver<()>,
) {
    let mut buffer = vec![0_u8; MAX_RTP_DATAGRAM_SIZE];

    loop {
        let (size, source) = tokio::select! {
            _ = &mut shutdown_rx => {
                debug!(local_port, packet_kind = packet_kind.label(), "shutting down media port loop");
                break;
            }
            res = socket.recv_from(&mut buffer) => {
                match res {
                    Ok(received) => received,
                    Err(error) => {
                        warn!(%error, local_port, packet_kind = packet_kind.label(), "failed to receive media packet");
                        break;
                    }
                }
            }
        };
        relay.record_metric(local_port, |metrics| metrics.received_packets += 1);
        debug!(local_port, packet_kind = packet_kind.label(), size, %source, "received media packet");

        if !relay.accept_rtp_source(local_port, source, anti_spoofing, source_relearn_after_secs) {
            warn!(%source, local_port, packet_kind = packet_kind.label(), "dropping media packet from unbound source");
            continue;
        }

        let mut decrypted_packet = None;
        if packet_kind == MediaPacketKind::Rtp {
            if let Ok(view) = RtpPacketView::parse(&buffer[..size]) {
                let peer_port = relay.peer_ports.get(&local_port).map(|entry| *entry);
                for port in [Some(local_port), peer_port].into_iter().flatten() {
                    if relay.crypto_sessions.contains_key(&port) {
                        continue;
                    }
                    let Some(offer) = relay.pending_srtp.get(&port).map(|entry| entry.clone())
                    else {
                        continue;
                    };
                    match MediaCryptoSession::from_sdes(&offer.suite, &offer.key_params, view.ssrc)
                    {
                        Ok(session) => {
                            relay
                                .crypto_sessions
                                .insert(port, Arc::new(tokio::sync::Mutex::new(session)));
                        }
                        Err(error) => {
                            relay.record_metric(local_port, |metrics| {
                                metrics.dropped_invalid_packets += 1
                            });
                            warn!(%error, port, "invalid pending SDES-SRTP offer");
                        }
                    }
                }
            }
            if let Some(session) = relay
                .crypto_sessions
                .get(&local_port)
                .map(|entry| entry.clone())
            {
                let mut candidate = buffer[..size].to_vec();
                let decrypted_len = match session.lock().await.decrypt(&mut candidate) {
                    Ok(length) => length,
                    Err(error) => {
                        relay.record_metric(local_port, |metrics| {
                            metrics.dropped_invalid_packets += 1
                        });
                        warn!(%error, local_port, "dropping RTP packet with invalid SRTP authentication");
                        continue;
                    }
                };
                candidate.truncate(decrypted_len);
                decrypted_packet = Some(candidate);
            }
        }

        let packet = decrypted_packet.as_deref().unwrap_or(&buffer[..size]);
        if packet.is_empty() {
            continue;
        }

        let first_byte = packet[0];
        let is_pass_through = (0..=3).contains(&first_byte) || (20..=63).contains(&first_byte);

        let summary = if is_pass_through {
            None
        } else {
            match packet_kind.inspect(packet) {
                Ok(summary) => Some(summary),
                Err(error) => {
                    relay.record_metric(local_port, |metrics| metrics.dropped_invalid_packets += 1);
                    warn!(%error, %source, local_port, packet_kind = packet_kind.label(), "dropping invalid media packet");
                    continue;
                }
            }
        };

        if let Some(s) = &summary {
            relay.record_rtcp_reports(local_port, s);
        }

        let generated_receiver_report = summary
            .as_ref()
            .and_then(|summary| summary.rtp_packet)
            .and_then(|rtp_packet| {
                let mut stats = relay.rtp_stats.entry(local_port).or_default();
                stats.observe(rtp_packet);
                stats.receiver_report()
            });

        if symmetric_rtp_learning {
            if let Some(update) = relay.learn_symmetric_source(local_port, source) {
                debug!(
                    source_port = update.source_port,
                    target_port = update.target_port,
                    learned_target = %update.learned_target,
                    previous_target = ?update.previous_target,
                    packet_kind = packet_kind.label(),
                    "learned symmetric media source"
                );
            }
        }

        let Some(target) = relay.target_for_port(local_port) else {
            relay.record_metric(local_port, |metrics| metrics.dropped_no_target_packets += 1);
            debug!(%source, local_port, packet_kind = packet_kind.label(), "dropping media packet without relay target");
            continue;
        };

        if target.ip().is_unspecified() || target.port() == 0 {
            continue;
        }

        if let (Some(report), Some(rtcp_port)) =
            (generated_receiver_report, target.port().checked_add(1))
        {
            let rtcp_target = SocketAddr::new(target.ip(), rtcp_port);
            if let Err(error) = socket.send_to(&report, rtcp_target).await {
                relay.record_metric(local_port, |metrics| metrics.send_errors += 1);
                warn!(%error, local_port, %rtcp_target, "failed to send generated RTCP receiver report");
            } else {
                relay.record_metric(local_port, |metrics| metrics.forwarded_packets += 1);
            }
        }

        if let Some(s) = &summary {
            if let Some(rtp_packet) = s.rtp_packet.as_ref() {
                relay.process_dtmf_packet(local_port, *rtp_packet);
                match relay.record_rtp_packet(local_port, *rtp_packet) {
                    Ok(true) => {
                        relay.record_metric(local_port, |metrics| metrics.recorded_packets += 1);
                    }
                    Ok(false) => {}
                    Err(MediaError::RecordingQueueFull) => {
                        relay.record_metric(local_port, |metrics| {
                            metrics.recording_dropped_packets += 1
                        });
                    }
                    Err(error) => {
                        relay.record_metric(local_port, |metrics| metrics.recording_errors += 1);
                        warn!(%error, %source, local_port, "failed to record RTP packet");
                    }
                }
            }
        }

        let mut encrypted_packet = None;
        if packet_kind == MediaPacketKind::Rtp {
            if let Some(peer_port) = relay.peer_ports.get(&local_port).map(|entry| *entry) {
                if let Some(session) = relay
                    .crypto_sessions
                    .get(&peer_port)
                    .map(|entry| entry.clone())
                {
                    let mut candidate = packet.to_vec();
                    if let Err(error) = session.lock().await.encrypt(&mut candidate) {
                        relay.record_metric(local_port, |metrics| metrics.send_errors += 1);
                        warn!(%error, local_port, "failed to encrypt RTP packet for relay target");
                        continue;
                    }
                    encrypted_packet = Some(candidate);
                }
            }
        }
        let outbound_packet = encrypted_packet.as_deref().unwrap_or(packet);
        if let Err(error) = socket.send_to(outbound_packet, target).await {
            relay.record_metric(local_port, |metrics| metrics.send_errors += 1);
            warn!(%error, %source, %target, local_port, packet_kind = packet_kind.label(), "failed to relay media packet");
            continue;
        }

        relay.record_metric(local_port, |metrics| metrics.forwarded_packets += 1);
    }
}

fn rtcp_port_for(rtp_port: u16) -> Option<u16> {
    rtp_port.checked_add(1)
}

fn rtp_port_for(relay_port: u16) -> Option<u16> {
    if relay_port % 2 == 0 {
        Some(relay_port)
    } else {
        relay_port.checked_sub(1)
    }
}

fn next_rtp_port(port: u16, config: &MediaConfig) -> u16 {
    match port.checked_add(2) {
        Some(next) if next <= config.port_max => next,
        _ => config.port_min,
    }
}

// Helper functions moved to config.rs and sdp.rs

#[cfg(test)]
mod tests {
    use super::{
        spawn_rtp_relay_listeners,
        MediaConfig, MediaCryptoSession, MediaError, MediaRelayMetrics, MediaRelayState,
        RtcpQualitySnapshot, RtpPacketView,
    };
    use crate::media::sdp::{is_sdp_body, parse_sdp_rtp_endpoint, rewrite_sdp_body};
    use crate::media::recording::{decode_pcmu, decode_pcma, available_disk_bytes, RecordingPool};
    use crate::media::metrics::RtcpQualityWindow;
    use crate::media::utils::rtt_millis_from_compact_ntp;
    use rtp_core::{SrtpConfig, SrtpContext};
    use sdp_core::RtpEndpoint;
    use sip_core::{HeaderMap, HeaderName, HeaderValue};
    use std::{fs, net::SocketAddr, path::PathBuf};
    use tokio::net::UdpSocket;
    use tokio::time::{sleep, timeout, Duration};

    #[test]
    fn recording_pool_reports_capacity_and_queue_depth() {
        let pool = RecordingPool::new(2, 3);

        assert_eq!(pool.worker_count(), 2);
        assert_eq!(pool.total_capacity(), 6);
        assert_eq!(pool.queued_commands(), 0);
    }

    #[test]
    fn media_crypto_session_round_trips_rtp_payload() {
        let config = SrtpConfig {
            master_key: [7u8; 16],
            master_salt: [9u8; 14],
            profile: rtp_core::SrtpProfile::Aes128CmHmacSha1_80,
        };
        let mut sender = MediaCryptoSession {
            context: SrtpContext::new(config.clone(), 0x0102_0304),
        };
        let mut receiver = MediaCryptoSession {
            context: SrtpContext::new(config, 0x0102_0304),
        };
        let mut packet = vec![
            0x80, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0x01, 0x02, 0x03, 0x04, 1, 2, 3, 4,
        ];

        let original = packet.clone();
        sender.encrypt(&mut packet).unwrap();
        assert_ne!(packet, original);
        let decrypted_len = receiver.decrypt(&mut packet).unwrap();
        packet.truncate(decrypted_len);
        assert_eq!(packet, original);
    }

    #[test]
    fn rtcp_quality_window_calculates_averages_and_mos() {
        let mut window = RtcpQualityWindow::default();
        window.observe(RtcpQualitySnapshot {
            reports: 2,
            report_blocks: 2,
            last_fraction_lost: Some(13),
            last_jitter: Some(80),
            last_rtt_ms: Some(40),
            ..RtcpQualitySnapshot::default()
        });

        assert_eq!(window.samples, 2);
        assert_eq!(window.average_fraction_lost, Some(13));
        assert_eq!(window.average_jitter, Some(80));
        assert_eq!(window.average_rtt_ms, Some(40));
        assert!(window.r_factor_x100.is_some());
        assert!(window.mos_x100.is_some());
    }

    #[test]
    fn detects_application_sdp_with_parameters() {
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::new("Content-Type").unwrap(),
            HeaderValue::new("application/sdp; charset=utf-8"),
        );

        assert!(is_sdp_body(&headers, b"v=0\r\n"));
    }

    #[tokio::test]
    async fn allocates_even_ports_without_reusing_active_leases() {
        let config = MediaConfig::new("203.0.113.10", 40_001, 40_004);
        let relay = MediaRelayState::new();

        assert_eq!(
            relay.allocate_endpoint(&config).unwrap(),
            RtpEndpoint::new("203.0.113.10", 40_002)
        );
        assert_eq!(
            relay.allocate_endpoint(&config).unwrap(),
            RtpEndpoint::new("203.0.113.10", 40_004)
        );
        assert_eq!(
            relay.allocate_endpoint(&config).unwrap_err(),
            MediaError::PortRangeExhausted {
                port_min: 40_002,
                port_max: 40_004
            }
        );

        relay.clear_target(40_002);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(
            relay.allocate_endpoint(&config).unwrap(),
            RtpEndpoint::new("203.0.113.10", 40_002)
        );
    }

    #[test]
    fn estimates_rtcp_rtt_from_compact_ntp_values() {
        assert_eq!(
            rtt_millis_from_compact_ntp(0x0003_0000, 0x0001_0000, 0x0001_0000),
            Some(1_000)
        );
        assert_eq!(
            rtt_millis_from_compact_ntp(0x0003_0000, 0, 0x0001_0000),
            None
        );
        assert_eq!(
            rtt_millis_from_compact_ntp(0x0003_0000, 0x0001_0000, 0),
            None
        );
    }

    #[test]
    fn decodes_g711_static_payloads_to_pcm() {
        assert_eq!(decode_pcmu(0xff), 0);
        assert_eq!(decode_pcmu(0x7f), 0);
        assert_eq!(decode_pcma(0xd5), 8);
        assert_eq!(decode_pcma(0x55), -8);
    }

    #[test]
    fn records_pcmu_and_pcma_rtp_to_stereo_wav() {
        let dir = test_recording_dir("records_pcmu_and_pcma_rtp_to_stereo_wav");
        let config = MediaConfig::new("127.0.0.1", 40_000, 40_002).with_recording(true, &dir);
        let relay = MediaRelayState::new();
        let wav_path = relay
            .start_call_recording("call/with:unsafe@example.com", 40_000, 40_002, &config)
            .unwrap()
            .expect("recording should be enabled");

        let caller_packet = rtp_core::RtpPacket::new(0, 1, 0, 42, vec![0xff, 0xff])
            .unwrap()
            .encode()
            .unwrap();
        let caller_packet = RtpPacketView::parse(&caller_packet).unwrap();
        assert!(relay.record_rtp_packet(40_000, caller_packet).unwrap());

        let gateway_packet = rtp_core::RtpPacket::new(8, 1, 0, 24, vec![0xd5, 0xd5])
            .unwrap()
            .encode()
            .unwrap();
        let gateway_packet = RtpPacketView::parse(&gateway_packet).unwrap();
        assert!(relay.record_rtp_packet(40_002, gateway_packet).unwrap());
        relay.flush_recording_for_test(40_000).unwrap();

        let bytes = fs::read(&wav_path).unwrap();
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
        assert_eq!(u16::from_le_bytes([bytes[22], bytes[23]]), 2);
        assert_eq!(
            u32::from_le_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]),
            8_000
        );
        assert_eq!(
            u32::from_le_bytes([bytes[40], bytes[41], bytes[42], bytes[43]]),
            8
        );
        assert_eq!(bytes.len(), 52);
        assert_eq!(i16::from_le_bytes([bytes[44], bytes[45]]), 0);
        assert_eq!(i16::from_le_bytes([bytes[46], bytes[47]]), 8);
        assert_eq!(i16::from_le_bytes([bytes[48], bytes[49]]), 0);
        assert_eq!(i16::from_le_bytes([bytes[50], bytes[51]]), 8);

        assert!(!wav_path.with_extension("json").exists());
    }

    #[test]
    fn rotates_recording_when_segment_size_is_reached() {
        let dir = test_recording_dir("rotates_recording_when_segment_size_is_reached");
        let mut config = MediaConfig::new("127.0.0.1", 40_000, 40_002).with_recording(true, &dir);
        config.recording_max_file_bytes = 52;
        config.recording_max_duration_secs = 0;
        let relay = MediaRelayState::new();
        let first_path = relay
            .start_call_recording("rotating-call", 40_000, 40_002, &config)
            .unwrap()
            .expect("recording should be enabled");

        for (sequence, timestamp) in [(1, 0), (2, 2)] {
            let packet = rtp_core::RtpPacket::new(0, sequence, timestamp, 42, vec![0xff, 0xff])
                .unwrap()
                .encode()
                .unwrap();
            let packet = RtpPacketView::parse(&packet).unwrap();
            assert!(relay.record_rtp_packet(40_000, packet).unwrap());
        }
        relay.flush_recording_for_test(40_000).unwrap();

        let second_path = first_path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join(format!(
                "{}-part-0001.wav",
                first_path.file_stem().unwrap().to_string_lossy()
            ));
        assert!(first_path.is_file());
        assert!(second_path.is_file());
        assert!(!second_path.with_extension("json").exists());
    }

    #[test]
    fn rewrites_sdp_body_for_relay_endpoint() {
        let body = concat!(
            "v=0\r\n",
            "o=- 1 1 IN IP4 192.0.2.10\r\n",
            "s=-\r\n",
            "c=IN IP4 192.0.2.10\r\n",
            "t=0 0\r\n",
            "m=audio 49170 RTP/AVP 0 8 101\r\n",
            "a=rtpmap:0 PCMU/8000\r\n",
            "a=rtpmap:8 PCMA/8000\r\n",
            "a=rtpmap:101 telephone-event/8000\r\n"
        );

        let rewritten =
            rewrite_sdp_body(body.as_bytes(), RtpEndpoint::new("203.0.113.10", 40_000)).unwrap();
        let rewritten = String::from_utf8(rewritten).unwrap();

        assert!(rewritten.contains("c=IN IP4 203.0.113.10\r\n"));
        assert!(rewritten.contains("m=audio 40000 RTP/AVP 0 8 101\r\n"));
        assert!(rewritten.contains("a=rtpmap:0 PCMU/8000\r\n"));
        assert!(rewritten.contains("a=rtpmap:8 PCMA/8000\r\n"));
        assert!(rewritten.contains("a=rtpmap:101 telephone-event/8000\r\n"));
    }

    #[test]
    fn rejects_sdp_without_pcmu_or_pcma() {
        let body = concat!(
            "v=0\r\n",
            "o=- 1 1 IN IP4 192.0.2.10\r\n",
            "s=-\r\n",
            "c=IN IP4 192.0.2.10\r\n",
            "t=0 0\r\n",
            "m=audio 49170 RTP/AVP 101\r\n",
            "a=rtpmap:101 telephone-event/8000\r\n"
        );

        let error = rewrite_sdp_body(body.as_bytes(), RtpEndpoint::new("203.0.113.10", 40_000))
            .unwrap_err();

        assert!(error.to_string().contains("missing compatible audio codec"));
    }

    #[test]
    fn parses_original_sdp_rtp_endpoint() {
        let body = concat!(
            "v=0\r\n",
            "o=- 1 1 IN IP4 192.0.2.10\r\n",
            "s=-\r\n",
            "c=IN IP4 192.0.2.10\r\n",
            "t=0 0\r\n",
            "m=audio 49170 RTP/AVP 0\r\n"
        );

        assert_eq!(
            parse_sdp_rtp_endpoint(body.as_bytes()).unwrap(),
            RtpEndpoint::new("192.0.2.10", 49170)
        );
    }

    #[test]
    fn stores_and_clears_relay_targets() {
        let relay = MediaRelayState::new();
        let relay_endpoint = RtpEndpoint::new("203.0.113.10", 40_000);
        let target_endpoint = RtpEndpoint::new("127.0.0.1", 49_170);
        let target: SocketAddr = "127.0.0.1:49170".parse().unwrap();
        let rtcp_target: SocketAddr = "127.0.0.1:49171".parse().unwrap();

        relay.set_target(&relay_endpoint, &target_endpoint).unwrap();
        assert_eq!(relay.target_for_port(40_000), Some(target));
        assert_eq!(relay.target_for_port(40_001), Some(rtcp_target));

        relay.clear_target(40_000);
        assert_eq!(relay.target_for_port(40_000), None);
        assert_eq!(relay.target_for_port(40_001), None);
    }

    #[test]
    fn pairs_ports_and_learns_symmetric_source() {
        let relay = MediaRelayState::new();
        let original_target: SocketAddr = "127.0.0.1:49170".parse().unwrap();
        let learned_source: SocketAddr = "127.0.0.1:53000".parse().unwrap();

        relay.pair_ports(40_000, 40_002);
        relay.set_target_addr(40_002, original_target);

        assert_eq!(relay.peer_port_for(40_000), Some(40_002));
        assert_eq!(relay.peer_port_for(40_002), Some(40_000));
        assert_eq!(relay.peer_port_for(40_001), Some(40_003));
        assert_eq!(relay.peer_port_for(40_003), Some(40_001));

        let update = relay
            .learn_symmetric_source(40_000, learned_source)
            .expect("symmetric source should be learned");
        assert_eq!(update.source_port, 40_000);
        assert_eq!(update.target_port, 40_002);
        assert_eq!(update.previous_target, Some(original_target));
        assert_eq!(update.learned_target, learned_source);
        assert_eq!(relay.target_for_port(40_002), Some(learned_source));
        assert_eq!(relay.metrics_for_port(40_000).learned_source_updates, 1);

        assert_eq!(relay.learn_symmetric_source(40_000, learned_source), None);
        assert_eq!(relay.metrics_for_port(40_000).learned_source_updates, 1);

        relay.clear_target(40_000);
        assert_eq!(relay.peer_port_for(40_000), None);
        assert_eq!(relay.peer_port_for(40_002), None);
        assert_eq!(relay.peer_port_for(40_001), None);
        assert_eq!(relay.peer_port_for(40_003), None);
    }

    #[tokio::test]
    async fn rtp_relay_listener_forwards_valid_rtp_packets() {
        let relay_port = unused_even_udp_port();
        let config = MediaConfig::new("127.0.0.1", relay_port, relay_port);
        let relay = MediaRelayState::new();
        let handles = spawn_rtp_relay_listeners(&config, relay.clone())
            .await
            .unwrap();

        let target_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let target_port = target_socket.local_addr().unwrap().port();
        relay
            .set_target(
                &RtpEndpoint::new("127.0.0.1", config.port_min),
                &RtpEndpoint::new("127.0.0.1", target_port),
            )
            .unwrap();

        let sender = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let packet = rtp_core::RtpPacket::new(0, 1, 160, 42, vec![0x55, 0x56])
            .unwrap()
            .encode()
            .unwrap();
        sender
            .send_to(&packet, ("127.0.0.1", config.port_min))
            .await
            .unwrap();

        let mut buffer = [0_u8; 1500];
        let (size, _) = timeout(Duration::from_secs(1), target_socket.recv_from(&mut buffer))
            .await
            .expect("RTP packet should be relayed")
            .unwrap();
        assert_eq!(&buffer[..size], packet.as_slice());
        let metrics = wait_for_metrics(&relay, config.port_min, |metrics| {
            metrics.received_packets == 1 && metrics.forwarded_packets == 1
        })
        .await;
        assert_eq!(metrics.dropped_invalid_packets, 0);
        assert_eq!(metrics.dropped_no_target_packets, 0);
        assert_eq!(metrics.send_errors, 0);
        let totals = relay.metrics_totals();
        assert_eq!(totals.received_packets, metrics.received_packets);
        assert_eq!(totals.forwarded_packets, metrics.forwarded_packets);
        assert!(totals.recording_workers > 0);
        assert!(totals.recording_queue_capacity >= totals.recording_workers);

        for handle in handles {
            handle.abort();
        }
    }

    #[tokio::test]
    async fn rtp_relay_listener_learns_symmetric_source_for_paired_port() {
        let (caller_bound_port, gateway_bound_port) = unused_even_udp_port_pair();
        let config = MediaConfig::new("127.0.0.1", caller_bound_port, gateway_bound_port);
        let relay = MediaRelayState::new();
        let handles = spawn_rtp_relay_listeners(&config, relay.clone())
            .await
            .unwrap();
        relay.pair_ports(caller_bound_port, gateway_bound_port);

        let original_caller_target = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        relay.set_target_addr(
            caller_bound_port,
            original_caller_target.local_addr().unwrap(),
        );

        let gateway_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        relay.set_target_addr(gateway_bound_port, gateway_socket.local_addr().unwrap());

        let caller_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let learned_caller_addr = caller_socket.local_addr().unwrap();
        let caller_packet = rtp_core::RtpPacket::new(0, 1, 160, 42, vec![0x55, 0x56])
            .unwrap()
            .encode()
            .unwrap();
        caller_socket
            .send_to(&caller_packet, ("127.0.0.1", gateway_bound_port))
            .await
            .unwrap();

        let mut gateway_buffer = [0_u8; 1500];
        let (gateway_size, _) = timeout(
            Duration::from_secs(1),
            gateway_socket.recv_from(&mut gateway_buffer),
        )
        .await
        .expect("caller RTP should be relayed to gateway target")
        .unwrap();
        assert_eq!(&gateway_buffer[..gateway_size], caller_packet.as_slice());

        wait_for_target(&relay, caller_bound_port, learned_caller_addr).await;
        assert_eq!(
            relay
                .metrics_for_port(gateway_bound_port)
                .learned_source_updates,
            1
        );

        let gateway_packet = rtp_core::RtpPacket::new(8, 2, 320, 24, vec![0x11, 0x12])
            .unwrap()
            .encode()
            .unwrap();
        gateway_socket
            .send_to(&gateway_packet, ("127.0.0.1", caller_bound_port))
            .await
            .unwrap();

        let mut caller_buffer = [0_u8; 1500];
        let (caller_size, _) = timeout(
            Duration::from_secs(1),
            caller_socket.recv_from(&mut caller_buffer),
        )
        .await
        .expect("gateway RTP should use learned caller source")
        .unwrap();
        assert_eq!(&caller_buffer[..caller_size], gateway_packet.as_slice());

        for handle in handles {
            handle.abort();
        }
    }

    #[tokio::test]
    async fn rtcp_relay_listener_forwards_compound_packets() {
        let relay_port = unused_even_udp_port();
        let config = MediaConfig::new("127.0.0.1", relay_port, relay_port);
        let relay = MediaRelayState::new();
        let handles = spawn_rtp_relay_listeners(&config, relay.clone())
            .await
            .unwrap();

        let target_rtp_port = unused_even_udp_port();
        let target_rtcp_socket = UdpSocket::bind(("127.0.0.1", target_rtp_port + 1))
            .await
            .unwrap();
        relay
            .set_target(
                &RtpEndpoint::new("127.0.0.1", relay_port),
                &RtpEndpoint::new("127.0.0.1", target_rtp_port),
            )
            .unwrap();

        let sender = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let receiver_report = rtp_core::RtcpPacket::new(
            1,
            rtp_core::RtcpPacketType::ReceiverReport,
            vec![
                0xde, 0xad, 0xbe, 0xef, // reporter SSRC
                0x01, 0x02, 0x03, 0x04, // source SSRC
                0x20, 0x00, 0x00, 0x03, // fraction lost, cumulative lost
                0x00, 0x00, 0x10, 0x00, // extended highest sequence
                0x00, 0x00, 0x00, 0x2a, // jitter
                0x12, 0x34, 0x56, 0x78, // LSR
                0x00, 0x00, 0x00, 0x09, // DLSR
            ],
        )
        .unwrap()
        .encode()
        .unwrap();
        let source_description = rtp_core::RtcpPacket::new(
            1,
            rtp_core::RtcpPacketType::SourceDescription,
            vec![0xde, 0xad, 0xbe, 0xef],
        )
        .unwrap()
        .encode()
        .unwrap();
        let mut compound = receiver_report;
        compound.extend_from_slice(&source_description);

        sender
            .send_to(&compound, ("127.0.0.1", relay_port + 1))
            .await
            .unwrap();

        let mut buffer = [0_u8; 1500];
        let (size, _) = timeout(
            Duration::from_secs(1),
            target_rtcp_socket.recv_from(&mut buffer),
        )
        .await
        .expect("RTCP packet should be relayed")
        .unwrap();
        assert_eq!(&buffer[..size], compound.as_slice());

        let metrics = wait_for_metrics(&relay, relay_port + 1, |metrics| {
            metrics.received_packets == 1
                && metrics.forwarded_packets == 1
                && metrics.rtcp_quality.reports == 1
                && metrics.rtcp_quality.receiver_reports == 1
                && metrics.rtcp_quality.report_blocks == 1
        })
        .await;
        assert_eq!(metrics.dropped_invalid_packets, 0);
        assert_eq!(metrics.dropped_no_target_packets, 0);
        assert_eq!(metrics.send_errors, 0);
        assert_eq!(metrics.rtcp_quality.sender_reports, 0);
        assert_eq!(metrics.rtcp_quality.last_fraction_lost, Some(32));
        assert_eq!(metrics.rtcp_quality.max_fraction_lost, Some(32));
        assert_eq!(metrics.rtcp_quality.last_cumulative_lost, Some(3));
        assert_eq!(metrics.rtcp_quality.max_cumulative_lost, Some(3));
        assert_eq!(metrics.rtcp_quality.last_jitter, Some(42));
        assert_eq!(metrics.rtcp_quality.max_jitter, Some(42));
        assert_eq!(metrics.rtcp_quality.last_sender_report, Some(0x12345678));
        assert_eq!(metrics.rtcp_quality.delay_since_last_sender_report, Some(9));
        assert!(metrics.rtcp_quality.last_rtt_ms.is_some());
        assert_eq!(
            metrics.rtcp_quality.max_rtt_ms,
            metrics.rtcp_quality.last_rtt_ms
        );

        for handle in handles {
            handle.abort();
        }
    }

    #[tokio::test]
    async fn rtp_relay_listener_tracks_invalid_rtp_packets() {
        let relay_port = unused_even_udp_port();
        let config = MediaConfig::new("127.0.0.1", relay_port, relay_port);
        let relay = MediaRelayState::new();
        let handles = spawn_rtp_relay_listeners(&config, relay.clone())
            .await
            .unwrap();

        let sender = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        sender
            .send_to(&[0x80, 0x00], ("127.0.0.1", config.port_min))
            .await
            .unwrap();

        let metrics = wait_for_metrics(&relay, config.port_min, |metrics| {
            metrics.received_packets == 1 && metrics.dropped_invalid_packets == 1
        })
        .await;
        assert_eq!(metrics.forwarded_packets, 0);
        assert_eq!(metrics.dropped_no_target_packets, 0);
        assert_eq!(metrics.send_errors, 0);

        for handle in handles {
            handle.abort();
        }
    }

    #[tokio::test]
    async fn rtp_relay_listener_tracks_packets_without_target() {
        let relay_port = unused_even_udp_port();
        let config = MediaConfig::new("127.0.0.1", relay_port, relay_port);
        let relay = MediaRelayState::new();
        let handles = spawn_rtp_relay_listeners(&config, relay.clone())
            .await
            .unwrap();

        let sender = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let packet = rtp_core::RtpPacket::new(0, 1, 160, 42, vec![0x55, 0x56])
            .unwrap()
            .encode()
            .unwrap();
        sender
            .send_to(&packet, ("127.0.0.1", config.port_min))
            .await
            .unwrap();

        let metrics = wait_for_metrics(&relay, config.port_min, |metrics| {
            metrics.received_packets == 1 && metrics.dropped_no_target_packets == 1
        })
        .await;
        assert_eq!(metrics.forwarded_packets, 0);
        assert_eq!(metrics.dropped_invalid_packets, 0);
        assert_eq!(metrics.send_errors, 0);

        for handle in handles {
            handle.abort();
        }
    }

    async fn wait_for_metrics(
        relay: &MediaRelayState,
        port: u16,
        predicate: impl Fn(MediaRelayMetrics) -> bool,
    ) -> MediaRelayMetrics {
        timeout(Duration::from_secs(1), async {
            loop {
                let metrics = relay.metrics_for_port(port);
                if predicate(metrics) {
                    return metrics;
                }
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("RTP relay metrics should be updated")
    }

    async fn wait_for_target(relay: &MediaRelayState, port: u16, target: SocketAddr) {
        timeout(Duration::from_secs(1), async {
            loop {
                if relay.target_for_port(port) == Some(target) {
                    return;
                }
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("RTP relay target should be updated")
    }

    fn unused_even_udp_port() -> u16 {
        loop {
            let rtp_socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
            let port = rtp_socket.local_addr().unwrap().port();
            let Some(rtcp_port) = port.checked_add(1) else {
                continue;
            };
            if port % 2 == 0 && std::net::UdpSocket::bind(("127.0.0.1", rtcp_port)).is_ok() {
                return port;
            }
        }
    }

    fn unused_even_udp_port_pair() -> (u16, u16) {
        loop {
            let first_socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
            let first_port = first_socket.local_addr().unwrap().port();
            let Some(second_port) = first_port.checked_add(2) else {
                continue;
            };
            if first_port % 2 != 0 {
                continue;
            }

            let first_rtcp = std::net::UdpSocket::bind(("127.0.0.1", first_port + 1));
            let second_rtp = std::net::UdpSocket::bind(("127.0.0.1", second_port));
            let second_rtcp = std::net::UdpSocket::bind(("127.0.0.1", second_port + 1));
            if first_rtcp.is_ok() && second_rtp.is_ok() && second_rtcp.is_ok() {
                return (first_port, second_port);
            }
        }
    }

    fn test_recording_dir(name: &str) -> PathBuf {
        let dir = PathBuf::from("target")
            .join("test-recordings")
            .join(format!("{}-{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn recording_storage_policy_reports_available_space() {
        let dir = test_recording_dir("recording_storage_policy_reports_available_space");
        let available = available_disk_bytes(&dir).unwrap();
        assert!(available > 0);
    }

    #[test]
    fn recording_storage_policy_rejects_insufficient_space() {
        let dir = test_recording_dir("recording_storage_policy_rejects_insufficient_space");
        let relay = MediaRelayState::new();
        let mut config = MediaConfig::new("127.0.0.1", 40_000, 40_002).with_recording(true, &dir);
        config.recording_min_free_bytes = u64::MAX;

        let result = relay.start_call_recording("storage-policy-call", 40_000, 40_002, &config);
        assert!(matches!(
            result,
            Err(MediaError::Recording(message))
                if message.contains("below configured minimum")
        ));
    }

    #[test]
    fn test_dtmf_reconstruction() {
        let relay = MediaRelayState::new();
        let call_id = "test-call-dtmf";
        let local_port = 40000;
        let pt = 101;

        // 1. Register DTMF tracking
        relay.register_port_dtmf_tracking(call_id, local_port, pt);

        // 2. Simulate sending DTMF digit '5' (Event=5) with timestamp 1000
        // Payload: event=5, flags=0 (E=0), duration=80
        let payload1 = vec![5, 0, 0, 80];
        let packet1 = encoded_rtp_packet(pt, 1, 1000, payload1);
        relay.process_dtmf_packet(local_port, RtpPacketView::parse(&packet1).unwrap());

        // Send a duplicate packet of '5' (same timestamp)
        let payload2 = vec![5, 0x80, 0, 240]; // E=1
        let packet2 = encoded_rtp_packet(pt, 2, 1000, payload2);
        relay.process_dtmf_packet(local_port, RtpPacketView::parse(&packet2).unwrap());

        // 3. Simulate sending DTMF digit '#' (Event=11) with timestamp 2000
        let payload3 = vec![11, 0, 0, 80];
        let packet3 = encoded_rtp_packet(pt, 3, 2000, payload3);
        relay.process_dtmf_packet(local_port, RtpPacketView::parse(&packet3).unwrap());

        // 4. Verify reconstructed digits
        let digits = relay.get_dtmf_digits(call_id).unwrap();
        assert_eq!(digits, "5#");
        assert_eq!(relay.metrics_for_port(local_port).dtmf_events, 2);

        // 5. Clear digits and verify
        relay.clear_dtmf_digits(call_id);
        assert!(relay.get_dtmf_digits(call_id).is_none());
    }

    fn encoded_rtp_packet(
        payload_type: u8,
        sequence: u16,
        timestamp: u32,
        payload: Vec<u8>,
    ) -> Vec<u8> {
        rtp_core::RtpPacket::new(payload_type, sequence, timestamp, 42, payload)
            .unwrap()
            .encode()
            .unwrap()
    }

    #[tokio::test]
    async fn test_stun_and_dtls_passthrough() {
        let (port_a, port_b) = unused_even_udp_port_pair();
        let config = MediaConfig::new("127.0.0.1", port_a, port_b);
        let relay = MediaRelayState::new();

        let handles = spawn_rtp_relay_listeners(&config, relay.clone())
            .await
            .unwrap();

        // Register endpoints
        let caller_endpoint = RtpEndpoint::new("127.0.0.1", port_a);
        let gateway_endpoint = RtpEndpoint::new("127.0.0.1", port_b);
        relay
            .set_target(&caller_endpoint, &gateway_endpoint)
            .unwrap();

        let sender = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let receiver = UdpSocket::bind("127.0.0.1:0").await.unwrap();

        // Setup targets
        relay.set_target_addr(port_a, receiver.local_addr().unwrap());
        relay.set_target_addr(port_b, sender.local_addr().unwrap());

        // 1. Send STUN binding request (first byte 0x01)
        let stun_packet = vec![0x01, 0x00, 0x00, 0x08, 0x21, 0x12, 0xa4, 0x42];
        sender
            .send_to(&stun_packet, ("127.0.0.1", port_a))
            .await
            .unwrap();

        let mut recv_buf = vec![0_u8; 100];
        let (size, _from) = timeout(Duration::from_secs(1), receiver.recv_from(&mut recv_buf))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&recv_buf[..size], &stun_packet);

        // 2. Send DTLS client hello (first byte 0x16)
        let dtls_packet = vec![0x16, 0x03, 0x01, 0x00, 0x50];
        sender
            .send_to(&dtls_packet, ("127.0.0.1", port_a))
            .await
            .unwrap();

        let (size2, _from2) = timeout(Duration::from_secs(1), receiver.recv_from(&mut recv_buf))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&recv_buf[..size2], &dtls_packet);

        // Verify metrics show no dropped invalid packets for STUN/DTLS
        let metrics = wait_for_metrics(&relay, port_a, |m| m.received_packets == 2).await;
        assert_eq!(metrics.dropped_invalid_packets, 0);

        for handle in handles {
            handle.abort();
        }
    }
}
