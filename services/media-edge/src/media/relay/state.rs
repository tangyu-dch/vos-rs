use super::*;

impl Default for MediaRelayState {
    fn default() -> Self {
        Self::new()
    }
}

impl MediaRelayState {
    pub fn new() -> Self {
        Self::with_recording_pool(4, 10_000)
    }

    pub fn with_recording_pool(recording_workers: usize, recording_queue_capacity: usize) -> Self {
        let conference_manager = Arc::new(crate::media::conference::ConferenceManager::new());
        crate::media::conference::start_mixer_loop(Arc::clone(&conference_manager));

        Self {
            targets: Arc::new(DashMap::new()),
            peer_ports: Arc::new(DashMap::new()),
            codecs: Arc::new(DashMap::new()),
            metrics: Arc::new(DashMap::new()),
            recordings: Arc::new(DashMap::new()),
            recording_pool: Arc::new(crate::media::recording::new_recording_pool(
                recording_workers,
                recording_queue_capacity,
            )),
            dtmf_states: Arc::new(DashMap::new()),
            active_loops: Arc::new(DashMap::new()),
            crypto_sessions: Arc::new(DashMap::new()),
            pending_srtp: Arc::new(DashMap::new()),
            source_bindings: Arc::new(DashMap::new()),
            leased_rtp_ports: Arc::new(rtp_core::PortLeaseMap::new(0, 65535)),
            next_port: Arc::new(AtomicU32::new(DEFAULT_RTP_PORT_MIN as u32)),
            path_epochs: Arc::new(DashMap::new()),
            state: Arc::new(Mutex::new(MediaRelayStateInner {
                recording_dirs: HashSet::new(),
                dtmf_accumulators: HashMap::new(),
                dtmf_event_log: HashMap::new(),
            })),
            active_sockets: Arc::new(DashMap::new()),
            playbacks: Arc::new(DashMap::new()),
            playback_loops: Arc::new(DashMap::new()),
            muted_ports: Arc::new(dashmap::DashSet::new()),
            continuity: Arc::new(DashMap::new()),
            port_sessions: media_core::RtpPortSessionTable::new(),
            conference_manager,
            monitors: Arc::new(DashMap::new()),
            webrtc_sessions: Arc::new(DashMap::new()),
            buffer_pool: Arc::new(pool::PacketBufferPool::new(MEDIA_PACKET_POOL_CAPACITY)),
        }
    }

    /// 在已分配的 RTP 端口上启用 ICE-Lite、DTLS 与 SRTP。
    pub fn register_webrtc_session(
        &self,
        port: u16,
    ) -> Result<webrtc::WebRtcSessionDescription, MediaError> {
        let socket = self
            .active_sockets
            .get(&port)
            .map(|entry| Arc::clone(entry.value()))
            .ok_or_else(|| MediaError::Io(format!("RTP 端口 {port} 尚未分配")))?;
        let (session, description) = webrtc::WebRtcSession::start(port, socket)
            .map_err(|error| MediaError::Io(format!("创建 WebRTC 会话失败: {error}")))?;
        self.webrtc_sessions.insert(port, session);
        self.mark_port_and_peer_features_changed(port);
        Ok(description)
    }

    /// 释放指定端口的 WebRTC 加密会话。
    pub fn unregister_webrtc_session(&self, port: u16) {
        self.webrtc_sessions.remove(&port);
        self.mark_port_and_peer_features_changed(port);
    }

    /// 为指定端口注册一个监控地址（旁听者）。
    ///
    /// 注册后，该端口收发的 RTP 包会被复制一份发送到 `supervisor` 地址，
    /// 用于通话质检、实时监听与录音旁路。可通过 UDS `start_monitoring` 端点调用。
    pub fn start_monitoring(&self, port: u16, supervisor: SocketAddr) {
        self.monitors.entry(port).or_default().push(supervisor);
        self.mark_relay_features_changed(port);
        tracing::info!(port, %supervisor, "started monitoring port");
    }

    /// 移除指定端口上的某个监控地址。
    ///
    /// 当最后一个监控者被移除时，自动清理该端口的 monitors 列表。
    /// 可通过 UDS `stop_monitoring` 端点调用。
    pub fn stop_monitoring(&self, port: u16, supervisor: SocketAddr) {
        let should_remove = if let Some(mut entry) = self.monitors.get_mut(&port) {
            entry.retain(|&x| x != supervisor);
            tracing::info!(port, %supervisor, "stopped monitoring port");
            entry.is_empty()
        } else {
            false
        };
        if should_remove {
            self.monitors.remove(&port);
        }
        self.mark_relay_features_changed(port);
    }

    /// 清除指定端口上的所有监控地址。
    ///
    /// 由 main.rs 的 `clear_monitors` UDS/HTTP 端点调用。
    pub fn clear_monitors(&self, port: u16) {
        if self.monitors.remove(&port).is_some() {
            self.mark_relay_features_changed(port);
            tracing::info!(port, "cleared monitors for port");
        }
    }

    pub(super) fn mark_resume_after_exclusive(&self, port: u16) {
        self.continuity
            .entry(port)
            .or_default()
            .resume_after_exclusive = true;
        self.mark_relay_features_changed(port);
    }

    pub(super) fn continuity_offsets(
        &self,
        port: u16,
        sequence: u16,
        timestamp: u32,
    ) -> (u16, u32) {
        let mut continuity = self.continuity.entry(port).or_default();
        if continuity.resume_after_exclusive {
            if let (Some(last_sequence), Some(last_timestamp)) =
                (continuity.last_sequence, continuity.last_timestamp)
            {
                continuity.sequence_offset = sequence.wrapping_sub(last_sequence.wrapping_add(1));
                continuity.timestamp_offset =
                    timestamp.wrapping_sub(last_timestamp.wrapping_add(160));
            }
            continuity.resume_after_exclusive = false;
        }

        let offsets = (continuity.sequence_offset, continuity.timestamp_offset);
        continuity.last_sequence = Some(sequence.wrapping_sub(offsets.0));
        continuity.last_timestamp = Some(timestamp.wrapping_sub(offsets.1));
        offsets
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
        if let Some(session) = self.port_sessions.get(relay_port) {
            session.set_target(target);
        }

        let binding_opt = self.source_bindings.get(&relay_port).map(|entry| *entry);
        if let Some(binding) = binding_opt {
            let target_port_opt = self.peer_ports.get(&relay_port).map(|entry| *entry);
            if let Some(target_port) = target_port_opt {
                self.targets.insert(target_port, binding.address);
                if let Some(session) = self.port_sessions.get(target_port) {
                    session.set_target(binding.address);
                }
                self.mark_relay_features_changed(target_port);

                if let (SocketAddr::V4(src_v4), SocketAddr::V4(dst_v4)) = (binding.address, target)
                {
                    let _ = crate::media::relay::ebpf::register_ebpf_relay(
                        *src_v4.ip(),
                        src_v4.port(),
                        *dst_v4.ip(),
                        dst_v4.port(),
                    );
                }
            }
        }
        self.mark_relay_features_changed(relay_port);
    }

    /// 为指定端口注册已协商完成的 SDES-SRTP 会话。
    ///
    /// 当信令层已完成 SDP `a=crypto:` 协商并提取到 SSRC 时，通过此方法立即安装
    /// SRTP 加解密上下文。可通过 UDS `register_srtp_session` 端点调用。
    pub fn register_srtp_session(
        &self,
        relay_port: u16,
        suite: &str,
        key_params: &str,
        ssrc: u32,
    ) -> Result<(), SrtpError> {
        let session = MediaCryptoSession::from_sdes(suite, key_params, ssrc)?;
        self.crypto_sessions
            .insert(relay_port, Arc::new(tokio::sync::Mutex::new(session)));
        self.mark_port_and_peer_features_changed(relay_port);
        Ok(())
    }

    /// 注册待定的 SDES-SRTP offer，等待首个 RTP 包携带的 SSRC 到达后自动激活。
    ///
    /// 当信令层仅拿到 `a=crypto:` 属性但尚未学到 SSRC 时调用。媒体转发循环在
    /// `listener::relay_media_port` 中收到首个 RTP 包时会读取此 offer 并通过
    /// `register_srtp_session` 完成激活。可通过 UDS `register_srtp_offer` 端点调用。
    pub fn register_srtp_offer(&self, relay_port: u16, suite: &str, key_params: &str) {
        self.pending_srtp.insert(
            relay_port,
            PendingSrtpConfig {
                suite: suite.to_string(),
                key_params: key_params.to_string(),
            },
        );
        self.mark_port_and_peer_features_changed(relay_port);
    }

    pub(crate) fn clear_srtp_session(&self, relay_port: u16) {
        self.crypto_sessions.remove(&relay_port);
        self.pending_srtp.remove(&relay_port);
        if let Some(peer_port) = self.peer_ports.get(&relay_port).map(|value| *value) {
            self.crypto_sessions.remove(&peer_port);
            self.pending_srtp.remove(&peer_port);
        }
        self.mark_port_and_peer_features_changed(relay_port);
    }

    /// 为指定端口注册音频编解码器，供媒体转发循环执行转码与录音解码。
    ///
    /// 由信令层在 SDP 协商完成后通过 UDS `register_port_codec` 端点调用，
    /// 或在测试中通过 `MediaRelayState::register_port_codec` 直接注入。
    pub fn register_port_codec(&self, port: u16, codec: rtp_core::AudioCodec) {
        self.codecs.insert(port, codec);
        self.mark_port_and_peer_features_changed(port);
    }

    pub fn clear_target(&self, relay_port: u16) {
        let rtp_port = rtp_port_for(relay_port).unwrap_or(relay_port);
        let peer_port = self.peer_ports.get(&rtp_port).map(|v| *v);

        if let Some(binding) = self.source_bindings.get(&rtp_port).map(|e| *e) {
            if let SocketAddr::V4(src_v4) = binding.address {
                let _ =
                    crate::media::relay::ebpf::unregister_ebpf_relay(*src_v4.ip(), src_v4.port());
            }
        }
        if let Some(p_port) = peer_port {
            if let Some(binding) = self.source_bindings.get(&p_port).map(|e| *e) {
                if let SocketAddr::V4(src_v4) = binding.address {
                    let _ = crate::media::relay::ebpf::unregister_ebpf_relay(
                        *src_v4.ip(),
                        src_v4.port(),
                    );
                }
            }
        }

        self.stop_playback(rtp_port);
        self.active_sockets.remove(&rtp_port);
        self.muted_ports.remove(&rtp_port);
        self.continuity.remove(&rtp_port);
        if let Some(p_port) = peer_port {
            self.stop_playback(p_port);
            self.active_sockets.remove(&p_port);
            self.muted_ports.remove(&p_port);
            self.continuity.remove(&p_port);
        }

        self.targets.remove(&rtp_port);
        self.metrics.remove(&rtp_port);
        self.source_bindings.remove(&rtp_port);
        self.peer_ports.remove(&rtp_port);
        self.codecs.remove(&rtp_port);
        self.recordings.remove(&rtp_port);
        self.clear_srtp_session(rtp_port);
        self.unregister_webrtc_session(rtp_port);
        self.dtmf_states.remove(&rtp_port);
        self.leased_rtp_ports.remove(rtp_port);
        self.port_sessions.remove(rtp_port);
        if let Some(peer_port) = peer_port {
            self.targets.remove(&peer_port);
            self.metrics.remove(&peer_port);
            self.source_bindings.remove(&peer_port);
            self.peer_ports.remove(&peer_port);
            self.codecs.remove(&peer_port);
            self.recordings.remove(&peer_port);
            self.unregister_webrtc_session(peer_port);
            self.dtmf_states.remove(&peer_port);
            self.port_sessions.remove(peer_port);
        }
        if let Some(rtcp_port) = rtcp_port_for(rtp_port) {
            self.targets.remove(&rtcp_port);
            self.metrics.remove(&rtcp_port);
            self.active_sockets.remove(&rtcp_port);
            let rtcp_peer = self.peer_ports.get(&rtcp_port).map(|v| *v);
            self.peer_ports.remove(&rtcp_port);
            if let Some(rtcp_peer_port) = rtcp_peer {
                self.active_sockets.remove(&rtcp_peer_port);
                self.peer_ports.remove(&rtcp_peer_port);
            }
        }
        if let Some((_, senders)) = self.active_loops.remove(&rtp_port) {
            for sender in senders {
                let _ = sender.send(());
            }
        }
        self.mark_relay_features_changed(rtp_port);
        if let Some(peer_port) = peer_port {
            self.mark_relay_features_changed(peer_port);
        }
    }
}
