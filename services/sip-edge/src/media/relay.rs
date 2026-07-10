use cdr_core::DtmfEventRecord;
use dashmap::DashMap;
use rtp_core::{
    AudioCodec, RtcpPacket, RtcpReportBlock, RtpPacketView, SrtpConfig, SrtpContext, SrtpError,
    TelephoneEvent,
};
use sdp_core::{RtpEndpoint, SdpError, SessionDescription};
use sip_core::HeaderMap;
use std::{
    collections::{HashMap, HashSet},
    env,
    error::Error,
    ffi::CString,
    fmt, fs,
    fs::File,
    io,
    io::{Seek, SeekFrom, Write},
    mem::MaybeUninit,
    net::{SocketAddr, ToSocketAddrs},
    path::{Path, PathBuf},
    str,
    sync::{
        atomic::{AtomicU64, AtomicUsize, Ordering},
        mpsc, Arc, Mutex,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{net::UdpSocket, task::JoinHandle};
use tracing::{debug, warn};

pub const RTP_ADVERTISED_ADDR_ENV: &str = "VOS_RS_RTP_ADVERTISED_ADDR";
pub const RTP_PORT_MIN_ENV: &str = "VOS_RS_RTP_PORT_MIN";
pub const RTP_PORT_MAX_ENV: &str = "VOS_RS_RTP_PORT_MAX";
pub const RTP_SYMMETRIC_LEARNING_ENV: &str = "VOS_RS_RTP_SYMMETRIC_LEARNING";
pub const RTP_ANTI_SPOOFING_ENV: &str = "VOS_RS_RTP_ANTI_SPOOFING";
pub const RTP_SOURCE_RELEARN_SECS_ENV: &str = "VOS_RS_RTP_SOURCE_RELEARN_SECS";
pub const RECORDING_ENABLED_ENV: &str = "VOS_RS_RECORDING_ENABLED";
pub const RECORDING_DIR_ENV: &str = "VOS_RS_RECORDING_DIR";
pub const RECORDING_RETENTION_SECS_ENV: &str = "VOS_RS_RECORDING_RETENTION_SECS";
pub const RECORDING_MIN_FREE_BYTES_ENV: &str = "VOS_RS_RECORDING_MIN_FREE_BYTES";
pub const RECORDING_MAX_FILE_BYTES_ENV: &str = "VOS_RS_RECORDING_MAX_FILE_BYTES";
pub const RECORDING_MAX_DURATION_SECS_ENV: &str = "VOS_RS_RECORDING_MAX_DURATION_SECS";
pub const DEFAULT_RTP_ADVERTISED_ADDR: &str = "127.0.0.1";
pub const DEFAULT_RTP_PORT_MIN: u16 = 40_000;
pub const DEFAULT_RTP_PORT_MAX: u16 = 40_100;
pub const DEFAULT_RTP_SYMMETRIC_LEARNING: bool = true;
pub const DEFAULT_RTP_ANTI_SPOOFING: bool = true;
pub const DEFAULT_RTP_SOURCE_RELEARN_SECS: u64 = 30;
pub const DEFAULT_RECORDING_ENABLED: bool = false;
pub const DEFAULT_RECORDING_DIR: &str = "target/recordings";
pub const DEFAULT_RECORDING_RETENTION_SECS: u64 = 7 * 24 * 60 * 60;
pub const DEFAULT_RECORDING_MIN_FREE_BYTES: u64 = 512 * 1024 * 1024;
pub const DEFAULT_RECORDING_MAX_FILE_BYTES: u64 = 128 * 1024 * 1024;
pub const DEFAULT_RECORDING_MAX_DURATION_SECS: u64 = 60 * 60;
const MAX_RTP_DATAGRAM_SIZE: usize = 65_535;
const RECORDING_SAMPLE_RATE: u32 = 8_000;
const RECORDING_CHANNELS: u16 = 2;
const RECORDING_BITS_PER_SAMPLE: u16 = 16;
const DEFAULT_RECORDING_QUEUE_CAPACITY: usize = 4096;
const RECORDING_QUEUE_CAPACITY_ENV: &str = "VOS_RS_RECORDING_QUEUE_CAPACITY";
const RECORDING_WORKERS_ENV: &str = "VOS_RS_RECORDING_WORKERS";
const DEFAULT_RECORDING_WORKERS: usize = 4;
const RTCP_QUALITY_WINDOW_MS: u128 = 60_000;
const RECORDING_WORKER_DRAIN_LIMIT: usize = 256;
static NEXT_RECORDING_SESSION_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaConfig {
    pub advertised_addr: String,
    pub port_min: u16,
    pub port_max: u16,
    pub symmetric_rtp_learning: bool,
    pub anti_spoofing: bool,
    pub source_relearn_after_secs: u64,
    pub recording_enabled: bool,
    pub recording_dir: PathBuf,
    pub recording_retention_secs: u64,
    pub recording_min_free_bytes: u64,
    pub recording_max_file_bytes: u64,
    pub recording_max_duration_secs: u64,
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
            anti_spoofing: DEFAULT_RTP_ANTI_SPOOFING,
            source_relearn_after_secs: DEFAULT_RTP_SOURCE_RELEARN_SECS,
            recording_enabled: DEFAULT_RECORDING_ENABLED,
            recording_dir: PathBuf::from(DEFAULT_RECORDING_DIR),
            recording_retention_secs: DEFAULT_RECORDING_RETENTION_SECS,
            recording_min_free_bytes: DEFAULT_RECORDING_MIN_FREE_BYTES,
            recording_max_file_bytes: DEFAULT_RECORDING_MAX_FILE_BYTES,
            recording_max_duration_secs: DEFAULT_RECORDING_MAX_DURATION_SECS,
        }
    }

    pub fn from_env() -> Self {
        let advertised_addr = env::var(RTP_ADVERTISED_ADDR_ENV)
            .unwrap_or_else(|_| DEFAULT_RTP_ADVERTISED_ADDR.to_string());
        let port_min = env_port(RTP_PORT_MIN_ENV).unwrap_or(DEFAULT_RTP_PORT_MIN);
        let port_max = env_port(RTP_PORT_MAX_ENV).unwrap_or(DEFAULT_RTP_PORT_MAX);
        let symmetric_rtp_learning =
            env_bool(RTP_SYMMETRIC_LEARNING_ENV).unwrap_or(DEFAULT_RTP_SYMMETRIC_LEARNING);
        let anti_spoofing = env_bool(RTP_ANTI_SPOOFING_ENV).unwrap_or(DEFAULT_RTP_ANTI_SPOOFING);
        let source_relearn_after_secs = env::var(RTP_SOURCE_RELEARN_SECS_ENV)
            .ok()
            .and_then(|value| value.trim().parse::<u64>().ok())
            .unwrap_or(DEFAULT_RTP_SOURCE_RELEARN_SECS);
        let recording_enabled =
            env_bool(RECORDING_ENABLED_ENV).unwrap_or(DEFAULT_RECORDING_ENABLED);
        let recording_dir =
            env::var(RECORDING_DIR_ENV).unwrap_or_else(|_| DEFAULT_RECORDING_DIR.to_string());
        let recording_retention_secs =
            env_u64(RECORDING_RETENTION_SECS_ENV).unwrap_or(DEFAULT_RECORDING_RETENTION_SECS);
        let recording_min_free_bytes =
            env_u64(RECORDING_MIN_FREE_BYTES_ENV).unwrap_or(DEFAULT_RECORDING_MIN_FREE_BYTES);
        let recording_max_file_bytes =
            env_u64(RECORDING_MAX_FILE_BYTES_ENV).unwrap_or(DEFAULT_RECORDING_MAX_FILE_BYTES);
        let recording_max_duration_secs =
            env_u64(RECORDING_MAX_DURATION_SECS_ENV).unwrap_or(DEFAULT_RECORDING_MAX_DURATION_SECS);
        let mut config = Self::new_with_symmetric_learning(
            advertised_addr,
            port_min,
            port_max,
            symmetric_rtp_learning,
        );
        config.recording_enabled = recording_enabled;
        config.anti_spoofing = anti_spoofing;
        config.source_relearn_after_secs = source_relearn_after_secs;
        config.recording_dir = PathBuf::from(recording_dir);
        config.recording_retention_secs = recording_retention_secs;
        config.recording_min_free_bytes = recording_min_free_bytes;
        config.recording_max_file_bytes = recording_max_file_bytes;
        config.recording_max_duration_secs = recording_max_duration_secs;
        config
    }

    #[cfg(test)]
    pub fn with_recording(mut self, enabled: bool, dir: impl Into<PathBuf>) -> Self {
        self.recording_enabled = enabled;
        self.recording_dir = dir.into();
        self.recording_retention_secs = 0;
        self.recording_min_free_bytes = 0;
        self.recording_max_file_bytes = 0;
        self.recording_max_duration_secs = 0;
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
    recording_pool: Arc<RecordingPool>,
    dtmf_states: Arc<DashMap<u16, DtmfState>>,
    active_loops: Arc<DashMap<u16, Vec<tokio::sync::oneshot::Sender<()>>>>,
    crypto_sessions: Arc<DashMap<u16, Arc<tokio::sync::Mutex<MediaCryptoSession>>>>,
    pending_srtp: Arc<DashMap<u16, PendingSrtpConfig>>,
    rtp_stats: Arc<DashMap<u16, RtpReceiveStats>>,
    source_bindings: Arc<DashMap<u16, SourceBinding>>,
    state: Arc<Mutex<MediaRelayStateInner>>,
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
struct MediaRelayStateInner {
    next_port: u16,
    leased_rtp_ports: HashSet<u16>,
    recording_dirs: HashSet<PathBuf>,
    dtmf_accumulators: HashMap<String, String>,
    dtmf_event_log: HashMap<String, Vec<DtmfEventRecord>>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct MediaRelayMetrics {
    pub received_packets: u64,
    pub forwarded_packets: u64,
    pub dropped_invalid_packets: u64,
    pub dropped_no_target_packets: u64,
    pub send_errors: u64,
    pub learned_source_updates: u64,
    pub dropped_spoofed_packets: u64,
    pub rtcp_quality: RtcpQualitySnapshot,
    pub rtcp_window: RtcpQualityWindow,
    pub rtcp_quality_alerts: u64,
    pub rtcp_quality_degraded: bool,
    pub recorded_packets: u64,
    pub recording_dropped_packets: u64,
    pub recording_errors: u64,
    pub recording_queue_depth: u64,
    pub recording_queue_capacity: u64,
    pub recording_workers: u64,
    pub dtmf_events: u64,
}

/// Rolling RTCP quality aggregates for the current 60-second window.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct RtcpQualityWindow {
    pub started_at_unix_ms: u128,
    pub reports: u64,
    pub samples: u64,
    pub average_fraction_lost: Option<u8>,
    pub average_jitter: Option<u32>,
    pub average_rtt_ms: Option<u32>,
    pub r_factor_x100: Option<u16>,
    pub mos_x100: Option<u16>,
    total_fraction_lost: u64,
    total_jitter: u64,
    total_rtt_ms: u64,
    rtt_samples: u64,
}

impl RtcpQualityWindow {
    fn is_degraded(&self) -> bool {
        self.average_fraction_lost
            .map(|value| u32::from(value) * 10_000 / 255 > 1_000)
            .unwrap_or(false)
            || self
                .average_jitter
                .map(|value| value > 100 * 8)
                .unwrap_or(false)
            || self
                .average_rtt_ms
                .map(|value| value > 300)
                .unwrap_or(false)
            || self.mos_x100.map(|value| value < 350).unwrap_or(false)
    }

    fn observe(&mut self, snapshot: RtcpQualitySnapshot) {
        let now = unix_timestamp_millis();
        if self.started_at_unix_ms == 0
            || now.saturating_sub(self.started_at_unix_ms) >= RTCP_QUALITY_WINDOW_MS
        {
            *self = Self {
                started_at_unix_ms: now,
                ..Self::default()
            };
        }

        self.reports += snapshot.reports;
        let samples = snapshot.report_blocks;
        self.samples += samples;
        if let Some(value) = snapshot.last_fraction_lost {
            self.total_fraction_lost += u64::from(value) * samples.max(1);
        }
        if let Some(value) = snapshot.last_jitter {
            self.total_jitter += u64::from(value) * samples.max(1);
        }
        if let Some(value) = snapshot.last_rtt_ms {
            self.total_rtt_ms += u64::from(value);
            self.rtt_samples += 1;
        }
        self.recalculate();
    }

    fn merge(&mut self, other: Self) {
        if other.started_at_unix_ms == 0 {
            return;
        }
        if self.started_at_unix_ms == 0 {
            *self = other;
            return;
        }
        self.started_at_unix_ms = self.started_at_unix_ms.min(other.started_at_unix_ms);
        self.reports += other.reports;
        self.samples += other.samples;
        self.total_fraction_lost += other.total_fraction_lost;
        self.total_jitter += other.total_jitter;
        self.total_rtt_ms += other.total_rtt_ms;
        self.rtt_samples += other.rtt_samples;
        self.recalculate();
    }

    fn recalculate(&mut self) {
        if self.samples == 0 {
            return;
        }
        self.average_fraction_lost = Some((self.total_fraction_lost / self.samples) as u8);
        self.average_jitter = Some((self.total_jitter / self.samples) as u32);
        self.average_rtt_ms =
            (self.rtt_samples > 0).then_some((self.total_rtt_ms / self.rtt_samples) as u32);

        let loss_percent =
            f64::from(self.average_fraction_lost.unwrap_or_default()) * 100.0 / 255.0;
        let jitter_ms = f64::from(self.average_jitter.unwrap_or_default()) / 8.0;
        let rtt_ms = f64::from(self.average_rtt_ms.unwrap_or_default());
        let r_factor =
            (93.2 - 0.024 * rtt_ms - 0.11 * loss_percent - 0.01 * jitter_ms).clamp(0.0, 100.0);
        let mos =
            (1.0 + 0.035 * r_factor + 0.000007 * r_factor * (r_factor - 60.0) * (100.0 - r_factor))
                .clamp(1.0, 4.5);
        self.r_factor_x100 = Some((r_factor * 100.0).round() as u16);
        self.mos_x100 = Some((mos * 100.0).round() as u16);
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
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

#[derive(Debug, Clone)]
struct PendingSrtpConfig {
    suite: String,
    key_params: String,
}

#[derive(Debug, Clone, Copy)]
struct SourceBinding {
    address: SocketAddr,
    last_seen_unix_ms: u128,
}

#[derive(Debug, Clone, Copy, Default)]
struct RtpReceiveStats {
    ssrc: u32,
    base_sequence: u16,
    highest_sequence: u16,
    received: u32,
    jitter: u32,
    last_transit: Option<i64>,
    last_report_unix_ms: u128,
}

impl RtpReceiveStats {
    fn observe(&mut self, packet: RtpPacketView<'_>) {
        if self.received == 0 {
            self.ssrc = packet.ssrc;
            self.base_sequence = packet.sequence_number;
            self.highest_sequence = packet.sequence_number;
        } else if packet.sequence_number.wrapping_sub(self.highest_sequence) < 0x8000 {
            self.highest_sequence = packet.sequence_number;
        }
        self.received = self.received.saturating_add(1);

        let arrival_units = (unix_timestamp_millis() as i64).saturating_mul(8);
        let transit = arrival_units.saturating_sub(i64::from(packet.timestamp));
        if let Some(previous) = self.last_transit {
            let delta = (transit - previous).unsigned_abs();
            let jitter = u64::from(self.jitter);
            self.jitter =
                ((jitter.saturating_mul(15) + delta) / 16).min(u64::from(u32::MAX)) as u32;
        }
        self.last_transit = Some(transit);
    }

    fn receiver_report(&mut self) -> Option<Vec<u8>> {
        let now = unix_timestamp_millis();
        if self.received == 0 {
            return None;
        }
        if self.last_report_unix_ms == 0 {
            self.last_report_unix_ms = now;
            return None;
        }
        if now.saturating_sub(self.last_report_unix_ms) < 5_000 {
            return None;
        }
        self.last_report_unix_ms = now;
        let expected = u32::from(self.highest_sequence.wrapping_sub(self.base_sequence)) + 1;
        let lost = expected.saturating_sub(self.received);
        let fraction_lost = if expected == 0 {
            0
        } else {
            ((u64::from(lost) * 256) / u64::from(expected)).min(255) as u8
        };
        let mut payload = Vec::with_capacity(28);
        payload.extend_from_slice(&self.ssrc.wrapping_add(1).to_be_bytes());
        payload.extend_from_slice(&self.ssrc.to_be_bytes());
        payload.push(fraction_lost);
        let cumulative_lost = i32::try_from(lost).unwrap_or(i32::MAX).clamp(0, 0x7f_ffff);
        let lost_bytes = cumulative_lost.to_be_bytes();
        payload.extend_from_slice(&lost_bytes[1..]);
        payload.extend_from_slice(&u32::from(self.highest_sequence).to_be_bytes());
        payload.extend_from_slice(&self.jitter.to_be_bytes());
        payload.extend_from_slice(&0_u32.to_be_bytes());
        payload.extend_from_slice(&0_u32.to_be_bytes());
        RtcpPacket::new(1, rtp_core::RtcpPacketType::ReceiverReport, payload)
            .ok()?
            .encode()
            .ok()
    }
}

/// SRTP state for one RTP direction and SSRC.
#[allow(dead_code)]
#[derive(Debug)]
pub(crate) struct MediaCryptoSession {
    context: SrtpContext,
}

#[allow(dead_code)]
impl MediaCryptoSession {
    fn from_sdes(suite: &str, key_params: &str, ssrc: u32) -> Result<Self, SrtpError> {
        let config = SrtpConfig::from_sdes_key_params(suite, key_params)?;
        Ok(Self {
            context: SrtpContext::new(config, ssrc),
        })
    }

    fn encrypt(&mut self, packet: &mut Vec<u8>) -> Result<usize, SrtpError> {
        self.context.encrypt_rtp(packet)
    }

    fn decrypt(&mut self, packet: &mut [u8]) -> Result<usize, SrtpError> {
        self.context.decrypt_srtp(packet)
    }
}

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
        self.ensure_recording_dir(&config.recording_dir)
            .map_err(recording_error)?;
        self.enforce_recording_storage_policy(
            &config.recording_dir,
            config.recording_retention_secs,
            config.recording_min_free_bytes,
        )
        .map_err(recording_error)?;

        let stem = recording_file_stem(call_id);
        let wav_path = config.recording_dir.join(format!("{stem}.wav"));
        let session = Arc::new(RecordingSession::new(
            wav_path.clone(),
            config.recording_min_free_bytes,
            config.recording_max_file_bytes,
            config.recording_max_duration_secs,
            Arc::clone(&self.recording_pool),
        ));

        self.recordings.insert(
            caller_relay_port,
            RecordingLeg {
                session: session.clone(),
                channel: RecordingChannel::Caller,
            },
        );
        self.recordings.insert(
            gateway_relay_port,
            RecordingLeg {
                session,
                channel: RecordingChannel::Gateway,
            },
        );

        Ok(Some(wav_path))
    }

    fn ensure_recording_dir(&self, dir: &Path) -> io::Result<()> {
        let should_create = {
            let mut inner = self
                .state
                .lock()
                .map_err(|_| io::Error::other("media relay lock poisoned"))?;
            inner.recording_dirs.insert(dir.to_path_buf())
        };
        if !should_create {
            return Ok(());
        }

        if let Err(error) = fs::create_dir_all(dir) {
            let mut inner = self
                .state
                .lock()
                .map_err(|_| io::Error::other("media relay lock poisoned"))?;
            inner.recording_dirs.remove(dir);
            return Err(error);
        }

        Ok(())
    }

    fn enforce_recording_storage_policy(
        &self,
        dir: &Path,
        retention_secs: u64,
        min_free_bytes: u64,
    ) -> io::Result<()> {
        let protected_paths = self.active_recording_paths();
        cleanup_expired_recordings(dir, retention_secs, &protected_paths)?;

        if min_free_bytes == 0 {
            return Ok(());
        }

        let available = available_disk_bytes(dir)?;
        if available < min_free_bytes {
            return Err(io::Error::other(format!(
                "recording disk free space {available} bytes is below configured minimum {min_free_bytes} bytes"
            )));
        }
        Ok(())
    }

    fn active_recording_paths(&self) -> HashSet<PathBuf> {
        self.recordings
            .iter()
            .flat_map(|entry| {
                let info = &entry.value().session.info;
                [info.wav_path.clone()]
            })
            .collect()
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

    fn record_rtp_packet(
        &self,
        relay_port: u16,
        packet: RtpPacketView<'_>,
    ) -> Result<bool, MediaError> {
        let recording = self.recordings.get(&relay_port).map(|v| v.clone());
        let Some(recording) = recording else {
            return Ok(false);
        };

        recording
            .session
            .try_record(recording.channel, packet)
            .map_err(recording_error)
    }

    #[cfg(test)]
    fn flush_recording_for_test(&self, relay_port: u16) -> Result<(), MediaError> {
        let recording = self.recordings.get(&relay_port).map(|v| v.clone());
        let Some(recording) = recording else {
            return Ok(());
        };

        recording.session.flush().map_err(recording_error)
    }

    fn process_dtmf_packet(&self, local_port: u16, packet: RtpPacketView<'_>) {
        let (call_id, last_timestamp) = {
            let Some(state) = self.dtmf_states.get(&local_port) else {
                return;
            };
            if packet.payload_type != state.payload_type {
                return;
            }
            (state.call_id.clone(), state.last_timestamp)
        };

        let Ok(event) = TelephoneEvent::parse(packet.payload) else {
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
    session: Arc<RecordingSession>,
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
}

#[derive(Debug)]
struct RecordingSession {
    info: Arc<RecordingSessionInfo>,
    pool: Arc<RecordingPool>,
}

impl RecordingSession {
    #[allow(clippy::too_many_arguments)]
    fn new(
        wav_path: PathBuf,
        min_free_bytes: u64,
        max_file_bytes: u64,
        max_duration_secs: u64,
        pool: Arc<RecordingPool>,
    ) -> Self {
        Self {
            info: Arc::new(RecordingSessionInfo {
                id: NEXT_RECORDING_SESSION_ID.fetch_add(1, Ordering::Relaxed),
                wav_path,
                min_free_bytes,
                max_file_bytes,
                max_duration_secs,
                last_disk_check_ms: AtomicU64::new(0),
            }),
            pool,
        }
    }

    fn try_record(&self, channel: RecordingChannel, packet: RtpPacketView<'_>) -> io::Result<bool> {
        if AudioCodec::from_static_payload_type(packet.payload_type).is_none()
            || packet.payload.is_empty()
        {
            return Ok(false);
        }

        self.pool.try_record(
            Arc::clone(&self.info),
            RecordedRtpPacket {
                channel,
                payload_type: packet.payload_type,
                timestamp: packet.timestamp,
                payload: packet.payload.to_vec(),
            },
        )
    }

    #[cfg(test)]
    fn flush(&self) -> io::Result<()> {
        self.pool.flush(Arc::clone(&self.info))
    }
}

impl Drop for RecordingSession {
    fn drop(&mut self) {
        self.pool.finish(self.info.id);
    }
}

#[derive(Debug)]
struct RecordingSessionInfo {
    id: u64,
    wav_path: PathBuf,
    min_free_bytes: u64,
    max_file_bytes: u64,
    max_duration_secs: u64,
    last_disk_check_ms: AtomicU64,
}

impl RecordingSessionInfo {
    fn max_file_frames(&self) -> Option<u64> {
        let duration_frames = (self.max_duration_secs > 0).then(|| {
            self.max_duration_secs
                .saturating_mul(u64::from(RECORDING_SAMPLE_RATE))
        });
        let size_frames = (self.max_file_bytes > 44)
            .then(|| (self.max_file_bytes - 44) / u64::from(RECORDING_CHANNELS * 2));
        match (duration_frames, size_frames) {
            (Some(duration), Some(size)) => Some(duration.min(size)),
            (Some(duration), None) => Some(duration),
            (None, Some(size)) => Some(size),
            (None, None) => None,
        }
    }

    fn ensure_disk_space(&self) -> io::Result<()> {
        if self.min_free_bytes == 0 {
            return Ok(());
        }

        let now = unix_timestamp_millis() as u64;
        let last_check = self.last_disk_check_ms.load(Ordering::Relaxed);
        if now.saturating_sub(last_check) < 1_000 {
            return Ok(());
        }
        if self
            .last_disk_check_ms
            .compare_exchange(last_check, now, Ordering::Relaxed, Ordering::Relaxed)
            .is_err()
        {
            return Ok(());
        }

        let directory = self.wav_path.parent().unwrap_or_else(|| Path::new("."));
        let available = available_disk_bytes(directory)?;
        if available < self.min_free_bytes {
            return Err(io::Error::other(format!(
                "recording disk free space {available} bytes is below configured minimum {} bytes",
                self.min_free_bytes
            )));
        }
        Ok(())
    }
}

#[derive(Debug)]
struct RecordingPool {
    workers: Vec<RecordingWorkerHandle>,
    queue_capacity: usize,
}

#[derive(Debug)]
struct RecordingWorkerHandle {
    sender: mpsc::SyncSender<RecordingCommand>,
    pending_commands: Arc<AtomicUsize>,
}

impl RecordingPool {
    fn new(worker_count: usize, queue_capacity: usize) -> Self {
        let worker_count = worker_count.max(1);
        let queue_capacity = queue_capacity.max(1);
        let mut workers = Vec::with_capacity(worker_count);
        for worker_index in 0..worker_count {
            let (sender, receiver) = mpsc::sync_channel(queue_capacity);
            let pending_commands = Arc::new(AtomicUsize::new(0));
            match spawn_recording_worker(worker_index, receiver, Arc::clone(&pending_commands)) {
                Ok(()) => workers.push(RecordingWorkerHandle {
                    sender,
                    pending_commands,
                }),
                Err(error) => warn!(%error, worker_index, "failed to spawn recording worker"),
            }
        }
        Self {
            workers,
            queue_capacity,
        }
    }

    fn worker_count(&self) -> usize {
        self.workers.len()
    }

    fn queued_commands(&self) -> usize {
        self.workers
            .iter()
            .map(|worker| worker.pending_commands.load(Ordering::Relaxed))
            .sum()
    }

    fn total_capacity(&self) -> usize {
        self.workers.len() * self.queue_capacity
    }

    fn try_record(
        &self,
        session: Arc<RecordingSessionInfo>,
        packet: RecordedRtpPacket,
    ) -> io::Result<bool> {
        self.try_send(session.id, RecordingCommand::Packet { session, packet })?;
        Ok(true)
    }

    #[cfg(test)]
    fn flush(&self, session: Arc<RecordingSessionInfo>) -> io::Result<()> {
        let (sender, receiver) = mpsc::channel();
        self.send(
            session.id,
            RecordingCommand::Flush {
                session,
                reply: sender,
            },
        )?;
        receiver
            .recv()
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "recording worker is stopped"))?
    }

    fn finish(&self, session_id: u64) {
        let _ = self.try_send(session_id, RecordingCommand::Finish { session_id });
    }

    fn try_send(&self, session_id: u64, command: RecordingCommand) -> io::Result<()> {
        let worker = self.worker(session_id)?;
        match worker.sender.try_send(command) {
            Ok(()) => {
                worker.pending_commands.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }
            Err(mpsc::TrySendError::Full(_)) => Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "recording queue is full",
            )),
            Err(mpsc::TrySendError::Disconnected(_)) => Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "recording worker is stopped",
            )),
        }
    }

    #[cfg(test)]
    fn send(&self, session_id: u64, command: RecordingCommand) -> io::Result<()> {
        let worker = self.worker(session_id)?;
        worker.sender.send(command).map_err(|_| {
            io::Error::new(io::ErrorKind::BrokenPipe, "recording worker is stopped")
        })?;
        worker.pending_commands.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn worker(&self, session_id: u64) -> io::Result<&RecordingWorkerHandle> {
        if self.workers.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "recording worker pool is unavailable",
            ));
        }
        let index = session_id as usize % self.workers.len();
        Ok(&self.workers[index])
    }
}

fn spawn_recording_worker(
    worker_index: usize,
    receiver: mpsc::Receiver<RecordingCommand>,
    pending_commands: Arc<AtomicUsize>,
) -> io::Result<()> {
    std::thread::Builder::new()
        .name(format!("vos-rs-recording-{worker_index}"))
        .spawn(move || run_recording_worker(worker_index, receiver, pending_commands))
        .map(|_| ())
        .map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("failed to spawn recording worker: {error}"),
            )
        })
}

fn run_recording_worker(
    worker_index: usize,
    receiver: mpsc::Receiver<RecordingCommand>,
    pending_commands: Arc<AtomicUsize>,
) {
    let mut recorders = HashMap::<u64, RecordingFile>::new();
    while let Ok(command) = receiver.recv() {
        pending_commands.fetch_sub(1, Ordering::Relaxed);
        handle_recording_command(command, &mut recorders);

        for _ in 0..RECORDING_WORKER_DRAIN_LIMIT {
            let Ok(command) = receiver.try_recv() else {
                break;
            };
            pending_commands.fetch_sub(1, Ordering::Relaxed);
            handle_recording_command(command, &mut recorders);
        }
    }

    for (session_id, mut recording_file) in recorders {
        if let Err(error) = recording_file.recorder.flush_recording() {
            warn!(%error, session_id, worker_index, "failed to finalize call recording");
        }
    }
}

fn handle_recording_command(
    command: RecordingCommand,
    recorders: &mut HashMap<u64, RecordingFile>,
) {
    match command {
        RecordingCommand::Packet { session, packet } => {
            if let Err(error) = session.ensure_disk_space() {
                warn!(%error, session_id = session.id, "recording disk protection stopped packet write");
                return;
            }
            let should_rotate = recorders
                .get(&session.id)
                .map(|recording_file| {
                    recording_file.recorder.would_exceed_limit(
                        packet.channel,
                        packet.timestamp,
                        packet.payload.len(),
                        session.max_file_frames(),
                    )
                })
                .unwrap_or(false);
            if should_rotate {
                if let Err(error) = rotate_recording(recorders, &session) {
                    warn!(%error, session_id = session.id, "failed to rotate call recording");
                    return;
                }
            }
            let recording_file = match recorder_for_session(recorders, &session) {
                Ok(recording_file) => recording_file,
                Err(error) => {
                    warn!(%error, session_id = session.id, "failed to open call recording");
                    return;
                }
            };
            if let Err(error) = recording_file.recorder.record(
                packet.channel,
                packet.payload_type,
                packet.timestamp,
                &packet.payload,
            ) {
                warn!(%error, session_id = session.id, "failed to write RTP packet to recording");
            }
        }
        #[cfg(test)]
        RecordingCommand::Flush { session, reply } => {
            let result = recorder_for_session(recorders, &session)
                .and_then(|recording_file| recording_file.recorder.flush_recording());
            let _ = reply.send(result);
        }
        RecordingCommand::Finish { session_id } => {
            if let Some(mut recording_file) = recorders.remove(&session_id) {
                if let Err(error) = recording_file.recorder.flush_recording() {
                    warn!(%error, session_id, "failed to finalize call recording");
                }
            }
        }
    }
}

fn recorder_for_session<'a>(
    recorders: &'a mut HashMap<u64, RecordingFile>,
    session: &RecordingSessionInfo,
) -> io::Result<&'a mut RecordingFile> {
    if let std::collections::hash_map::Entry::Vacant(entry) = recorders.entry(session.id) {
        entry.insert(RecordingFile::create(session, 0)?);
    }
    recorders
        .get_mut(&session.id)
        .ok_or_else(|| io::Error::other("recording session was not initialized"))
}

fn rotate_recording(
    recorders: &mut HashMap<u64, RecordingFile>,
    session: &RecordingSessionInfo,
) -> io::Result<()> {
    let segment_index = recorders
        .get(&session.id)
        .map(|recording_file| recording_file.segment_index + 1)
        .unwrap_or(1);
    if let Some(mut previous) = recorders.remove(&session.id) {
        previous.recorder.flush_recording()?;
    }
    recorders.insert(session.id, RecordingFile::create(session, segment_index)?);
    Ok(())
}

struct RecordingFile {
    segment_index: u32,
    recorder: WavCallRecorder,
}

impl RecordingFile {
    fn create(session: &RecordingSessionInfo, segment_index: u32) -> io::Result<Self> {
        let wav_path = recording_segment_path(session, segment_index);
        let recorder = WavCallRecorder::create(wav_path)?;
        Ok(Self {
            segment_index,
            recorder,
        })
    }
}

struct RecordedRtpPacket {
    channel: RecordingChannel,
    payload_type: u8,
    timestamp: u32,
    payload: Vec<u8>,
}

enum RecordingCommand {
    Packet {
        session: Arc<RecordingSessionInfo>,
        packet: RecordedRtpPacket,
    },
    #[cfg(test)]
    Flush {
        session: Arc<RecordingSessionInfo>,
        reply: mpsc::Sender<io::Result<()>>,
    },
    Finish {
        session_id: u64,
    },
}

#[derive(Debug)]
struct WavCallRecorder {
    file: File,
    frames_written: u64,
    flushed_frames: u64,
    base_timestamps: [Option<u32>; 2],
    frames_since_flush: u64,
    interleaved_samples: Vec<i16>,
    write_buffer: Vec<u8>,
}

const RECORDING_FLUSH_INTERVAL_FRAMES: u64 = RECORDING_SAMPLE_RATE as u64 * 2;

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
            flushed_frames: 0,
            base_timestamps: [None, None],
            frames_since_flush: 0,
            interleaved_samples: Vec::new(),
            write_buffer: Vec::new(),
        })
    }

    fn record(
        &mut self,
        channel: RecordingChannel,
        payload_type: u8,
        timestamp: u32,
        payload: &[u8],
    ) -> io::Result<bool> {
        let codec = match AudioCodec::from_static_payload_type(payload_type) {
            Some(c) => c,
            None => return Ok(false),
        };
        if payload.is_empty() {
            return Ok(false);
        }

        let num_samples = payload.len();
        let start_frame = self.start_frame(channel, timestamp);
        self.ensure_frames(start_frame + num_samples as u64)?;
        if start_frame < self.flushed_frames {
            return Ok(true);
        }

        for (sample_index, &payload_byte) in payload.iter().enumerate() {
            let sample = match codec {
                AudioCodec::Pcmu => decode_pcmu(payload_byte),
                AudioCodec::Pcma => decode_pcma(payload_byte),
            };
            let frame = start_frame + sample_index as u64;
            self.set_sample(frame, channel, sample);
        }

        self.frames_since_flush += num_samples as u64;
        if self.frames_since_flush >= RECORDING_FLUSH_INTERVAL_FRAMES {
            self.flush_ready_frames(false)?;
            self.frames_since_flush = 0;
        }
        Ok(true)
    }

    fn would_exceed_limit(
        &self,
        channel: RecordingChannel,
        timestamp: u32,
        payload_len: usize,
        max_frames: Option<u64>,
    ) -> bool {
        let Some(max_frames) = max_frames else {
            return false;
        };
        let base = self.base_timestamps[channel.index()].unwrap_or(timestamp);
        let start_frame = u64::from(timestamp.wrapping_sub(base));
        self.frames_written > 0 && start_frame.saturating_add(payload_len as u64) > max_frames
    }

    fn start_frame(&mut self, channel: RecordingChannel, timestamp: u32) -> u64 {
        let base = self.base_timestamps[channel.index()].get_or_insert(timestamp);
        u64::from(timestamp.wrapping_sub(*base))
    }

    fn ensure_frames(&mut self, target_frames: u64) -> io::Result<()> {
        if self.frames_written >= target_frames || target_frames <= self.flushed_frames {
            return Ok(());
        }

        let buffered_frames = target_frames - self.flushed_frames;
        let samples = buffered_frames as usize * usize::from(RECORDING_CHANNELS);
        self.interleaved_samples.resize(samples, 0);
        self.frames_written = target_frames;
        Ok(())
    }

    fn set_sample(&mut self, frame: u64, channel: RecordingChannel, sample: i16) {
        let relative_frame = frame - self.flushed_frames;
        let offset = relative_frame as usize * usize::from(RECORDING_CHANNELS) + channel.index();
        if let Some(slot) = self.interleaved_samples.get_mut(offset) {
            *slot = sample;
        }
    }

    fn flush_ready_frames(&mut self, final_flush: bool) -> io::Result<()> {
        let buffered_frames = self.frames_written.saturating_sub(self.flushed_frames);
        if buffered_frames == 0 {
            if final_flush {
                self.refresh_header()?;
                self.flush()?;
            }
            return Ok(());
        }

        let frames_to_write = if final_flush {
            buffered_frames
        } else {
            buffered_frames.saturating_sub(RECORDING_FLUSH_INTERVAL_FRAMES)
        };
        if frames_to_write == 0 {
            return Ok(());
        }

        let sample_count = frames_to_write as usize * usize::from(RECORDING_CHANNELS);
        self.write_buffer.clear();
        self.write_buffer.reserve(sample_count * 2);
        for sample in self.interleaved_samples.iter().take(sample_count) {
            self.write_buffer.extend_from_slice(&sample.to_le_bytes());
        }

        self.file.seek(SeekFrom::End(0))?;
        self.file.write_all(&self.write_buffer)?;
        self.interleaved_samples.drain(..sample_count);
        self.flushed_frames += frames_to_write;
        self.refresh_header()?;
        self.flush()
    }

    fn refresh_header(&mut self) -> io::Result<()> {
        let data_bytes = u32::try_from(self.flushed_frames * u64::from(RECORDING_CHANNELS) * 2)
            .map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "WAV recording is too large")
            })?;
        self.file.seek(SeekFrom::Start(0))?;
        write_wav_header(&mut self.file, data_bytes)?;
        self.file.seek(SeekFrom::End(0))?;
        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }

    fn flush_recording(&mut self) -> io::Result<()> {
        self.flush_ready_frames(true)
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

fn recording_segment_path(session: &RecordingSessionInfo, segment_index: u32) -> PathBuf {
    if segment_index == 0 {
        return session.wav_path.clone();
    }

    let directory = session.wav_path.parent().unwrap_or_else(|| Path::new("."));
    let stem = session
        .wav_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("recording");
    let segment_stem = format!("{stem}-part-{segment_index:04}");
    directory.join(format!("{segment_stem}.wav"))
}

fn unix_timestamp_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis()
}

fn cleanup_expired_recordings(
    dir: &Path,
    retention_secs: u64,
    protected_paths: &HashSet<PathBuf>,
) -> io::Result<()> {
    if retention_secs == 0 {
        return Ok(());
    }

    let retention = Duration::from_secs(retention_secs);
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if protected_paths.contains(&path) || !entry.file_type()?.is_file() {
            continue;
        }

        let is_recording_artifact = path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(|extension| extension.eq_ignore_ascii_case("wav"))
            .unwrap_or(false);
        if !is_recording_artifact {
            continue;
        }

        let is_expired = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .and_then(|modified| modified.elapsed().map_err(io::Error::other))
            .map(|age| age >= retention)
            .unwrap_or(false);
        if is_expired {
            fs::remove_file(path)?;
        }
    }
    Ok(())
}

fn available_disk_bytes(path: &Path) -> io::Result<u64> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;

        let path = CString::new(path.as_os_str().as_bytes()).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidInput, "recording path contains NUL")
        })?;
        let mut statistics = MaybeUninit::<libc::statvfs>::uninit();
        let result = unsafe { libc::statvfs(path.as_ptr(), statistics.as_mut_ptr()) };
        if result != 0 {
            return Err(io::Error::last_os_error());
        }

        let statistics = unsafe { statistics.assume_init() };
        let block_size = u128::from(if statistics.f_frsize == 0 {
            statistics.f_bsize
        } else {
            statistics.f_frsize
        });
        let available = block_size * u128::from(statistics.f_bavail);
        Ok(available.min(u128::from(u64::MAX)) as u64)
    }

    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(u64::MAX)
    }
}

fn recording_error(error: io::Error) -> MediaError {
    if error.kind() == io::ErrorKind::WouldBlock {
        MediaError::RecordingQueueFull
    } else {
        MediaError::Recording(error.to_string())
    }
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
    RecordingQueueFull,
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
            Self::RecordingQueueFull => write!(f, "recording queue is full"),
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

fn env_usize(name: &str) -> Option<usize> {
    env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
}

fn env_u64(name: &str) -> Option<u64> {
    env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
}

fn recording_worker_count() -> usize {
    env_usize(RECORDING_WORKERS_ENV)
        .filter(|workers| *workers > 0)
        .unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(|parallelism| parallelism.get().clamp(1, DEFAULT_RECORDING_WORKERS))
                .unwrap_or(DEFAULT_RECORDING_WORKERS)
        })
}

fn recording_queue_capacity() -> usize {
    env_usize(RECORDING_QUEUE_CAPACITY_ENV)
        .filter(|capacity| *capacity > 0)
        .unwrap_or(DEFAULT_RECORDING_QUEUE_CAPACITY)
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
        MediaConfig, MediaCryptoSession, MediaError, MediaRelayMetrics, MediaRelayState,
        RtcpQualitySnapshot, RtcpQualityWindow, RtpPacketView,
    };
    use rtp_core::{SrtpConfig, SrtpContext};
    use sdp_core::RtpEndpoint;
    use sip_core::{HeaderMap, HeaderName, HeaderValue};
    use std::{fs, net::SocketAddr, path::PathBuf};
    use tokio::net::UdpSocket;
    use tokio::time::{sleep, timeout, Duration};

    #[test]
    fn recording_pool_reports_capacity_and_queue_depth() {
        let pool = super::RecordingPool::new(2, 3);

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
        let available = super::available_disk_bytes(&dir).unwrap();
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
