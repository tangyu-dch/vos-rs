use cdr_core::DtmfEventRecord;
use dashmap::DashMap;
use rtp_core::{RtpPacketView, SrtpError};
use sdp_core::RtpEndpoint;
use std::{
    collections::{HashMap, HashSet},
    io,
    net::SocketAddr,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tokio::{net::UdpSocket, task::JoinHandle};
use tracing::{debug, warn};

// Import custom media modules
use super::rtcp_processor::{
    next_rtp_port, rtcp_port_for, rtp_port_for, MediaPacketKind, MediaPacketSummary,
};
use crate::media::config::{recording_queue_capacity, recording_worker_count};
pub use crate::media::config::{MediaConfig, DEFAULT_RTP_PORT_MIN};
pub use crate::media::crypto::MediaCryptoSession;
pub use crate::media::dtmf::DtmfState;
pub use crate::media::metrics::{MediaRelayMetrics, RtcpQualitySnapshot, RtpReceiveStats};
pub use crate::media::recording::{MediaError, RecordingLeg, RecordingPool};
use crate::media::sdp::socket_addr_for_endpoint;
pub use crate::media::utils::unix_timestamp_millis;
use std::sync::atomic::{AtomicU32, Ordering};

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
    pub(crate) codecs: Arc<DashMap<u16, rtp_core::AudioCodec>>,
    pub(crate) metrics: Arc<DashMap<u16, MediaRelayMetrics>>,
    pub(crate) recordings: Arc<DashMap<u16, RecordingLeg>>,
    pub(crate) recording_pool: Arc<RecordingPool>,
    pub(crate) dtmf_states: Arc<DashMap<u16, DtmfState>>,
    pub(crate) active_loops: Arc<DashMap<u16, Vec<tokio::sync::oneshot::Sender<()>>>>,
    pub(crate) crypto_sessions: Arc<DashMap<u16, Arc<tokio::sync::Mutex<MediaCryptoSession>>>>,
    pub(crate) pending_srtp: Arc<DashMap<u16, PendingSrtpConfig>>,
    pub(crate) rtp_stats: Arc<DashMap<u16, RtpReceiveStats>>,
    pub(crate) source_bindings: Arc<DashMap<u16, SourceBinding>>,
    pub(crate) leased_rtp_ports: Arc<dashmap::DashSet<u16>>,
    pub(crate) next_port: Arc<AtomicU32>,
    pub(crate) state: Arc<Mutex<MediaRelayStateInner>>,
}

impl Clone for MediaRelayState {
    fn clone(&self) -> Self {
        Self {
            targets: Arc::clone(&self.targets),
            peer_ports: Arc::clone(&self.peer_ports),
            codecs: Arc::clone(&self.codecs),
            metrics: Arc::clone(&self.metrics),
            recordings: Arc::clone(&self.recordings),
            recording_pool: Arc::clone(&self.recording_pool),
            dtmf_states: Arc::clone(&self.dtmf_states),
            active_loops: Arc::clone(&self.active_loops),
            crypto_sessions: Arc::clone(&self.crypto_sessions),
            pending_srtp: Arc::clone(&self.pending_srtp),
            rtp_stats: Arc::clone(&self.rtp_stats),
            source_bindings: Arc::clone(&self.source_bindings),
            leased_rtp_ports: Arc::clone(&self.leased_rtp_ports),
            next_port: Arc::clone(&self.next_port),
            state: Arc::clone(&self.state),
        }
    }
}

impl std::fmt::Debug for MediaRelayState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MediaRelayState")
            .field("targets_len", &self.targets.len())
            .field("peer_ports_len", &self.peer_ports.len())
            .field("codecs_len", &self.codecs.len())
            .field("metrics_len", &self.metrics.len())
            .field("recordings_len", &self.recordings.len())
            .field("recording_workers", &self.recording_pool.worker_count())
            .finish()
    }
}

#[derive(Debug)]
pub(crate) struct MediaRelayStateInner {
    pub(crate) recording_dirs: HashSet<PathBuf>,
    pub(crate) dtmf_accumulators: HashMap<String, String>,
    pub(crate) dtmf_event_log: HashMap<String, Vec<DtmfEventRecord>>,
}

impl MediaRelayState {
    pub fn new() -> Self {
        Self {
            targets: Arc::new(DashMap::new()),
            peer_ports: Arc::new(DashMap::new()),
            codecs: Arc::new(DashMap::new()),
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
            leased_rtp_ports: Arc::new(dashmap::DashSet::new()),
            next_port: Arc::new(AtomicU32::new(DEFAULT_RTP_PORT_MIN as u32)),
            state: Arc::new(Mutex::new(MediaRelayStateInner {
                recording_dirs: HashSet::new(),
                dtmf_accumulators: HashMap::new(),
                dtmf_event_log: HashMap::new(),
            })),
        }
    }

    pub fn allocate_endpoint(&self, config: &MediaConfig) -> Result<RtpEndpoint, MediaError> {
        let port_min = config.port_min;
        let port_max = config.port_max;
        let available_ports = ((port_max - port_min) / 2) + 1;

        let mut port_candidate = self.next_port.load(Ordering::Relaxed) as u16;
        if port_candidate < port_min || port_candidate > port_max {
            port_candidate = port_min;
            self.next_port
                .store(port_candidate as u32, Ordering::Relaxed);
        }

        for _ in 0..available_ports {
            let port = port_candidate;
            port_candidate = next_rtp_port(port, config);
            self.next_port
                .store(port_candidate as u32, Ordering::Relaxed);

            if self.leased_rtp_ports.insert(port) {
                // Try to bind sockets dynamically for RTP and RTCP
                let rtp_addr = SocketAddr::from(([0, 0, 0, 0], port));
                let rtp_std = match std::net::UdpSocket::bind(rtp_addr) {
                    Ok(s) => s,
                    Err(e) => {
                        warn!(port, error = %e, "failed to bind RTP socket");
                        self.leased_rtp_ports.remove(&port);
                        continue;
                    }
                };

                let rtcp_port = rtcp_port_for(port).unwrap_or(port + 1);
                let rtcp_addr = SocketAddr::from(([0, 0, 0, 0], rtcp_port));
                let rtcp_std = match std::net::UdpSocket::bind(rtcp_addr) {
                    Ok(s) => s,
                    Err(_) => {
                        self.leased_rtp_ports.remove(&port);
                        continue;
                    }
                };

                // Convert to tokio UdpSockets
                if let Err(e) = rtp_std.set_nonblocking(true) {
                    self.leased_rtp_ports.remove(&port);
                    return Err(MediaError::Io(e.to_string()));
                }
                if let Err(e) = rtcp_std.set_nonblocking(true) {
                    self.leased_rtp_ports.remove(&port);
                    return Err(MediaError::Io(e.to_string()));
                }

                let rtp_socket = match tokio::net::UdpSocket::from_std(rtp_std) {
                    Ok(s) => s,
                    Err(e) => {
                        self.leased_rtp_ports.remove(&port);
                        return Err(MediaError::Io(e.to_string()));
                    }
                };
                let rtcp_socket = match tokio::net::UdpSocket::from_std(rtcp_std) {
                    Ok(s) => s,
                    Err(e) => {
                        self.leased_rtp_ports.remove(&port);
                        return Err(MediaError::Io(e.to_string()));
                    }
                };

                // Spawn loops
                let (rtp_tx, rtp_rx) = tokio::sync::oneshot::channel();
                let (rtcp_tx, rtcp_rx) = tokio::sync::oneshot::channel();

                let relay_clone1 = self.clone();
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

                let relay_clone2 = self.clone();
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

                debug!(
                    port,
                    rtcp_port, "allocated media relay endpoint (lock-free)"
                );
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
        let rtp_target = socket_addr_for_endpoint(target_endpoint)?;
        self.set_target_addr(relay_endpoint.port, rtp_target);

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

        let binding_opt = self.source_bindings.get(&relay_port).map(|entry| *entry);
        if let Some(binding) = binding_opt {
            let target_port_opt = self.peer_ports.get(&relay_port).map(|entry| *entry);
            if let Some(target_port) = target_port_opt {
                self.targets.insert(target_port, binding.address);
            }
        }
    }

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

    pub fn register_port_codec(&self, port: u16, codec: rtp_core::AudioCodec) {
        self.codecs.insert(port, codec);
    }

    pub fn clear_target(&self, relay_port: u16) {
        let rtp_port = rtp_port_for(relay_port).unwrap_or(relay_port);
        let peer_port = self.peer_ports.get(&rtp_port).map(|v| *v);
        self.targets.remove(&rtp_port);
        self.metrics.remove(&rtp_port);
        self.rtp_stats.remove(&rtp_port);
        self.source_bindings.remove(&rtp_port);
        self.peer_ports.remove(&rtp_port);
        self.codecs.remove(&rtp_port);
        self.recordings.remove(&rtp_port);
        self.clear_srtp_session(rtp_port);
        self.dtmf_states.remove(&rtp_port);
        self.leased_rtp_ports.remove(&rtp_port);
        if let Some(peer_port) = peer_port {
            self.targets.remove(&peer_port);
            self.metrics.remove(&peer_port);
            self.rtp_stats.remove(&peer_port);
            self.source_bindings.remove(&peer_port);
            self.peer_ports.remove(&peer_port);
            self.codecs.remove(&peer_port);
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

    pub fn target_for_port(&self, relay_port: u16) -> Option<SocketAddr> {
        self.targets.get(&relay_port).map(|entry| *entry)
    }

    pub fn metrics_for_port(&self, relay_port: u16) -> MediaRelayMetrics {
        self.metrics
            .get(&relay_port)
            .map(|entry| *entry)
            .unwrap_or_default()
    }

    #[allow(dead_code)]
    pub fn record_rtcp_reports_for_test(&self, relay_port: u16, quality: RtcpQualitySnapshot) {
        self.record_metric(relay_port, |metrics| {
            metrics.rtcp_quality = quality;
        });
    }

    pub fn metrics_totals(&self) -> MediaRelayMetrics {
        let mut totals = MediaRelayMetrics::default();
        for entry in self.metrics.iter() {
            let metrics = entry.value();
            totals.received_packets += metrics.received_packets;
            totals.forwarded_packets += metrics.forwarded_packets;
            totals.dropped_invalid_packets += metrics.dropped_invalid_packets;
            totals.dropped_no_target_packets += metrics.dropped_no_target_packets;
            totals.send_errors += metrics.send_errors;
            totals.learned_source_updates += metrics.learned_source_updates;
            totals.recorded_packets += metrics.recorded_packets;
            totals.recording_dropped_packets += metrics.recording_dropped_packets;
            totals.recording_errors += metrics.recording_errors;

            if metrics.rtcp_quality.reports > 0 {
                totals.rtcp_quality.merge(metrics.rtcp_quality);
                totals.rtcp_window.merge(metrics.rtcp_window);
                totals.rtcp_quality_alerts += metrics.rtcp_quality_alerts;
                totals.rtcp_quality_degraded |= metrics.rtcp_quality_degraded;
            }
        }
        totals.recording_workers = self.recording_pool.worker_count() as u64;
        totals.recording_queue_capacity = self.recording_pool.total_capacity() as u64;
        totals.recording_queue_depth = self.recording_pool.queued_commands() as u64;
        totals
    }

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
        self.peer_ports.get(&relay_port).map(|entry| *entry)
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SymmetricSourceUpdate {
    source_port: u16,
    target_port: u16,
    previous_target: Option<SocketAddr>,
    learned_target: SocketAddr,
}

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
        let relay_clone = relay.clone();
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

        let mut transcoded_packet = None;
        if packet_kind == MediaPacketKind::Rtp {
            if let Some(peer_port) = relay.peer_ports.get(&local_port).map(|entry| *entry) {
                if let (Some(local_codec), Some(peer_codec)) = (
                    relay.codecs.get(&local_port).map(|v| *v),
                    relay.codecs.get(&peer_port).map(|v| *v),
                ) {
                    if local_codec != peer_codec {
                        if let Ok(mut rtp) = rtp_core::RtpPacket::parse(packet) {
                            let new_payload = match (local_codec, peer_codec) {
                                (rtp_core::AudioCodec::Pcma, rtp_core::AudioCodec::Pcmu) => Some(
                                    crate::media::transcode::transcode_pcma_to_pcmu(&rtp.payload),
                                ),
                                (rtp_core::AudioCodec::Pcmu, rtp_core::AudioCodec::Pcma) => Some(
                                    crate::media::transcode::transcode_pcmu_to_pcma(&rtp.payload),
                                ),
                                _ => None,
                            };
                            if let Some(payload) = new_payload {
                                rtp.payload = payload;
                                if let Some(pt) = peer_codec.static_payload_type() {
                                    rtp.payload_type = pt;
                                }
                                if let Ok(encoded) = rtp.encode() {
                                    transcoded_packet = Some(encoded);
                                }
                            }
                        }
                    }
                }
            }
        }
        let packet = transcoded_packet.as_deref().unwrap_or(packet);

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

#[cfg(test)]
mod tests;
