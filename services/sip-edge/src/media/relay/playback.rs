use super::*;

impl MediaRelayState {
    pub fn start_playback(
        &self,
        port: u16,
        file_path: std::path::PathBuf,
        mode: PlaybackMode,
        loop_playback: bool,
    ) -> Result<(), String> {
        let samples = crate::media::wav::load_wav_pcm(&file_path)
            .map_err(|e| format!("加载音频文件失败: {e}"))?;

        self.stop_playback(port);

        let (cancel_tx, mut cancel_rx) = tokio::sync::oneshot::channel();
        self.playback_loops.insert(port, cancel_tx);

        let playback_state = Arc::new(std::sync::Mutex::new(PlaybackState {
            file_path,
            mode,
            loop_playback,
            samples,
            current_sample_idx: 0,
            ssrc: (unix_timestamp_millis() & 0xFFFFFFFF) as u32,
            sequence_number: (unix_timestamp_millis() & 0xFFFF) as u16,
            timestamp: ((unix_timestamp_millis() >> 16) & 0xFFFFFFFF) as u32,
        }));

        self.playbacks.insert(port, Arc::clone(&playback_state));

        let socket = self
            .active_sockets
            .get(&port)
            .map(|entry| Arc::clone(entry.value()));
        let relay = self.clone();

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_millis(20));
            interval.tick().await;
            let mut is_first = true;

            loop {
                tokio::select! {
                    _ = &mut cancel_rx => {
                        break;
                    }
                    _ = interval.tick() => {
                        let (pcm_chunk, codec, ssrc, sequence_number, timestamp) = {
                            let mut state = match playback_state.lock() {
                                Ok(s) => s,
                                Err(_) => break,
                            };
                            let codec = relay.codecs.get(&port).map(|v| *v).unwrap_or(rtp_core::AudioCodec::Pcma);
                            let start = state.current_sample_idx;
                            let end = (start + 160).min(state.samples.len());
                            let chunk = if start >= state.samples.len() {
                                if state.loop_playback {
                                    state.current_sample_idx = 0;
                                    let wrap_end = 160.min(state.samples.len());
                                    state.current_sample_idx = wrap_end;
                                    state.samples[0..wrap_end].to_vec()
                                } else {
                                    break;
                                }
                            } else {
                                state.current_sample_idx = end;
                                let mut chunk = state.samples[start..end].to_vec();
                                if chunk.len() < 160 {
                                    if state.loop_playback {
                                        state.current_sample_idx = 160 - chunk.len();
                                        let needed = 160 - chunk.len();
                                        chunk.extend_from_slice(&state.samples[0..needed.min(state.samples.len())]);
                                    } else {
                                        chunk.resize(160, 0);
                                    }
                                }
                                chunk
                            };

                            let ssrc = state.ssrc;
                            let seq = state.sequence_number;
                            let ts = state.timestamp;
                            state.sequence_number = state.sequence_number.wrapping_add(1);
                            state.timestamp = state.timestamp.wrapping_add(160);
                            (chunk, codec, ssrc, seq, ts)
                        };

                        let target = match relay.target_for_port(port) {
                            Some(t) => t,
                            None => continue,
                        };
                        if target.ip().is_unspecified() || target.port() == 0 {
                            continue;
                        }

                        let payload: Vec<u8> = match codec {
                            rtp_core::AudioCodec::Pcma => pcm_chunk.iter().map(|&s| crate::media::transcode::linear_to_alaw(s)).collect(),
                            rtp_core::AudioCodec::Pcmu => pcm_chunk.iter().map(|&s| crate::media::transcode::linear_to_ulaw(s)).collect(),
                            _ => pcm_chunk.iter().map(|&s| crate::media::transcode::linear_to_alaw(s)).collect(),
                        };
                        let payload_type = codec.static_payload_type().unwrap_or(8);
                        let rtp = rtp_core::RtpPacket {
                            marker: is_first,
                            payload_type,
                            sequence_number,
                            timestamp,
                            ssrc,
                            csrcs: Vec::new(),
                            extension: None,
                            payload,
                            padding_len: 0,
                        };
                        if is_first {
                            is_first = false;
                        }

                        let encoded = match rtp.encode() {
                            Ok(bytes) => bytes,
                            Err(_) => continue,
                        };
                        let mut final_packet = encoded;
                        if let Some(peer_port) = relay.peer_ports.get(&port).map(|entry| *entry) {
                            if let Some(session) = relay.crypto_sessions.get(&peer_port).map(|entry| entry.clone()) {
                                let mut candidate = final_packet.clone();
                                if session.lock().await.encrypt(&mut candidate).is_ok() {
                                    final_packet = candidate;
                                }
                            }
                        }

                        if let Some(socket) = &socket {
                            let _ = socket.send_to(&final_packet, target).await;
                        }
                    }
                }
            }

            let is_exclusive = playback_state
                .lock()
                .map(|state| state.mode == PlaybackMode::Exclusive)
                .unwrap_or(false);
            if is_exclusive {
                if let Some(peer_port) = relay.peer_ports.get(&port).map(|entry| *entry) {
                    relay.mark_resume_after_exclusive(peer_port);
                }
            }
            relay.playbacks.remove(&port);
            relay.playback_loops.remove(&port);
        });

        Ok(())
    }

    pub fn stop_playback(&self, port: u16) {
        if let Some((_, playback_state)) = self.playbacks.remove(&port) {
            let is_exclusive = playback_state
                .lock()
                .map(|state| state.mode == PlaybackMode::Exclusive)
                .unwrap_or(false);
            if is_exclusive {
                if let Some(peer_port) = self.peer_ports.get(&port).map(|entry| *entry) {
                    self.mark_resume_after_exclusive(peer_port);
                }
            }
        }
        if let Some(cancel_tx) = self.playback_loops.remove(&port) {
            let _ = cancel_tx.1.send(());
        }
    }
}
