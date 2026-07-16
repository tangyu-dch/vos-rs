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
pub use crate::media::config::{MediaConfig, DEFAULT_RTP_PORT_MIN};
pub use crate::media::crypto::MediaCryptoSession;
pub use crate::media::dtmf::DtmfState;
pub use crate::media::metrics::{MediaRelayMetrics, RtcpQualitySnapshot, RtpReceiveStats};
pub use crate::media::recording::{MediaError, RecordingLeg, RecordingPool};
use crate::media::sdp::socket_addr_for_endpoint;
pub use crate::media::utils::unix_timestamp_millis;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

pub mod affinity;
#[allow(dead_code)]
pub mod ai_plugin;
mod allocation;
#[allow(dead_code)]
pub mod ebpf;
mod listener;
mod path;
#[allow(dead_code)]
mod playback;
pub mod pool {
    pub use rtp_core::{PacketBufferPool, ReusablePacket};
}
#[allow(dead_code)]
pub mod sans_io;
mod source;
mod state;
pub mod webrtc;

pub(crate) use listener::relay_media_port;
#[allow(unused_imports)]
pub use listener::spawn_rtp_relay_listeners;
use path::{FastPathCounters, RelayPath};

pub const MAX_RTP_DATAGRAM_SIZE: usize = 65_535;
const MEDIA_PACKET_POOL_CAPACITY: usize = 4_096;

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PlaybackMode {
    Exclusive,  // 独占替换模式：只播放音频，拦截来自另一侧的原始声音
    Background, // 背景混音模式：将本地音频与来自另一侧的原始声音混合
}

#[derive(Debug, Clone)]
pub(crate) struct PlaybackState {
    pub(crate) mode: PlaybackMode,
    pub(crate) loop_playback: bool,
    pub(crate) samples: Vec<i16>, // 解码后的 PCM 采样数据
    pub(crate) current_sample_idx: usize,
    pub(crate) ssrc: u32,
    pub(crate) sequence_number: u16,
    pub(crate) timestamp: u32,
}

#[derive(Debug, Clone)]
pub(crate) struct PendingSrtpConfig {
    pub(crate) suite: String,
    pub(crate) key_params: String,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SourceBinding {
    pub(crate) address: SocketAddr,
}

#[derive(Debug, Clone, Copy, Default)]
struct RtpContinuityState {
    last_sequence: Option<u16>,
    last_timestamp: Option<u32>,
    sequence_offset: u16,
    timestamp_offset: u32,
    resume_after_exclusive: bool,
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
    pub(crate) source_bindings: Arc<DashMap<u16, SourceBinding>>,
    pub(crate) leased_rtp_ports: Arc<rtp_core::PortLeaseMap>,
    pub(crate) next_port: Arc<AtomicU32>,
    pub(crate) path_epochs: Arc<DashMap<u16, Arc<AtomicU64>>>,
    pub(crate) state: Arc<Mutex<MediaRelayStateInner>>,
    pub(crate) active_sockets: Arc<DashMap<u16, Arc<UdpSocket>>>,
    pub(crate) playbacks: Arc<DashMap<u16, Arc<std::sync::Mutex<PlaybackState>>>>,
    pub(crate) playback_loops: Arc<DashMap<u16, tokio::sync::oneshot::Sender<()>>>,
    pub(crate) muted_ports: Arc<dashmap::DashSet<u16>>,
    continuity: Arc<DashMap<u16, RtpContinuityState>>,
    pub(crate) conference_manager: Arc<crate::media::conference::ConferenceManager>,
    pub(crate) monitors: Arc<DashMap<u16, Vec<SocketAddr>>>,
    pub(crate) webrtc_sessions: Arc<DashMap<u16, webrtc::WebRtcSession>>,
    pub(crate) buffer_pool: Arc<pool::PacketBufferPool>,
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
            source_bindings: Arc::clone(&self.source_bindings),
            leased_rtp_ports: Arc::clone(&self.leased_rtp_ports),
            next_port: Arc::clone(&self.next_port),
            path_epochs: Arc::clone(&self.path_epochs),
            state: Arc::clone(&self.state),
            active_sockets: Arc::clone(&self.active_sockets),
            playbacks: Arc::clone(&self.playbacks),
            playback_loops: Arc::clone(&self.playback_loops),
            muted_ports: Arc::clone(&self.muted_ports),
            continuity: Arc::clone(&self.continuity),
            conference_manager: Arc::clone(&self.conference_manager),
            monitors: Arc::clone(&self.monitors),
            webrtc_sessions: Arc::clone(&self.webrtc_sessions),
            buffer_pool: Arc::clone(&self.buffer_pool),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SymmetricSourceUpdate {
    source_port: u16,
    target_port: u16,
    previous_target: Option<SocketAddr>,
    learned_target: SocketAddr,
}

#[cfg(test)]
mod tests;
