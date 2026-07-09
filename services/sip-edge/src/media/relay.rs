use cdr_core::DtmfEventRecord;
use dashmap::DashMap;
use rtp_core::{AudioCodec, RtcpPacket, RtcpReportBlock, RtpPacket, TelephoneEvent};
use sdp_core::{RtpEndpoint, SdpError, SessionDescription};
use sip_core::HeaderMap;
use std::{
    collections::{HashMap, HashSet},
    env,
    error::Error,
    fmt, fs,
    fs::File,
    io,
    io::{Read, Seek, SeekFrom, Write},
    net::{SocketAddr, ToSocketAddrs},
    path::{Path, PathBuf},
    str,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{net::UdpSocket, task::JoinHandle};
use tracing::{debug, warn};

pub const RTP_ADVERTISED_ADDR_ENV: &str = "VOS_RS_RTP_ADVERTISED_ADDR";
pub const RTP_PORT_MIN_ENV: &str = "VOS_RS_RTP_PORT_MIN";
pub const RTP_PORT_MAX_ENV: &str = "VOS_RS_RTP_PORT_MAX";
pub const RTP_SYMMETRIC_LEARNING_ENV: &str = "VOS_RS_RTP_SYMMETRIC_LEARNING";
pub const RECORDING_ENABLED_ENV: &str = "VOS_RS_RECORDING_ENABLED";
pub const RECORDING_DIR_ENV: &str = "VOS_RS_RECORDING_DIR";
pub const DEFAULT_RTP_ADVERTISED_ADDR: &str = "127.0.0.1";
pub const DEFAULT_RTP_PORT_MIN: u16 = 40_000;
pub const DEFAULT_RTP_PORT_MAX: u16 = 40_100;
pub const DEFAULT_RTP_SYMMETRIC_LEARNING: bool = true;
pub const DEFAULT_RECORDING_ENABLED: bool = false;
pub const DEFAULT_RECORDING_DIR: &str = "target/recordings";
const MAX_RTP_DATAGRAM_SIZE: usize = 65_535;
const RECORDING_SAMPLE_RATE: u32 = 8_000;
const RECORDING_CHANNELS: u16 = 2;
const RECORDING_BITS_PER_SAMPLE: u16 = 16;
const WAV_HEADER_LEN: u64 = 44;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaConfig {
    pub advertised_addr: String,
    pub port_min: u16,
    pub port_max: u16,
    pub symmetric_rtp_learning: bool,
    pub recording_enabled: bool,
    pub recording_dir: PathBuf,
}

impl MediaConfig {
    #[cfg(test)]
    pub fn new(advertised_addr: impl Into<String>, port_min: u16, port_max: u16) -> Self {
        Self::new_with_symmetric_learning(
            advertised_addr,
            port_min,
            port_max,
            DEFAULT_RTP_SYMMETRIC_LEARNING,
        )
    }

    pub fn new_with_symmetric_learning(
        advertised_addr: impl Into<String>,
        port_min: u16,
        port_max: u16,
        symmetric_rtp_learning: bool,
    ) -> Self {
        let mut port_min = even_port_at_or_above(port_min).unwrap_or(DEFAULT_RTP_PORT_MIN);
        let mut port_max = even_port_at_or_below(port_max).unwrap_or(DEFAULT_RTP_PORT_MAX);

        if port_min > port_max {
            port_min = DEFAULT_RTP_PORT_MIN;
            port_max = DEFAULT_RTP_PORT_MAX;
        }

        Self {
            advertised_addr: advertised_addr.into(),
            port_min,
            port_max,
            symmetric_rtp_learning,
            recording_enabled: DEFAULT_RECORDING_ENABLED,
            recording_dir: PathBuf::from(DEFAULT_RECORDING_DIR),
        }
    }

    pub fn from_env() -> Self {
        let advertised_addr = env::var(RTP_ADVERTISED_ADDR_ENV)
            .unwrap_or_else(|_| DEFAULT_RTP_ADVERTISED_ADDR.to_string());
        let port_min = env_port(RTP_PORT_MIN_ENV).unwrap_or(DEFAULT_RTP_PORT_MIN);
        let port_max = env_port(RTP_PORT_MAX_ENV).unwrap_or(DEFAULT_RTP_PORT_MAX);
        let symmetric_rtp_learning =
            env_bool(RTP_SYMMETRIC_LEARNING_ENV).unwrap_or(DEFAULT_RTP_SYMMETRIC_LEARNING);
        let recording_enabled =
            env_bool(RECORDING_ENABLED_ENV).unwrap_or(DEFAULT_RECORDING_ENABLED);
        let recording_dir =
            env::var(RECORDING_DIR_ENV).unwrap_or_else(|_| DEFAULT_RECORDING_DIR.to_string());
        let mut config = Self::new_with_symmetric_learning(
            advertised_addr,
            port_min,
            port_max,
            symmetric_rtp_learning,
        );
        config.recording_enabled = recording_enabled;
        config.recording_dir = PathBuf::from(recording_dir);
        config
    }

    #[cfg(test)]
    pub fn with_recording(mut self, enabled: bool, dir: impl Into<PathBuf>) -> Self {
        self.recording_enabled = enabled;
        self.recording_dir = dir.into();
        self
    }

    pub fn set_advertised_addr(&mut self, addr: impl Into<String>) {
        self.advertised_addr = addr.into();
    }
}

pub struct MediaRelayState {
    targets: Arc<DashMap<u16, SocketAddr>>,
    peer_ports: Arc<DashMap<u16, u16>>,
    metrics: Arc<DashMap<u16, MediaRelayMetrics>>,
    recordings: Arc<DashMap<u16, RecordingLeg>>,
    dtmf_states: Arc<DashMap<u16, DtmfState>>,
    active_loops: Arc<DashMap<u16, Vec<tokio::sync::oneshot::Sender<()>>>>,
    state: Arc<Mutex<MediaRelayStateInner>>,
}

impl Clone for MediaRelayState {
    fn clone(&self) -> Self {
        Self {
            targets: Arc::clone(&self.targets),
            peer_ports: Arc::clone(&self.peer_ports),
            metrics: Arc::clone(&self.metrics),
            recordings: Arc::clone(&self.recordings),
            dtmf_states: Arc::clone(&self.dtmf_states),
            active_loops: Arc::clone(&self.active_loops),
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
            .finish()
    }
}

#[derive(Debug)]
struct MediaRelayStateInner {
    next_port: u16,
    leased_rtp_ports: HashSet<u16>,
    dtmf_accumulators: HashMap<String, String>,
    dtmf_event_log: HashMap<String, Vec<DtmfEventRecord>>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MediaRelayMetrics {
    pub received_packets: u64,
    pub forwarded_packets: u64,
    pub dropped_invalid_packets: u64,
    pub dropped_no_target_packets: u64,
    pub send_errors: u64,
    pub learned_source_updates: u64,
    pub rtcp_quality: RtcpQualitySnapshot,
    pub recorded_packets: u64,
    pub recording_errors: u64,
    pub dtmf_events: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RtcpQualitySnapshot {
    pub reports: u64,
    pub sender_reports: u64,
    pub receiver_reports: u64,
    pub report_blocks: u64,
    pub last_fraction_lost: Option<u8>,
    pub max_fraction_lost: Option<u8>,
    pub last_cumulative_lost: Option<i32>,
    pub max_cumulative_lost: Option<i32>,
    pub last_jitter: Option<u32>,
    pub max_jitter: Option<u32>,
    pub last_sender_report: Option<u32>,
    pub delay_since_last_sender_report: Option<u32>,
    pub last_rtt_ms: Option<u32>,
    pub max_rtt_ms: Option<u32>,
}

impl RtcpQualitySnapshot {
    fn merge(&mut self, other: Self) {
        self.reports += other.reports;
        self.sender_reports += other.sender_reports;
        self.receiver_reports += other.receiver_reports;
        self.report_blocks += other.report_blocks;

        if let Some(value) = other.last_fraction_lost {
            self.last_fraction_lost = Some(value);
            self.max_fraction_lost = max_option(self.max_fraction_lost, value);
        }
        if let Some(value) = other.max_fraction_lost {
            self.max_fraction_lost = max_option(self.max_fraction_lost, value);
        }
        if let Some(value) = other.last_cumulative_lost {
            self.last_cumulative_lost = Some(value);
            self.max_cumulative_lost = max_option(self.max_cumulative_lost, value);
        }
        if let Some(value) = other.max_cumulative_lost {
            self.max_cumulative_lost = max_option(self.max_cumulative_lost, value);
        }
        if let Some(value) = other.last_jitter {
            self.last_jitter = Some(value);
            self.max_jitter = max_option(self.max_jitter, value);
        }
        if let Some(value) = other.max_jitter {
            self.max_jitter = max_option(self.max_jitter, value);
        }
        if let Some(value) = other.last_sender_report {
            self.last_sender_report = Some(value);
        }
        if let Some(value) = other.delay_since_last_sender_report {
            self.delay_since_last_sender_report = Some(value);
        }
        if let Some(value) = other.last_rtt_ms {
            self.last_rtt_ms = Some(value);
            self.max_rtt_ms = max_option(self.max_rtt_ms, value);
        }
        if let Some(value) = other.max_rtt_ms {
            self.max_rtt_ms = max_option(self.max_rtt_ms, value);
        }
    }

    fn record_report_block(&mut self, block: &RtcpReportBlock, arrival_ntp_middle_32: u32) {
        self.report_blocks += 1;
        self.last_fraction_lost = Some(block.fraction_lost);
        self.max_fraction_lost = max_option(self.max_fraction_lost, block.fraction_lost);
        self.last_cumulative_lost = Some(block.cumulative_lost);
        self.max_cumulative_lost = max_option(self.max_cumulative_lost, block.cumulative_lost);
        self.last_jitter = Some(block.interarrival_jitter);
        self.max_jitter = max_option(self.max_jitter, block.interarrival_jitter);
        self.last_sender_report = Some(block.last_sender_report);
        self.delay_since_last_sender_report = Some(block.delay_since_last_sender_report);

        if let Some(rtt_ms) = rtt_millis_from_compact_ntp(
            arrival_ntp_middle_32,
            block.last_sender_report,
            block.delay_since_last_sender_report,
        ) {
            self.last_rtt_ms = Some(rtt_ms);
            self.max_rtt_ms = max_option(self.max_rtt_ms, rtt_ms);
        }
    }
}

fn max_option<T: Ord + Copy>(current: Option<T>, candidate: T) -> Option<T> {
    Some(current.map_or(candidate, |value| value.max(candidate)))
}

#[derive(Debug, Clone)]
struct DtmfState {
    call_id: String,
    payload_type: u8,
    last_timestamp: Option<u32>,
}

impl MediaRelayState {
    pub fn new() -> Self {
        Self {
            targets: Arc::new(DashMap::new()),
            peer_ports: Arc::new(DashMap::new()),
            metrics: Arc::new(DashMap::new()),
            recordings: Arc::new(DashMap::new()),
            dtmf_states: Arc::new(DashMap::new()),
            active_loops: Arc::new(DashMap::new()),
            state: Arc::new(Mutex::new(MediaRelayStateInner {
                next_port: DEFAULT_RTP_PORT_MIN,
                leased_rtp_ports: HashSet::new(),
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
                    dtmf_states: Arc::clone(&self.dtmf_states),
                    active_loops: Arc::clone(&self.active_loops),
                    state: Arc::clone(&self.state),
                };
                let rtp_learning = config.symmetric_rtp_learning;
                tokio::spawn(relay_media_port(
                    rtp_socket,
                    port,
                    relay_clone1,
                    rtp_learning,
                    MediaPacketKind::Rtp,
                    rtp_rx,
                ));

                let relay_clone2 = Self {
                    targets: Arc::clone(&self.targets),
                    peer_ports: Arc::clone(&self.peer_ports),
                    metrics: Arc::clone(&self.metrics),
                    recordings: Arc::clone(&self.recordings),
                    dtmf_states: Arc::clone(&self.dtmf_states),
                    active_loops: Arc::clone(&self.active_loops),
                    state: Arc::clone(&self.state),
                };
                tokio::spawn(relay_media_port(
                    rtcp_socket,
                    rtcp_port,
                    relay_clone2,
                    rtp_learning,
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

    pub fn clear_target(&self, relay_port: u16) {
        let rtp_port = rtp_port_for(relay_port).unwrap_or(relay_port);
        let peer_port = self.peer_ports.get(&rtp_port).map(|v| *v);
        self.targets.remove(&rtp_port);
        self.metrics.remove(&rtp_port);
        self.peer_ports.remove(&rtp_port);
        self.recordings.remove(&rtp_port);
        self.dtmf_states.remove(&rtp_port);
        {
            let mut state = self.state.lock().expect("media relay lock poisoned");
            state.leased_rtp_ports.remove(&rtp_port);
        }
        if let Some(peer_port) = peer_port {
            self.targets.remove(&peer_port);
            self.metrics.remove(&peer_port);
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

    pub fn register_port_dtmf_tracking(&self, call_id: &str, port: u16, payload_type: u8) {
        self.dtmf_states.insert(
            port,
            DtmfState {
                call_id: call_id.to_string(),
                payload_type,
                last_timestamp: None,
            },
        );
    }

    pub fn get_dtmf_digits(&self, call_id: &str) -> Option<String> {
        let inner = self.state.lock().expect("media relay lock poisoned");
        inner.dtmf_accumulators.get(call_id).cloned()
    }

    pub fn clear_dtmf_digits(&self, call_id: &str) {
        let mut inner = self.state.lock().expect("media relay lock poisoned");
        inner.dtmf_accumulators.remove(call_id);
    }

    pub fn register_info_dtmf_digit(&self, call_id: &str, digit: char) {
        let mut inner = self.state.lock().expect("media relay lock poisoned");
        let acc = inner
            .dtmf_accumulators
            .entry(call_id.to_string())
            .or_default();
        acc.push(digit);
        let record = DtmfEventRecord::from_sip_info(call_id, digit);
        inner
            .dtmf_event_log
            .entry(call_id.to_string())
            .or_default()
            .push(record);
        debug!(call_id, digit = %digit, "reconstructed DTMF digit from SIP INFO");
    }

    pub fn take_dtmf_events(&self, call_id: &str) -> Vec<DtmfEventRecord> {
        let mut inner = self.state.lock().expect("media relay lock poisoned");
        inner.dtmf_event_log.remove(call_id).unwrap_or_default()
    }

    pub fn clear_dtmf_events(&self, call_id: &str) {
        let mut inner = self.state.lock().expect("media relay lock poisoned");
        inner.dtmf_event_log.remove(call_id);
    }

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
        self.metrics.iter().map(|entry| *entry.value()).fold(
            MediaRelayMetrics::default(),
            |mut totals, metrics| {
                totals.received_packets += metrics.received_packets;
                totals.forwarded_packets += metrics.forwarded_packets;
                totals.dropped_invalid_packets += metrics.dropped_invalid_packets;
                totals.dropped_no_target_packets += metrics.dropped_no_target_packets;
                totals.send_errors += metrics.send_errors;
                totals.learned_source_updates += metrics.learned_source_updates;
                totals.rtcp_quality.merge(metrics.rtcp_quality);
                totals.recorded_packets += metrics.recorded_packets;
                totals.recording_errors += metrics.recording_errors;
                totals.dtmf_events += metrics.dtmf_events;
                totals
            },
        )
    }

    pub fn start_call_recording(
        &self,
        call_id: &str,
        caller_relay_port: u16,
        gateway_relay_port: u16,
        config: &MediaConfig,
    ) -> Result<Option<PathBuf>, MediaError> {
        if !config.recording_enabled {
            return Ok(None);
        }

        let caller_relay_port = rtp_port_for(caller_relay_port).unwrap_or(caller_relay_port);
        let gateway_relay_port = rtp_port_for(gateway_relay_port).unwrap_or(gateway_relay_port);
        fs::create_dir_all(&config.recording_dir).map_err(recording_error)?;

        let stem = recording_file_stem(call_id);
        let wav_path = config.recording_dir.join(format!("{stem}.wav"));
        let metadata_path = config.recording_dir.join(format!("{stem}.json"));
        let recorder = Arc::new(Mutex::new(
            WavCallRecorder::create(wav_path.clone()).map_err(recording_error)?,
        ));
        write_recording_metadata(
            &metadata_path,
            call_id,
            &wav_path,
            caller_relay_port,
            gateway_relay_port,
        )
        .map_err(recording_error)?;

        self.recordings.insert(
            caller_relay_port,
            RecordingLeg {
                recorder: recorder.clone(),
                channel: RecordingChannel::Caller,
            },
        );
        self.recordings.insert(
            gateway_relay_port,
            RecordingLeg {
                recorder,
                channel: RecordingChannel::Gateway,
            },
        );

        Ok(Some(wav_path))
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

    fn record_metric(&self, relay_port: u16, update: impl FnOnce(&mut MediaRelayMetrics)) {
        update(self.metrics.entry(relay_port).or_default().value_mut());
    }

    fn record_rtcp_reports(&self, relay_port: u16, summary: &MediaPacketSummary) {
        if summary.rtcp_quality.reports == 0 {
            return;
        }

        self.record_metric(relay_port, |metrics| {
            metrics.rtcp_quality.merge(summary.rtcp_quality);
        });
    }

    fn record_rtp_packet(&self, relay_port: u16, packet: &RtpPacket) -> Result<bool, MediaError> {
        let recording = self.recordings.get(&relay_port).map(|v| v.clone());
        let Some(recording) = recording else {
            return Ok(false);
        };

        let mut recorder = recording.recorder.lock().expect("recording lock poisoned");
        recorder
            .record(recording.channel, packet)
            .map_err(recording_error)
    }

    fn process_dtmf_packet(&self, local_port: u16, packet: &RtpPacket) {
        let (call_id, last_timestamp) = {
            let Some(state) = self.dtmf_states.get(&local_port) else {
                return;
            };
            if packet.payload_type != state.payload_type {
                return;
            }
            (state.call_id.clone(), state.last_timestamp)
        };

        let Ok(event) = TelephoneEvent::parse(&packet.payload) else {
            return;
        };
        let Some(digit) = event.digit() else {
            return;
        };

        let timestamp = packet.timestamp;
        if Some(timestamp) != last_timestamp {
            if let Some(mut state) = self.dtmf_states.get_mut(&local_port) {
                state.last_timestamp = Some(timestamp);
            }
            let mut inner = self.state.lock().expect("media relay lock poisoned");
            let acc = inner.dtmf_accumulators.entry(call_id.clone()).or_default();
            acc.push(digit);
            let record =
                DtmfEventRecord::from_rtp(&call_id, digit, timestamp, event.duration, event.volume);
            inner
                .dtmf_event_log
                .entry(call_id.clone())
                .or_default()
                .push(record);
            drop(inner);
            self.metrics.entry(local_port).or_default().dtmf_events += 1;
            debug!(
                call_id,
                digit = %digit,
                timestamp,
                duration = event.duration,
                end = event.end,
                volume = event.volume,
                "reconstructed RTP DTMF digit"
            );
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SymmetricSourceUpdate {
    source_port: u16,
    target_port: u16,
    previous_target: Option<SocketAddr>,
    learned_target: SocketAddr,
}

#[derive(Debug, Clone)]
struct RecordingLeg {
    recorder: Arc<Mutex<WavCallRecorder>>,
    channel: RecordingChannel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RecordingChannel {
    Caller,
    Gateway,
}

impl RecordingChannel {
    fn index(self) -> usize {
        match self {
            Self::Caller => 0,
            Self::Gateway => 1,
        }
    }

    fn sample_offset(self) -> usize {
        self.index() * 2
    }
}

#[derive(Debug)]
struct WavCallRecorder {
    file: File,
    frames_written: u64,
    base_timestamps: [Option<u32>; 2],
    frames_since_flush: u64,
}

const FLUSH_INTERVAL: u64 = 100; // flush every 100 frames (2s at 50fps)

impl WavCallRecorder {
    fn create(path: PathBuf) -> io::Result<Self> {
        let mut file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)?;
        write_wav_header(&mut file, 0)?;
        Ok(Self {
            file,
            frames_written: 0,
            base_timestamps: [None, None],
            frames_since_flush: 0,
        })
    }

    fn record(&mut self, channel: RecordingChannel, packet: &RtpPacket) -> io::Result<bool> {
        let codec = match AudioCodec::from_static_payload_type(packet.payload_type) {
            Some(c) => c,
            None => return Ok(false),
        };
        if packet.payload.is_empty() {
            return Ok(false);
        }

        let num_samples = packet.payload.len();
        let start_frame = self.start_frame(channel, packet.timestamp);
        self.ensure_frames(start_frame + num_samples as u64)?;

        let base_offset = WAV_HEADER_LEN + start_frame * u64::from(RECORDING_CHANNELS) * 2;
        let sample_offset = channel.sample_offset();
        self.file
            .seek(SeekFrom::Start(base_offset + sample_offset as u64))?;

        // Decode and write samples inline (no Vec allocation)
        for &payload_byte in &packet.payload {
            let sample = match codec {
                AudioCodec::Pcmu => decode_pcmu(payload_byte),
                AudioCodec::Pcma => decode_pcma(payload_byte),
            };
            let bytes = sample.to_le_bytes();
            self.file.write_all(&bytes)?;
            if RECORDING_CHANNELS > 1 {
                let skip = 2_usize * (RECORDING_CHANNELS as usize - 1);
                let mut skip_buf = [0_u8; 8];
                self.file.read_exact(&mut skip_buf[..skip])?;
                self.file.seek(SeekFrom::Current(-(skip as i64)))?;
            }
        }

        self.frames_since_flush += num_samples as u64;
        if self.frames_since_flush >= FLUSH_INTERVAL {
            self.refresh_header()?;
            self.flush()?;
            self.frames_since_flush = 0;
        } else {
            // Update header but don't flush yet
            self.update_header_only()?;
        }
        Ok(true)
    }

    fn start_frame(&mut self, channel: RecordingChannel, timestamp: u32) -> u64 {
        let base = self.base_timestamps[channel.index()].get_or_insert(timestamp);
        u64::from(timestamp.wrapping_sub(*base))
    }

    fn ensure_frames(&mut self, target_frames: u64) -> io::Result<()> {
        if self.frames_written >= target_frames {
            return Ok(());
        }

        self.file.seek(SeekFrom::End(0))?;
        let missing_frames = target_frames - self.frames_written;
        let silence = [0_u8; 4];
        // Batch write silence in chunks instead of byte-by-byte
        const SILENCE_CHUNK: usize = 640; // 10ms at 8kHz stereo
        let mut remaining = missing_frames as usize;
        while remaining > 0 {
            let chunk = remaining.min(SILENCE_CHUNK);
            for _ in 0..chunk {
                self.file.write_all(&silence)?;
            }
            remaining -= chunk;
        }
        self.frames_written = target_frames;
        Ok(())
    }

    fn refresh_header(&mut self) -> io::Result<()> {
        let data_bytes = u32::try_from(self.frames_written * u64::from(RECORDING_CHANNELS) * 2)
            .map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "WAV recording is too large")
            })?;
        self.file.seek(SeekFrom::Start(0))?;
        write_wav_header(&mut self.file, data_bytes)?;
        self.file.seek(SeekFrom::End(0))?;
        Ok(())
    }

    fn update_header_only(&mut self) -> io::Result<()> {
        self.refresh_header()
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

fn write_wav_header(file: &mut File, data_bytes: u32) -> io::Result<()> {
    let byte_rate = RECORDING_SAMPLE_RATE
        * u32::from(RECORDING_CHANNELS)
        * u32::from(RECORDING_BITS_PER_SAMPLE)
        / 8;
    let block_align = RECORDING_CHANNELS * RECORDING_BITS_PER_SAMPLE / 8;
    let riff_size = 36_u32.saturating_add(data_bytes);

    file.write_all(b"RIFF")?;
    file.write_all(&riff_size.to_le_bytes())?;
    file.write_all(b"WAVE")?;
    file.write_all(b"fmt ")?;
    file.write_all(&16_u32.to_le_bytes())?;
    file.write_all(&1_u16.to_le_bytes())?;
    file.write_all(&RECORDING_CHANNELS.to_le_bytes())?;
    file.write_all(&RECORDING_SAMPLE_RATE.to_le_bytes())?;
    file.write_all(&byte_rate.to_le_bytes())?;
    file.write_all(&block_align.to_le_bytes())?;
    file.write_all(&RECORDING_BITS_PER_SAMPLE.to_le_bytes())?;
    file.write_all(b"data")?;
    file.write_all(&data_bytes.to_le_bytes())?;
    Ok(())
}

fn decode_pcmu(sample: u8) -> i16 {
    let sample = !sample;
    let sign = sample & 0x80;
    let exponent = (sample >> 4) & 0x07;
    let mantissa = sample & 0x0f;
    let magnitude = (((i16::from(mantissa)) << 3) + 0x84) << exponent;

    if sign != 0 {
        0x84 - magnitude
    } else {
        magnitude - 0x84
    }
}

fn decode_pcma(sample: u8) -> i16 {
    let sample = sample ^ 0x55;
    let sign = sample & 0x80;
    let exponent = (sample & 0x70) >> 4;
    let mantissa = sample & 0x0f;
    let magnitude = if exponent == 0 {
        (i16::from(mantissa) << 4) + 8
    } else {
        ((i16::from(mantissa) << 4) + 0x108) << (exponent - 1)
    };

    if sign != 0 {
        magnitude
    } else {
        -magnitude
    }
}

fn recording_file_stem(call_id: &str) -> String {
    let sanitized = call_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    format!("{}-{}", sanitized, unix_timestamp_millis())
}

fn unix_timestamp_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis()
}

fn write_recording_metadata(
    path: &Path,
    call_id: &str,
    wav_path: &Path,
    caller_relay_port: u16,
    gateway_relay_port: u16,
) -> io::Result<()> {
    let metadata = format!(
        concat!(
            "{{\n",
            "  \"call_id\": \"{}\",\n",
            "  \"wav_path\": \"{}\",\n",
            "  \"sample_rate\": {},\n",
            "  \"channels\": {},\n",
            "  \"caller_relay_port\": {},\n",
            "  \"gateway_relay_port\": {},\n",
            "  \"created_at_unix_ms\": {}\n",
            "}}\n"
        ),
        json_escape(call_id),
        json_escape(&wav_path.display().to_string()),
        RECORDING_SAMPLE_RATE,
        RECORDING_CHANNELS,
        caller_relay_port,
        gateway_relay_port,
        unix_timestamp_millis()
    );
    fs::write(path, metadata)
}

fn json_escape(value: &str) -> String {
    value
        .chars()
        .flat_map(|ch| match ch {
            '"' => "\\\"".chars().collect::<Vec<_>>(),
            '\\' => "\\\\".chars().collect::<Vec<_>>(),
            '\n' => "\\n".chars().collect::<Vec<_>>(),
            '\r' => "\\r".chars().collect::<Vec<_>>(),
            '\t' => "\\t".chars().collect::<Vec<_>>(),
            ch => vec![ch],
        })
        .collect()
}

fn recording_error(error: io::Error) -> MediaError {
    MediaError::Recording(error.to_string())
}

impl Default for MediaRelayState {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaError {
    InvalidUtf8,
    InvalidEndpoint(String),
    PortRangeExhausted { port_min: u16, port_max: u16 },
    Recording(String),
    Sdp(SdpError),
    Io(String),
}

impl fmt::Display for MediaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidUtf8 => write!(f, "SDP body is not valid UTF-8"),
            Self::InvalidEndpoint(endpoint) => write!(f, "invalid RTP endpoint: {endpoint}"),
            Self::PortRangeExhausted { port_min, port_max } => {
                write!(f, "RTP port range exhausted: {port_min}-{port_max}")
            }
            Self::Recording(error) => write!(f, "recording error: {error}"),
            Self::Sdp(error) => write!(f, "{error}"),
            Self::Io(error) => write!(f, "media IO error: {error}"),
        }
    }
}

impl Error for MediaError {}

impl From<SdpError> for MediaError {
    fn from(error: SdpError) -> Self {
        Self::Sdp(error)
    }
}

pub fn is_sdp_body(headers: &HeaderMap, body: &[u8]) -> bool {
    if body.is_empty() {
        return false;
    }
    // Fast byte-level check: look for "application/sdp" without String allocation
    if let Some(ct) = headers.get("content-type") {
        let raw = ct.as_str().as_bytes();
        // Check for "application/sdp" at start or after "; "
        if raw.len() >= 15 {
            if raw[..15].eq_ignore_ascii_case(b"application/sdp") {
                return true;
            }
            // Check after "; " separator
            for i in 1..raw.len().saturating_sub(15) {
                if raw[i] == b';' && raw[i + 1] == b' ' {
                    let rest = &raw[i + 2..];
                    if rest.len() >= 15 && rest[..15].eq_ignore_ascii_case(b"application/sdp") {
                        return true;
                    }
                }
            }
        }
    }
    false
}

#[allow(dead_code)]
pub fn rewrite_sdp_body(body: &[u8], endpoint: RtpEndpoint) -> Result<Vec<u8>, MediaError> {
    let input = str::from_utf8(body).map_err(|_| MediaError::InvalidUtf8)?;

    // Fast path: direct byte replacement when SDP has compatible codecs
    if let Some(result) = try_fast_rewrite(input, &endpoint) {
        return Ok(result);
    }

    // Slow path: full parse-modify-serialize
    let mut session = SessionDescription::parse(input)?;
    let payloads = compatible_audio_payloads(&session)?;
    session.retain_first_audio_rtp_payloads(&payloads)?;
    session.rewrite_first_audio_rtp_endpoint(endpoint)?;
    Ok(session.to_bytes())
}

/// Rewrite SDP body AND extract the original remote RTP endpoint in one pass.
/// Returns (rewritten_body, original_endpoint).
pub fn rewrite_sdp_and_extract_endpoint(
    body: &[u8],
    relay_endpoint: &RtpEndpoint,
) -> Result<(Vec<u8>, RtpEndpoint), MediaError> {
    let input = str::from_utf8(body).map_err(|_| MediaError::InvalidUtf8)?;

    // Fast path: single-pass rewrite + extract
    if let Some(result) = try_fast_rewrite_and_extract(input, relay_endpoint) {
        return Ok(result);
    }

    // Slow path
    let mut session = SessionDescription::parse(input)?;
    let original_endpoint = session.first_audio_rtp_endpoint()?;
    let payloads = compatible_audio_payloads(&session)?;
    session.retain_first_audio_rtp_payloads(&payloads)?;
    session.rewrite_first_audio_rtp_endpoint(relay_endpoint.clone())?;
    Ok((session.to_bytes(), original_endpoint))
}

/// Fast path: rewrite c= and m= lines directly without full SDP parsing.
/// Returns None if the SDP doesn't look like a standard audio SDP.
#[allow(dead_code)]
fn try_fast_rewrite(input: &str, endpoint: &RtpEndpoint) -> Option<Vec<u8>> {
    try_fast_rewrite_inner(input, endpoint).map(|(bytes, _)| bytes)
}

/// Fast path: rewrite AND extract original endpoint in one pass.
fn try_fast_rewrite_and_extract(
    input: &str,
    endpoint: &RtpEndpoint,
) -> Option<(Vec<u8>, RtpEndpoint)> {
    try_fast_rewrite_inner(input, endpoint)
}

/// Core fast path: rewrite c= and m= lines + extract original endpoint.
fn try_fast_rewrite_inner(input: &str, endpoint: &RtpEndpoint) -> Option<(Vec<u8>, RtpEndpoint)> {
    // Check if this SDP has compatible audio codecs (PCMU or PCMA)
    if !input.contains("PCMU") && !input.contains("PCMA") {
        return None;
    }

    let mut found_audio_m = false;
    let mut session_c_rewritten = false;
    let mut result = Vec::with_capacity(input.len() + 64);
    let mut in_audio_section = false;
    let mut original_port: Option<u16> = None;
    let mut original_addr: Option<String> = None;

    for line in input.lines() {
        let trimmed = line.trim_end_matches('\r');

        if trimmed.starts_with("m=audio ") {
            found_audio_m = true;
            in_audio_section = true;
            // Rewrite m=audio line: replace port, extract original port
            if let Some(rest) = trimmed.get(8..) {
                if let Some(space2) = rest.find(' ') {
                    let port_str = &rest[..space2];
                    if original_port.is_none() {
                        original_port = port_str.parse().ok();
                    }
                    result.extend_from_slice(b"m=audio ");
                    result.extend_from_slice(endpoint.port.to_string().as_bytes());
                    result.extend_from_slice(&rest.as_bytes()[space2..]);
                    result.extend_from_slice(b"\r\n");
                    continue;
                }
            }
        } else if trimmed.starts_with("m=") {
            in_audio_section = false;
        }

        // Rewrite c= line AND extract original address
        if trimmed.starts_with("c=IN IP") {
            // Extract original address for endpoint extraction
            if original_addr.is_none() {
                if let Some(rest) = trimmed.get(7..) {
                    if let Some(addr) = rest.split_whitespace().nth(1) {
                        original_addr = Some(addr.to_string());
                    }
                }
            }

            // Rewrite if needed
            if in_audio_section || (!found_audio_m && !session_c_rewritten) {
                if !found_audio_m {
                    session_c_rewritten = true;
                }
                let addr_type = if endpoint.address.contains(':') {
                    "IP6"
                } else {
                    "IP4"
                };
                result.extend_from_slice(b"c=IN ");
                result.extend_from_slice(addr_type.as_bytes());
                result.extend_from_slice(b" ");
                result.extend_from_slice(endpoint.address.as_bytes());
                result.extend_from_slice(b"\r\n");
                continue;
            }
        }

        result.extend_from_slice(line.as_bytes());
        if !line.ends_with('\n') {
            result.extend_from_slice(b"\r\n");
        }
    }

    if found_audio_m {
        let original_endpoint = RtpEndpoint {
            address: original_addr.unwrap_or_else(|| "0.0.0.0".to_string()),
            port: original_port.unwrap_or(0),
        };
        Some((result, original_endpoint))
    } else {
        None
    }
}

pub fn parse_sdp_rtp_endpoint(body: &[u8]) -> Result<RtpEndpoint, MediaError> {
    let input = str::from_utf8(body).map_err(|_| MediaError::InvalidUtf8)?;

    // Fast path: scan for m=audio and c= lines without full parsing
    if let Some(endpoint) = try_fast_parse_endpoint(input) {
        return Ok(endpoint);
    }

    // Slow path: full parse
    let session = SessionDescription::parse(input)?;
    Ok(session.first_audio_rtp_endpoint()?)
}

/// Fast path: extract audio RTP endpoint by scanning for m=audio and c= lines.
fn try_fast_parse_endpoint(input: &str) -> Option<RtpEndpoint> {
    let mut audio_port: Option<u16> = None;
    let mut connection_addr: Option<&str> = None;
    let mut in_audio_section = false;

    for line in input.lines() {
        let trimmed = line.trim_end_matches('\r');

        if let Some(rest) = trimmed.strip_prefix("m=audio ") {
            let port_str = rest.split_whitespace().next()?;
            audio_port = Some(port_str.parse().ok()?);
            in_audio_section = true;
        } else if trimmed.starts_with("m=") {
            in_audio_section = false;
        } else if trimmed.starts_with("c=IN IP")
            && (in_audio_section || (audio_port.is_none() && connection_addr.is_none()))
        {
            let rest = &trimmed[7..];
            connection_addr = rest.split_whitespace().nth(1);
        }
    }

    let port = audio_port?;
    let address = connection_addr.unwrap_or("0.0.0.0").to_string();

    Some(RtpEndpoint { address, port })
}

pub fn parse_sdp_dtmf_payload_type(body: &[u8]) -> Option<u8> {
    let input = str::from_utf8(body).ok()?;
    let session = SessionDescription::parse(input).ok()?;
    let formats = session.first_audio_rtp_formats().ok()?;
    for format in formats {
        if let Some(encoding_name) = &format.encoding_name {
            if encoding_name.eq_ignore_ascii_case("telephone-event")
                && format.clock_rate == Some(8000)
            {
                return format.payload_type.parse::<u8>().ok();
            }
        }
    }
    None
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
        let relay_clone = MediaRelayState {
            targets: Arc::clone(&relay.targets),
            peer_ports: Arc::clone(&relay.peer_ports),
            metrics: Arc::clone(&relay.metrics),
            recordings: Arc::clone(&relay.recordings),
            dtmf_states: Arc::clone(&relay.dtmf_states),
            active_loops: Arc::clone(&relay.active_loops),
            state: Arc::clone(&relay.state),
        };
        let handle = tokio::spawn(relay_media_port(
            socket,
            port,
            relay_clone,
            config.symmetric_rtp_learning,
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

    fn inspect(self, packet: &[u8]) -> Result<MediaPacketSummary, rtp_core::RtpError> {
        match self {
            Self::Rtp => RtpPacket::parse(packet).map(|rtp_packet| MediaPacketSummary {
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
struct MediaPacketSummary {
    rtp_packet: Option<RtpPacket>,
    rtcp_quality: RtcpQualitySnapshot,
}

fn rtcp_summary(packets: &[RtcpPacket]) -> Result<MediaPacketSummary, rtp_core::RtpError> {
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

fn compact_ntp_middle_32_now() -> u32 {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO);
    let ntp_seconds = duration.as_secs().wrapping_add(2_208_988_800);
    let ntp_fraction = (u64::from(duration.subsec_nanos()) << 32) / 1_000_000_000;
    let ntp_timestamp = (ntp_seconds << 32) | ntp_fraction;
    ((ntp_timestamp >> 16) & u64::from(u32::MAX)) as u32
}

fn rtt_millis_from_compact_ntp(
    arrival_ntp_middle_32: u32,
    last_sender_report: u32,
    delay_since_last_sender_report: u32,
) -> Option<u32> {
    if last_sender_report == 0 || delay_since_last_sender_report == 0 {
        return None;
    }

    let rtt_units = arrival_ntp_middle_32
        .wrapping_sub(last_sender_report)
        .wrapping_sub(delay_since_last_sender_report);
    let millis = ((u64::from(rtt_units) * 1_000) + 32_768) / 65_536;
    Some(u32::try_from(millis).unwrap_or(u32::MAX))
}

async fn relay_media_port(
    socket: UdpSocket,
    local_port: u16,
    relay: MediaRelayState,
    symmetric_rtp_learning: bool,
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

        let packet = &buffer[..size];
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

        if let Some(s) = &summary {
            if let Some(rtp_packet) = s.rtp_packet.as_ref() {
                relay.process_dtmf_packet(local_port, rtp_packet);
                match relay.record_rtp_packet(local_port, rtp_packet) {
                    Ok(true) => {
                        relay.record_metric(local_port, |metrics| metrics.recorded_packets += 1);
                    }
                    Ok(false) => {}
                    Err(error) => {
                        relay.record_metric(local_port, |metrics| metrics.recording_errors += 1);
                        warn!(%error, %source, local_port, "failed to record RTP packet");
                    }
                }
            }
        }

        if let Err(error) = socket.send_to(packet, target).await {
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

fn env_port(name: &str) -> Option<u16> {
    env::var(name).ok()?.parse().ok()
}

fn env_bool(name: &str) -> Option<bool> {
    let value = env::var(name).ok()?;
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn even_port_at_or_above(port: u16) -> Option<u16> {
    if port % 2 == 0 {
        Some(port)
    } else {
        port.checked_add(1)
    }
}

fn even_port_at_or_below(port: u16) -> Option<u16> {
    if port % 2 == 0 {
        Some(port)
    } else {
        port.checked_sub(1)
    }
}

fn socket_addr_for_endpoint(endpoint: &RtpEndpoint) -> Result<SocketAddr, MediaError> {
    let target = if endpoint.address.contains(':') {
        format!("[{}]:{}", endpoint.address, endpoint.port)
    } else {
        format!("{}:{}", endpoint.address, endpoint.port)
    };

    target
        .to_socket_addrs()
        .map_err(|_| MediaError::InvalidEndpoint(target.clone()))?
        .next()
        .ok_or(MediaError::InvalidEndpoint(target))
}

fn compatible_audio_payloads(session: &SessionDescription) -> Result<Vec<String>, MediaError> {
    let formats = session.first_audio_rtp_formats()?;

    // Single pass: collect compatible voice codecs + telephone-event
    let mut payloads = Vec::with_capacity(formats.len());
    let mut has_voice = false;

    for format in &formats {
        let is_voice = match (format.encoding_name.as_deref(), format.clock_rate) {
            (Some(name), Some(rate)) => AudioCodec::from_rtpmap(name, rate).is_some(),
            _ => format
                .payload_type
                .parse::<u8>()
                .ok()
                .and_then(AudioCodec::from_static_payload_type)
                .is_some(),
        };

        if is_voice {
            has_voice = true;
            payloads.push(format.payload_type.clone());
        } else if let Some(name) = &format.encoding_name {
            if name.eq_ignore_ascii_case("telephone-event") && format.clock_rate == Some(8000) {
                payloads.push(format.payload_type.clone());
            }
        }
    }

    if !has_voice {
        return Ok(Vec::new());
    }

    Ok(payloads)
}

#[cfg(test)]
mod tests {
    use super::{
        is_sdp_body, parse_sdp_rtp_endpoint, rewrite_sdp_body, spawn_rtp_relay_listeners,
        MediaConfig, MediaError, MediaRelayMetrics, MediaRelayState,
    };
    use sdp_core::RtpEndpoint;
    use sip_core::{HeaderMap, HeaderName, HeaderValue};
    use std::{fs, net::SocketAddr, path::PathBuf};
    use tokio::net::UdpSocket;
    use tokio::time::{sleep, timeout, Duration};

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
            super::rtt_millis_from_compact_ntp(0x0003_0000, 0x0001_0000, 0x0001_0000),
            Some(1_000)
        );
        assert_eq!(
            super::rtt_millis_from_compact_ntp(0x0003_0000, 0, 0x0001_0000),
            None
        );
        assert_eq!(
            super::rtt_millis_from_compact_ntp(0x0003_0000, 0x0001_0000, 0),
            None
        );
    }

    #[test]
    fn decodes_g711_static_payloads_to_pcm() {
        assert_eq!(super::decode_pcmu(0xff), 0);
        assert_eq!(super::decode_pcmu(0x7f), 0);
        assert_eq!(super::decode_pcma(0xd5), 8);
        assert_eq!(super::decode_pcma(0x55), -8);
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
        let caller_packet = rtp_core::RtpPacket::parse(&caller_packet).unwrap();
        assert!(relay.record_rtp_packet(40_000, &caller_packet).unwrap());

        let gateway_packet = rtp_core::RtpPacket::new(8, 1, 0, 24, vec![0xd5, 0xd5])
            .unwrap()
            .encode()
            .unwrap();
        let gateway_packet = rtp_core::RtpPacket::parse(&gateway_packet).unwrap();
        assert!(relay.record_rtp_packet(40_002, &gateway_packet).unwrap());

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

        let metadata_path = wav_path.with_extension("json");
        let metadata = fs::read_to_string(metadata_path).unwrap();
        assert!(metadata.contains("\"call_id\": \"call/with:unsafe@example.com\""));
        assert!(metadata.contains("\"caller_relay_port\": 40000"));
        assert!(metadata.contains("\"gateway_relay_port\": 40002"));
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
        assert_eq!(relay.metrics_totals(), metrics);

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
        let packet1 = rtp_core::RtpPacket::new(pt, 1, 1000, 42, payload1).unwrap();
        relay.process_dtmf_packet(local_port, &packet1);

        // Send a duplicate packet of '5' (same timestamp)
        let payload2 = vec![5, 0x80, 0, 240]; // E=1
        let packet2 = rtp_core::RtpPacket::new(pt, 2, 1000, 42, payload2).unwrap();
        relay.process_dtmf_packet(local_port, &packet2);

        // 3. Simulate sending DTMF digit '#' (Event=11) with timestamp 2000
        let payload3 = vec![11, 0, 0, 80];
        let packet3 = rtp_core::RtpPacket::new(pt, 3, 2000, 42, payload3).unwrap();
        relay.process_dtmf_packet(local_port, &packet3);

        // 4. Verify reconstructed digits
        let digits = relay.get_dtmf_digits(call_id).unwrap();
        assert_eq!(digits, "5#");
        assert_eq!(relay.metrics_for_port(local_port).dtmf_events, 2);

        // 5. Clear digits and verify
        relay.clear_dtmf_digits(call_id);
        assert!(relay.get_dtmf_digits(call_id).is_none());
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
