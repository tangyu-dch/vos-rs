use super::*;

/// RTP 包在当前端口采用的处理路径。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RelayPath {
    /// 仅校验 RTP 固定头并直接转发，不进入录音、转码等处理链。
    Fast,
    /// 进入完整的媒体处理链。
    Processed,
}

/// 监听循环缓存的不可变转发计划。
#[derive(Debug, Clone, Copy)]
pub(super) struct RelayPlan {
    pub(super) path: RelayPath,
    pub(super) target: Option<SocketAddr>,
    pub(super) dtmf_payload_type: Option<u8>,
}

/// 快路径按批次回写指标，避免每个 RTP 包都获取 DashMap 分片锁。
#[derive(Debug, Default)]
pub(super) struct FastPathCounters {
    received_packets: u64,
    forwarded_packets: u64,
    send_errors: u64,
    fast_path_packets: u64,
}

impl FastPathCounters {
    const FLUSH_INTERVAL_PACKETS: u64 = 256;

    pub(super) fn record_received(&mut self) {
        self.received_packets += 1;
    }

    pub(super) fn record_forwarded(&mut self) {
        self.forwarded_packets += 1;
        self.fast_path_packets += 1;
    }

    pub(super) fn record_send_error(&mut self) {
        self.send_errors += 1;
        self.fast_path_packets += 1;
    }

    pub(super) fn flush_if_needed(&mut self, relay: &MediaRelayState, relay_port: u16) {
        if self.received_packets >= Self::FLUSH_INTERVAL_PACKETS {
            self.flush(relay, relay_port);
        }
    }

    pub(super) fn flush(&mut self, relay: &MediaRelayState, relay_port: u16) {
        if self.received_packets == 0 && self.forwarded_packets == 0 && self.send_errors == 0 {
            return;
        }
        relay.record_metric(relay_port, |metrics| {
            metrics.received_packets += self.received_packets;
            metrics.forwarded_packets += self.forwarded_packets;
            metrics.send_errors += self.send_errors;
            metrics.fast_path_packets += self.fast_path_packets;
        });
        *self = Self::default();
    }
}

impl MediaRelayState {
    /// 通知媒体监听循环重新计算转发计划。
    pub(crate) fn mark_relay_features_changed(&self, relay_port: u16) {
        self.relay_features_version(relay_port)
            .fetch_add(1, Ordering::Release);
    }

    pub(super) fn mark_port_and_peer_features_changed(&self, relay_port: u16) {
        self.mark_relay_features_changed(relay_port);
        if let Some(peer_port) = self.peer_ports.get(&relay_port).map(|entry| *entry) {
            self.mark_relay_features_changed(peer_port);
        }
    }

    pub(super) fn relay_features_version(&self, relay_port: u16) -> Arc<AtomicU64> {
        self.path_epochs
            .entry(relay_port)
            .or_insert_with(|| Arc::new(AtomicU64::new(1)))
            .clone()
    }

    pub(super) fn relay_plan(&self, relay_port: u16) -> RelayPlan {
        let target = self.targets.get(&relay_port).map(|entry| *entry);
        let peer_port = self.peer_ports.get(&relay_port).map(|entry| *entry);
        let dtmf_payload_type = self
            .dtmf_states
            .get(&relay_port)
            .map(|state| state.payload_type);

        let requires_processing = self.requires_processed_path(relay_port, peer_port);
        let has_valid_target = target
            .map(|address| !address.ip().is_unspecified() && address.port() != 0)
            .unwrap_or(false);

        RelayPlan {
            path: if has_valid_target && !requires_processing {
                RelayPath::Fast
            } else {
                RelayPath::Processed
            },
            target,
            dtmf_payload_type,
        }
    }

    fn requires_processed_path(&self, relay_port: u16, peer_port: Option<u16>) -> bool {
        if self.recordings.contains_key(&relay_port)
            || self.crypto_sessions.contains_key(&relay_port)
            || self.pending_srtp.contains_key(&relay_port)
            || self.muted_ports.contains(&relay_port)
            || self.monitors.contains_key(&relay_port)
            || self
                .conference_manager
                .port_to_conference
                .contains_key(&relay_port)
        {
            return true;
        }

        if self
            .continuity
            .get(&relay_port)
            .map(|state| {
                state.resume_after_exclusive
                    || state.sequence_offset != 0
                    || state.timestamp_offset != 0
            })
            .unwrap_or(false)
        {
            return true;
        }

        let Some(peer_port) = peer_port else {
            return true;
        };

        if self.playbacks.contains_key(&peer_port)
            || self.crypto_sessions.contains_key(&peer_port)
            || self.pending_srtp.contains_key(&peer_port)
        {
            return true;
        }

        matches!(
            (
                self.codecs.get(&relay_port).map(|codec| *codec),
                self.codecs.get(&peer_port).map(|codec| *codec),
            ),
            (Some(local_codec), Some(peer_codec)) if local_codec != peer_codec
        )
    }
}
