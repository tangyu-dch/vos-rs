use super::*;
use futures::{SinkExt, StreamExt};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tracing::{info, warn};

impl MediaRelayState {
    pub async fn start_stream(
        &self,
        port: u16,
        websocket_url: String,
        format: String,
        _barge_in: bool,
    ) -> Result<(), String> {
        self.stop_stream(port);

        let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel::<()>();
        self.websocket_loops.insert(port, cancel_tx);

        let (ws_tx, mut ws_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(100);
        self.websockets.insert(port, ws_tx);

        let url = match reqwest::Url::parse(&websocket_url) {
            Ok(u) => u,
            Err(e) => return Err(format!("Invalid WebSocket URL: {e}")),
        };

        let codec = self
            .codecs
            .get(&port)
            .map(|v| *v)
            .unwrap_or(rtp_core::AudioCodec::Pcma);
        let active_socket = self
            .active_sockets
            .get(&port)
            .map(|v| Arc::clone(v.value()));
        let target = self.target_for_port(port);
        let relay = self.clone();

        tokio::spawn(async move {
            info!(port, url = %websocket_url, "Connecting to WebSocket AI stream");
            let ws_stream = match connect_async(url).await {
                Ok((s, _)) => s,
                Err(e) => {
                    warn!(port, "Failed to connect to WebSocket AI stream: {:?}", e);
                    return;
                }
            };

            let (mut write_half, mut read_half) = ws_stream.split();

            // Task to read from channel (from RTP) and send to WebSocket
            let format_clone = format.clone();
            let outbound_loop = tokio::spawn(async move {
                while let Some(pcm_bytes) = ws_rx.recv().await {
                    if format_clone.contains("16k") || format_clone.contains("16000") {
                        // Resample from 8kHz PCM to 16kHz PCM by duplicating samples
                        let samples = pcm_bytes
                            .chunks_exact(2)
                            .map(|c| i16::from_le_bytes([c[0], c[1]]))
                            .collect::<Vec<i16>>();
                        let mut resampled = Vec::with_capacity(samples.len() * 4);
                        for s in samples {
                            resampled.extend_from_slice(&s.to_le_bytes());
                            resampled.extend_from_slice(&s.to_le_bytes());
                        }
                        if write_half.send(WsMessage::Binary(resampled)).await.is_err() {
                            break;
                        }
                    } else if format_clone == "pcma" {
                        // Convert 8kHz PCM back to PCMA (G.711a)
                        let samples = pcm_bytes
                            .chunks_exact(2)
                            .map(|c| i16::from_le_bytes([c[0], c[1]]))
                            .collect::<Vec<i16>>();
                        let alaw: Vec<u8> = samples
                            .iter()
                            .map(|&s| crate::media::transcode::linear_to_alaw(s))
                            .collect();
                        if write_half.send(WsMessage::Binary(alaw)).await.is_err() {
                            break;
                        }
                    } else if format_clone == "pcmu" {
                        // Convert 8kHz PCM back to PCMU (G.711u)
                        let samples = pcm_bytes
                            .chunks_exact(2)
                            .map(|c| i16::from_le_bytes([c[0], c[1]]))
                            .collect::<Vec<i16>>();
                        let ulaw: Vec<u8> = samples
                            .iter()
                            .map(|&s| crate::media::transcode::linear_to_ulaw(s))
                            .collect();
                        if write_half.send(WsMessage::Binary(ulaw)).await.is_err() {
                            break;
                        }
                    } else {
                        // Default raw 8kHz PCM
                        if write_half.send(WsMessage::Binary(pcm_bytes)).await.is_err() {
                            break;
                        }
                    }
                }
            });

            // Task to read from WebSocket and send to RTP
            let relay_clone = relay.clone();
            let inbound_loop = tokio::spawn(async move {
                let mut sequence_number = 0u16;
                let mut timestamp = 0u32;
                let ssrc = 0x12345678u32;

                while let Some(Ok(msg)) = read_half.next().await {
                    if let WsMessage::Binary(data) = msg {
                        let pcm_samples = if format.contains("16k") || format.contains("16000") {
                            // Resample from 16kHz PCM to 8kHz PCM by decimating
                            let samples = data
                                .chunks_exact(2)
                                .map(|c| i16::from_le_bytes([c[0], c[1]]))
                                .collect::<Vec<i16>>();
                            samples.iter().step_by(2).copied().collect::<Vec<i16>>()
                        } else if format == "pcma" {
                            data.iter()
                                .map(|&b| crate::media::recording::decode_pcma(b))
                                .collect::<Vec<i16>>()
                        } else if format == "pcmu" {
                            data.iter()
                                .map(|&b| crate::media::recording::decode_pcmu(b))
                                .collect::<Vec<i16>>()
                        } else {
                            data.chunks_exact(2)
                                .map(|c| i16::from_le_bytes([c[0], c[1]]))
                                .collect::<Vec<i16>>()
                        };

                        // Send G.711 frames in 20ms chunks (160 samples per chunk)
                        for chunk in pcm_samples.chunks(160) {
                            if chunk.len() < 160 {
                                break;
                            }
                            let payload: Vec<u8> = match codec {
                                rtp_core::AudioCodec::Pcma => chunk
                                    .iter()
                                    .map(|&s| crate::media::transcode::linear_to_alaw(s))
                                    .collect(),
                                rtp_core::AudioCodec::Pcmu => chunk
                                    .iter()
                                    .map(|&s| crate::media::transcode::linear_to_ulaw(s))
                                    .collect(),
                                _ => chunk
                                    .iter()
                                    .map(|&s| crate::media::transcode::linear_to_alaw(s))
                                    .collect(),
                            };

                            let payload_type = codec.static_payload_type().unwrap_or(8);
                            let rtp = rtp_core::RtpPacket {
                                marker: false,
                                payload_type,
                                sequence_number,
                                timestamp,
                                ssrc,
                                csrcs: Vec::new(),
                                extension: None,
                                payload,
                                padding_len: 0,
                            };

                            sequence_number = sequence_number.wrapping_add(1);
                            timestamp = timestamp.wrapping_add(160);

                            if let Ok(encoded) = rtp.encode() {
                                let mut final_packet = encoded;
                                if let Some(peer_port) =
                                    relay_clone.peer_ports.get(&port).map(|entry| *entry)
                                {
                                    if let Some(session) = relay_clone
                                        .crypto_sessions
                                        .get(&peer_port)
                                        .map(|entry| entry.clone())
                                    {
                                        let mut candidate = final_packet.clone();
                                        if session.lock().await.encrypt(&mut candidate).is_ok() {
                                            final_packet = candidate;
                                        }
                                    }
                                }
                                if let (Some(socket), Some(tgt)) = (&active_socket, target) {
                                    let _ = socket.send_to(&final_packet, tgt).await;
                                }
                            }
                        }
                    }
                }
            });

            // Wait for cancel or completion
            let _ = cancel_rx.await;
            outbound_loop.abort();
            inbound_loop.abort();
        });

        Ok(())
    }

    pub fn stop_stream(&self, port: u16) {
        self.websockets.remove(&port);
        if let Some((_, cancel_tx)) = self.websocket_loops.remove(&port) {
            let _ = cancel_tx.send(());
        }
    }
}
