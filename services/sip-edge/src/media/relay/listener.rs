use super::*;

struct CachedSourceBinding {
    address: SocketAddr,
    last_seen: std::time::Instant,
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
        let relay_clone = relay.clone();
        let socket = Arc::new(socket);
        relay.active_sockets.insert(port, Arc::clone(&socket));
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

#[allow(clippy::too_many_arguments)]
pub(crate) async fn relay_media_port(
    socket: Arc<UdpSocket>,
    local_port: u16,
    relay: MediaRelayState,
    symmetric_rtp_learning: bool,
    anti_spoofing: bool,
    source_relearn_after_secs: u64,
    packet_kind: MediaPacketKind,
    mut shutdown_rx: tokio::sync::oneshot::Receiver<()>,
) {
    let mut buffer = vec![0_u8; MAX_RTP_DATAGRAM_SIZE];
    let mut rtp_stats = RtpReceiveStats::default();
    let plan_version = relay.relay_features_version(local_port);
    let mut plan_epoch = plan_version.load(Ordering::Acquire);
    let mut plan = relay.relay_plan(local_port);
    let mut fast_path_counters = FastPathCounters::default();
    let mut metrics_flush_interval = tokio::time::interval(std::time::Duration::from_millis(100));
    metrics_flush_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    metrics_flush_interval.tick().await;
    let mut source_binding = None;
    let mut learned_symmetric_source = None;

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
            _ = metrics_flush_interval.tick() => {
                fast_path_counters.flush(&relay, local_port);
                continue;
            }
        };
        let current_epoch = plan_version.load(Ordering::Acquire);
        if current_epoch != plan_epoch {
            fast_path_counters.flush(&relay, local_port);
            plan = relay.relay_plan(local_port);
            plan_epoch = current_epoch;
        }

        let use_fast_path = packet_kind == MediaPacketKind::Rtp && plan.path == RelayPath::Fast;
        if use_fast_path {
            fast_path_counters.record_received();
        } else {
            relay.record_metric(local_port, |metrics| metrics.received_packets += 1);
        }
        debug!(local_port, packet_kind = packet_kind.label(), size, %source, "received media packet");

        if !accept_media_source(
            &relay,
            local_port,
            source,
            anti_spoofing,
            source_relearn_after_secs,
            &mut source_binding,
        ) {
            warn!(%source, local_port, packet_kind = packet_kind.label(), "dropping media packet from unbound source");
            continue;
        }

        if !use_fast_path && relay.muted_ports.contains(&local_port) {
            continue;
        }

        if use_fast_path {
            let packet = &buffer[..size];
            let is_pass_through = packet
                .first()
                .map(|first_byte| (0..=3).contains(first_byte) || (20..=63).contains(first_byte))
                .unwrap_or(false);

            if !is_pass_through {
                let rtp = match RtpPacketView::parse(packet) {
                    Ok(rtp) => rtp,
                    Err(error) => {
                        relay.record_metric(local_port, |metrics| {
                            metrics.dropped_invalid_packets += 1
                        });
                        warn!(%error, %source, local_port, "dropping invalid RTP packet on fast path");
                        fast_path_counters.flush_if_needed(&relay, local_port);
                        continue;
                    }
                };
                rtp_stats.observe(rtp);
                if plan.dtmf_payload_type == Some(rtp.payload_type) {
                    relay.process_dtmf_packet(local_port, rtp);
                }
            }

            if symmetric_rtp_learning {
                track_symmetric_source(
                    &relay,
                    local_port,
                    source,
                    packet_kind,
                    &mut learned_symmetric_source,
                );
            }

            let Some(target) = plan.target else {
                fast_path_counters.flush(&relay, local_port);
                continue;
            };
            if let Err(error) = socket.send_to(packet, target).await {
                fast_path_counters.record_send_error();
                warn!(%error, %source, %target, local_port, "failed to relay RTP packet on fast path");
            } else {
                fast_path_counters.record_forwarded();
            }
            fast_path_counters.flush_if_needed(&relay, local_port);
            continue;
        }

        let mut decrypted_packet = None;
        if packet_kind == MediaPacketKind::Rtp {
            if let Ok(view) = RtpPacketView::parse(&buffer[..size]) {
                let peer_port = plan.peer_port;
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
            if let Some(session) = &plan.crypto_session {
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

        let mut parsed_rtp = None;
        let mut rewritten_packet = None;
        if packet_kind == MediaPacketKind::Rtp {
            if let Ok(rtp) = RtpPacketView::parse(packet) {
                parsed_rtp = Some(rtp);
                // 如果当前端口属于活跃会议，将数据解包并路由至混音管理器，然后跳过单播转发
                if plan.is_in_conference {
                    let codec = plan.codec.unwrap_or(rtp_core::AudioCodec::Pcma);
                    relay
                        .conference_manager
                        .handle_rtp_packet(local_port, rtp.payload, codec);

                    // 如果开启了录音，我们依然执行录音包落盘
                    if let Some(leg) = &plan.recording {
                        let _ = leg.session.try_record(leg.channel, rtp);
                    }
                    continue;
                }

                let (seq_offset, ts_offset) =
                    relay.continuity_offsets(local_port, rtp.sequence_number, rtp.timestamp);

                if seq_offset != 0 || ts_offset != 0 {
                    if let Ok(mut owned_rtp) = rtp_core::RtpPacket::parse(packet) {
                        owned_rtp.sequence_number =
                            owned_rtp.sequence_number.wrapping_sub(seq_offset);
                        owned_rtp.timestamp = owned_rtp.timestamp.wrapping_sub(ts_offset);
                        if let Ok(encoded) = owned_rtp.encode() {
                            rewritten_packet = Some(encoded);
                        }
                    }
                }
            }
        }
        let packet = rewritten_packet.as_deref().unwrap_or(packet);

        let first_byte = packet[0];
        let is_pass_through = (0..=3).contains(&first_byte) || (20..=63).contains(&first_byte);

        let summary = if is_pass_through {
            None
        } else if rewritten_packet.is_none() {
            match parsed_rtp {
                Some(rtp_packet) => Some(MediaPacketSummary {
                    rtp_packet: Some(rtp_packet),
                    ..MediaPacketSummary::default()
                }),
                None => match packet_kind.inspect(packet) {
                    Ok(summary) => Some(summary),
                    Err(error) => {
                        relay.record_metric(local_port, |metrics| {
                            metrics.dropped_invalid_packets += 1
                        });
                        warn!(%error, %source, local_port, packet_kind = packet_kind.label(), "dropping invalid media packet");
                        continue;
                    }
                },
            }
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
                rtp_stats.observe(rtp_packet);
                rtp_stats.receiver_report()
            });

        if symmetric_rtp_learning {
            track_symmetric_source(
                &relay,
                local_port,
                source,
                packet_kind,
                &mut learned_symmetric_source,
            );
        }

        let peer_port = relay.peer_ports.get(&local_port).map(|entry| *entry);
        if let Some(p_port) = peer_port {
            if let Some(playback) = relay.playbacks.get(&p_port) {
                if playback.lock().unwrap().mode == PlaybackMode::Exclusive {
                    continue;
                }
            }
        }

        let Some(target) = plan.target else {
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
                if let Some(leg) = &plan.recording {
                    match leg.session.try_record(leg.channel, *rtp_packet) {
                        Ok(true) => {
                            relay
                                .record_metric(local_port, |metrics| metrics.recorded_packets += 1);
                        }
                        Ok(false) => {}
                        Err(error) => {
                            if error.kind() == std::io::ErrorKind::WouldBlock {
                                relay.record_metric(local_port, |metrics| {
                                    metrics.recording_dropped_packets += 1
                                });
                            } else {
                                relay.record_metric(local_port, |metrics| {
                                    metrics.recording_errors += 1
                                });
                                warn!(%error, %source, local_port, "failed to record RTP packet");
                            }
                        }
                    }
                }
            }
        }

        let mut transcoded_packet = None;
        if packet_kind == MediaPacketKind::Rtp && plan.peer_port.is_some() {
            if let (Some(local_codec), Some(peer_codec)) = (plan.codec, plan.peer_codec) {
                if local_codec != peer_codec {
                    if let Ok(mut rtp) = rtp_core::RtpPacket::parse(packet) {
                        let new_payload = match (local_codec, peer_codec) {
                            (rtp_core::AudioCodec::Pcma, rtp_core::AudioCodec::Pcmu) => Some(
                                crate::media::transcode::transcode_pcma_to_pcmu(&rtp.payload),
                            ),
                            (rtp_core::AudioCodec::Pcmu, rtp_core::AudioCodec::Pcma) => Some(
                                crate::media::transcode::transcode_pcmu_to_pcma(&rtp.payload),
                            ),
                            _ => None,
                        };
                        if let Some(payload) = new_payload {
                            rtp.payload = payload;
                            if let Some(pt) = peer_codec.static_payload_type() {
                                rtp.payload_type = pt;
                            }
                            if let Ok(encoded) = rtp.encode() {
                                transcoded_packet = Some(encoded);
                            }
                        }
                    }
                }
            }
        }
        let packet = transcoded_packet.as_deref().unwrap_or(packet);

        let mut encrypted_packet = None;
        if packet_kind == MediaPacketKind::Rtp && plan.peer_port.is_some() {
            if let Some(session) = &plan.peer_crypto_session {
                let mut candidate = packet.to_vec();
                if let Err(error) = session.lock().await.encrypt(&mut candidate) {
                    relay.record_metric(local_port, |metrics| metrics.send_errors += 1);
                    warn!(%error, local_port, "failed to encrypt RTP packet for relay target");
                    continue;
                }
                encrypted_packet = Some(candidate);
            }
        }
        // 旁听/监控：如果本端口配置了监控目标，复制 RTP 包发送给监控端
        if let Some(monitors_list) = relay.monitors.get(&local_port) {
            for &monitor_addr in monitors_list.iter() {
                // 用 plain/decrypted packet，确保旁听者听到的是解密后的明文声音
                let _ = socket.send_to(packet, monitor_addr).await;
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

    fast_path_counters.flush(&relay, local_port);
}

fn track_symmetric_source(
    relay: &MediaRelayState,
    local_port: u16,
    source: SocketAddr,
    packet_kind: MediaPacketKind,
    learned_source: &mut Option<SocketAddr>,
) {
    if *learned_source == Some(source) {
        return;
    }
    *learned_source = Some(source);
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

fn accept_media_source(
    relay: &MediaRelayState,
    local_port: u16,
    source: SocketAddr,
    anti_spoofing: bool,
    relearn_after_secs: u64,
    binding: &mut Option<CachedSourceBinding>,
) -> bool {
    if !anti_spoofing {
        return true;
    }

    let now = std::time::Instant::now();
    match binding {
        Some(current) if current.address == source => {
            current.last_seen = now;
            true
        }
        Some(current)
            if now.duration_since(current.last_seen)
                < std::time::Duration::from_secs(relearn_after_secs) =>
        {
            relay.record_metric(local_port, |metrics| metrics.dropped_spoofed_packets += 1);
            false
        }
        _ => {
            *binding = Some(CachedSourceBinding {
                address: source,
                last_seen: now,
            });
            relay
                .source_bindings
                .insert(local_port, SourceBinding { address: source });
            true
        }
    }
}
