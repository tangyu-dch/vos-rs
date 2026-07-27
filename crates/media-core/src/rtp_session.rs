use dashmap::DashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use crate::dtmf::DtmfTracker;
use crate::metrics::MediaRelayMetrics;
use rtp_core::AudioCodec;

/// 关联单个 RTP 端口的完整会话状态。
/// 将目标地址、编解码器、指标、DTMF 状态统一在单个结构中，
/// 彻底消除 RTP 单包处理过程中多次查询 DashMap 带来的锁竞争与 Cache Line 抖动。
#[derive(Debug)]
pub struct RtpPortSession {
    pub port: u16,
    pub peer_port: Option<u16>,
    target: RwLock<Option<SocketAddr>>,
    pub codec: AudioCodec,
    pub metrics: MediaRelayMetrics,
    dtmf: Mutex<Option<DtmfTracker>>,
    pub packets_forwarded: AtomicU64,
    pub bytes_forwarded: AtomicU64,
}

impl RtpPortSession {
    pub fn new(port: u16, peer_port: Option<u16>, codec: AudioCodec) -> Self {
        Self {
            port,
            peer_port,
            target: RwLock::new(None),
            codec,
            metrics: MediaRelayMetrics::default(),
            dtmf: Mutex::new(None),
            packets_forwarded: AtomicU64::new(0),
            bytes_forwarded: AtomicU64::new(0),
        }
    }

    pub fn set_target(&self, addr: SocketAddr) {
        if let Ok(mut guard) = self.target.write() {
            *guard = Some(addr);
        }
    }

    pub fn get_target(&self) -> Option<SocketAddr> {
        self.target.read().ok().and_then(|guard| *guard)
    }

    pub fn set_dtmf_tracker(&self, tracker: DtmfTracker) {
        if let Ok(mut guard) = self.dtmf.lock() {
            *guard = Some(tracker);
        }
    }

    pub fn record_forwarded_packet(&self, bytes: usize) {
        self.packets_forwarded.fetch_add(1, Ordering::Relaxed);
        self.bytes_forwarded
            .fetch_add(bytes as u64, Ordering::Relaxed);
    }
}

/// 基于单个 DashMap 索引的高并发 RTP 端口会话表。
#[derive(Debug, Default, Clone)]
pub struct RtpPortSessionTable {
    sessions: Arc<DashMap<u16, Arc<RtpPortSession>>>,
}

impl RtpPortSessionTable {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(DashMap::new()),
        }
    }

    pub fn insert(&self, port: u16, session: RtpPortSession) -> Arc<RtpPortSession> {
        let arc = Arc::new(session);
        self.sessions.insert(port, arc.clone());
        arc
    }

    pub fn get(&self, port: u16) -> Option<Arc<RtpPortSession>> {
        self.sessions.get(&port).map(|r| r.value().clone())
    }

    pub fn remove(&self, port: u16) -> Option<Arc<RtpPortSession>> {
        self.sessions.remove(&port).map(|(_, s)| s)
    }

    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }
}
