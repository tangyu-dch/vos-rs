use super::*;

impl MediaRelayState {
    #[cfg(test)]
    pub fn new() -> Self {
        Self::with_recording_pool(4, 10_000, None)
    }

    pub fn with_recording_pool(
        recording_workers: usize,
        recording_queue_capacity: usize,
        storage: Option<Arc<dyn storage_core::StorageBackend>>,
    ) -> Self {
        let conference_manager = Arc::new(crate::media::conference::ConferenceManager::new());
        crate::media::conference::start_mixer_loop(Arc::clone(&conference_manager));

        let relay = Self {
            mode: MediaRelayMode::Local,
            targets: Arc::new(DashMap::new()),
            peer_ports: Arc::new(DashMap::new()),
            codecs: Arc::new(DashMap::new()),
            metrics: Arc::new(DashMap::new()),
            recordings: Arc::new(DashMap::new()),
            recording_pool: Arc::new(RecordingPool::new(
                recording_workers,
                recording_queue_capacity,
                storage.clone(),
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
            playback_modes: Arc::new(DashMap::new()),
            playback_loops: Arc::new(DashMap::new()),
            websockets: Arc::new(DashMap::new()),
            websocket_loops: Arc::new(DashMap::new()),
            muted_ports: Arc::new(dashmap::DashSet::new()),
            talking_status: Arc::new(DashMap::new()),
            continuity: Arc::new(DashMap::new()),
            conference_manager,
            monitors: Arc::new(DashMap::new()),
            buffer_pool: Arc::new(pool::PacketBufferPool::new(MEDIA_PACKET_POOL_CAPACITY)),
            storage,
        };

        spawn_rtp_keepalive_loop(relay.clone());
        relay
    }

    /// 请求 media-edge 在指定端口启用 ICE-Lite、DTLS 与 SRTP。
    pub fn register_webrtc_session(
        &self,
        port: u16,
    ) -> Result<WebRtcSessionDescription, MediaError> {
        let params = serde_json::json!({ "port": port });
        match &self.mode {
            MediaRelayMode::Pool { .. } if !self.port_is_local(port) => {
                match self.remote_target_for_port(port) {
                    Some(RemoteControlTarget::Http {
                        client,
                        base_url,
                        control_token,
                    }) => {
                        let url = format!("{base_url}/register_webrtc_session");
                        let handle = tokio::runtime::Handle::current();
                        let response = tokio::task::block_in_place(|| {
                            handle.block_on(async {
                                let mut request = client.post(url);
                                if !control_token.is_empty() {
                                    request = request.header("X-VOS-Media-Token", control_token);
                                }
                                request
                                    .json(&params)
                                    .send()
                                    .await?
                                    .json::<Result<WebRtcSessionDescription, String>>()
                                    .await
                            })
                        })
                        .map_err(|error| MediaError::Io(error.to_string()))?;
                        response.map_err(MediaError::Io)
                    }
                    Some(RemoteControlTarget::Uds { path }) => self
                        .call_uds(&path, "register_webrtc_session", params)
                        .and_then(|value| {
                            serde_json::from_value(value).map_err(|error| error.to_string())
                        })
                        .map_err(MediaError::Io),
                    None => Err(MediaError::Io(format!(
                        "未找到 RTP 端口 {port} 对应的媒体节点"
                    ))),
                }
            }
            MediaRelayMode::Local | MediaRelayMode::Pool { .. } => Err(MediaError::Io(
                "WebRTC 仅由独立 media-edge 承载，请配置 sip_edge.media.nodes".to_string(),
            )),
        }
    }

    pub fn with_node_pool(
        config: &crate::cluster::MediaClusterConfig,
        recording_workers: usize,
        recording_queue_capacity: usize,
        storage: Option<Arc<dyn storage_core::StorageBackend>>,
    ) -> Self {
        let mut state = Self::with_recording_pool(recording_workers, recording_queue_capacity, storage);
        state.mode = MediaRelayMode::Pool {
            pool: MediaNodePool::new(config),
        };
        state
    }

    pub(crate) fn call_uds(
        &self,
        uds_path: &str,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let handle = tokio::runtime::Handle::current();
        let uds_path = uds_path.to_string();
        let method = method.to_string();

        let fut = async move {
            use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
            let mut stream = tokio::net::UnixStream::connect(&uds_path)
                .await
                .map_err(|e| format!("UDS connect failed: {e}"))?;
            let req = serde_json::json!({
                "method": method,
                "params": params
            });
            let req_str = serde_json::to_string(&req)
                .map_err(|e| format!("Request serialization failed: {e}"))?
                + "\n";

            stream
                .write_all(req_str.as_bytes())
                .await
                .map_err(|e| format!("UDS write failed: {e}"))?;

            let (reader, _) = stream.split();
            let mut reader = BufReader::new(reader);
            let mut line = String::new();
            if reader
                .read_line(&mut line)
                .await
                .map_err(|e| format!("UDS read failed: {e}"))?
                > 0
            {
                #[derive(serde::Deserialize)]
                struct UdsResponse {
                    result: Option<serde_json::Value>,
                    error: Option<String>,
                }
                let resp: UdsResponse = serde_json::from_str(&line)
                    .map_err(|e| format!("Response parse failed: {e}"))?;
                if let Some(err) = resp.error {
                    Err(err)
                } else if let Some(res) = resp.result {
                    Ok(res)
                } else {
                    Err("No result or error field".to_string())
                }
            } else {
                Err("Empty UDS response".to_string())
            }
        };

        tokio::task::block_in_place(|| handle.block_on(fut))
    }

    pub(crate) fn call_remote_target(
        &self,
        target: RemoteControlTarget,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        match target {
            RemoteControlTarget::Uds { path } => self.call_uds(&path, method, params),
            RemoteControlTarget::Http {
                client,
                base_url,
                control_token,
            } => {
                let url = format!("{base_url}/{method}");
                let handle = tokio::runtime::Handle::current();
                tokio::task::block_in_place(|| {
                    handle.block_on(async {
                        let mut request = client.post(url);
                        if !control_token.is_empty() {
                            request = request.header("X-VOS-Media-Token", control_token);
                        }
                        request
                            .json(&params)
                            .send()
                            .await
                            .map_err(|error| error.to_string())?
                            .error_for_status()
                            .map_err(|error| error.to_string())?
                            .json::<serde_json::Value>()
                            .await
                            .map_err(|error| error.to_string())
                    })
                })
            }
        }
    }

    pub(crate) fn start_monitoring(&self, port: u16, supervisor: SocketAddr) {
        self.monitors.entry(port).or_default().push(supervisor);
        self.mark_relay_features_changed(port);
        tracing::info!(port, %supervisor, "started monitoring port");
    }

    pub(crate) fn stop_monitoring(&self, port: u16, supervisor: SocketAddr) {
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

    pub(crate) fn clear_monitors(&self, port: u16) {
        if let Some(target) = self.remote_target_for_port(port) {
            let _ = self.call_remote_target(
                target,
                "clear_monitors",
                serde_json::json!({ "port": port }),
            );
            return;
        }
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
        if let Some(remote) = self.remote_target_for_port(relay_port) {
            let _ = self.call_remote_target(
                remote,
                "set_target",
                serde_json::json!({ "local_port": relay_port, "target": target }),
            );
            return;
        }
        self.targets.insert(relay_port, target);

        let binding_opt = self.source_bindings.get(&relay_port).map(|entry| *entry);
        if let Some(binding) = binding_opt {
            let target_port_opt = self.peer_ports.get(&relay_port).map(|entry| *entry);
            if let Some(target_port) = target_port_opt {
                self.targets.insert(target_port, binding.address);
                self.mark_relay_features_changed(target_port);
            }
        }
        self.mark_relay_features_changed(relay_port);
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
        self.mark_port_and_peer_features_changed(relay_port);
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

    pub fn register_port_codec(&self, port: u16, codec: rtp_core::AudioCodec) {
        self.codecs.insert(port, codec);
        self.mark_port_and_peer_features_changed(port);
    }

    pub fn clear_target(&self, relay_port: u16) {
        if let MediaRelayMode::Pool { pool } = &self.mode {
            if let Some(target) = self.remote_target_for_port(relay_port) {
                let _ = self.call_remote_target(
                    target,
                    "clear_target",
                    serde_json::json!({ "port": relay_port }),
                );
                pool.release_port(relay_port);
                return;
            }
        }
        let rtp_port = rtp_port_for(relay_port).unwrap_or(relay_port);
        let peer_port = self.peer_ports.get(&rtp_port).map(|v| *v);

        self.stop_playback(rtp_port);
        self.active_sockets.remove(&rtp_port);
        self.muted_ports.remove(&rtp_port);
        self.talking_status.remove(&rtp_port);
        self.continuity.remove(&rtp_port);
        if let Some(p_port) = peer_port {
            self.stop_playback(p_port);
            self.active_sockets.remove(&p_port);
            self.muted_ports.remove(&p_port);
            self.talking_status.remove(&p_port);
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
        self.leased_rtp_ports.remove(rtp_port);
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
        if let MediaRelayMode::Pool { pool } = &self.mode {
            pool.release_port(rtp_port);
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

pub(crate) fn spawn_rtp_keepalive_loop(relay: MediaRelayState) {
    if !cfg!(test) && tokio::runtime::Handle::try_current().is_ok() {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(2));
            loop {
                interval.tick().await;
                for entry in relay.targets.iter() {
                    let port = *entry.key();
                    let target_addr = *entry.value();
                if let Some(socket) = relay.active_sockets.get(&port) {
                    let mut keepalive_packet = vec![0u8; 13];
                    keepalive_packet[0] = 0x80;
                    keepalive_packet[1] = 13; // Comfort Noise
                    keepalive_packet[2] = 0x12;
                    keepalive_packet[3] = 0x34;
                    keepalive_packet[4] = 0x00;
                    keepalive_packet[5] = 0x00;
                    keepalive_packet[6] = 0x04;
                    keepalive_packet[7] = 0xd2;
                    keepalive_packet[8] = 0x12;
                    keepalive_packet[9] = 0x34;
                    keepalive_packet[10] = 0x56;
                    keepalive_packet[11] = 0x78;
                    keepalive_packet[12] = 0x00;
                    if let Err(e) = socket.send_to(&keepalive_packet, target_addr).await {
                        tracing::debug!("RTP keepalive send failed on port {}: {}", port, e);
                    }
                }
            }
        }
    });
    }
}
