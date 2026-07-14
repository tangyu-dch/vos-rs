use super::*;

impl MediaRelayState {
    #[cfg(test)]
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
            recording_pool: Arc::new(RecordingPool::new(
                recording_workers,
                recording_queue_capacity,
            )),
            dtmf_states: Arc::new(DashMap::new()),
            active_loops: Arc::new(DashMap::new()),
            crypto_sessions: Arc::new(DashMap::new()),
            pending_srtp: Arc::new(DashMap::new()),
            source_bindings: Arc::new(DashMap::new()),
            leased_rtp_ports: Arc::new(dashmap::DashSet::new()),
            next_port: Arc::new(AtomicU32::new(DEFAULT_RTP_PORT_MIN as u32)),
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
            conference_manager,
            monitors: Arc::new(DashMap::new()),
        }
    }

    pub(crate) fn start_monitoring(&self, port: u16, supervisor: SocketAddr) {
        self.monitors.entry(port).or_default().push(supervisor);
        tracing::info!(port, %supervisor, "started monitoring port");
    }

    pub(crate) fn stop_monitoring(&self, port: u16, supervisor: SocketAddr) {
        if let Some(mut entry) = self.monitors.get_mut(&port) {
            entry.retain(|&x| x != supervisor);
            tracing::info!(port, %supervisor, "stopped monitoring port");
        }
    }

    pub(crate) fn clear_monitors(&self, port: u16) {
        if self.monitors.remove(&port).is_some() {
            tracing::info!(port, "cleared monitors for port");
        }
    }

    pub(super) fn mark_resume_after_exclusive(&self, port: u16) {
        self.continuity
            .entry(port)
            .or_default()
            .resume_after_exclusive = true;
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

    #[cfg(test)]
    pub(crate) fn seed_continuity_for_test(&self, port: u16, sequence: u16, timestamp: u32) {
        let mut continuity = self.continuity.entry(port).or_default();
        continuity.last_sequence = Some(sequence);
        continuity.last_timestamp = Some(timestamp);
    }

    #[cfg(test)]
    pub(crate) fn resume_continuity_for_test(&self, port: u16) {
        self.mark_resume_after_exclusive(port);
    }

    #[cfg(test)]
    pub(crate) fn continuity_offsets_for_test(
        &self,
        port: u16,
        sequence: u16,
        timestamp: u32,
    ) -> (u16, u32) {
        self.continuity_offsets(port, sequence, timestamp)
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
        self.dtmf_states.remove(&rtp_port);
        self.leased_rtp_ports.remove(&rtp_port);
        if let Some(peer_port) = peer_port {
            self.targets.remove(&peer_port);
            self.metrics.remove(&peer_port);
            self.source_bindings.remove(&peer_port);
            self.peer_ports.remove(&peer_port);
            self.codecs.remove(&peer_port);
            self.recordings.remove(&peer_port);
            self.dtmf_states.remove(&peer_port);
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
    }
}
