use super::*;

impl MediaRelayState {
    pub fn allocate_endpoint(&self, config: &MediaConfig) -> Result<RtpEndpoint, MediaError> {
        let port_min = config.port_min;
        let port_max = config.port_max;
        let available_ports = ((port_max - port_min) / 2) + 1;

        let mut port_candidate = self.next_port.load(Ordering::Relaxed) as u16;
        if port_candidate < port_min || port_candidate > port_max {
            port_candidate = port_min;
            self.next_port
                .store(port_candidate as u32, Ordering::Relaxed);
        }

        for _ in 0..available_ports {
            let port = port_candidate;
            port_candidate = next_rtp_port(port, config);
            self.next_port
                .store(port_candidate as u32, Ordering::Relaxed);

            if !self.leased_rtp_ports.insert(port) {
                continue;
            }
            let rtp_addr = SocketAddr::from(([0, 0, 0, 0], port));
            let rtp_std = match std::net::UdpSocket::bind(rtp_addr) {
                Ok(socket) => socket,
                Err(error) => {
                    warn!(port, %error, "failed to bind RTP socket");
                    self.leased_rtp_ports.remove(&port);
                    continue;
                }
            };

            let rtcp_port = rtcp_port_for(port).unwrap_or(port + 1);
            let rtcp_addr = SocketAddr::from(([0, 0, 0, 0], rtcp_port));
            let rtcp_std = match std::net::UdpSocket::bind(rtcp_addr) {
                Ok(socket) => socket,
                Err(_) => {
                    self.leased_rtp_ports.remove(&port);
                    continue;
                }
            };

            if let Err(error) = rtp_std.set_nonblocking(true) {
                self.leased_rtp_ports.remove(&port);
                return Err(MediaError::Io(error.to_string()));
            }
            if let Err(error) = rtcp_std.set_nonblocking(true) {
                self.leased_rtp_ports.remove(&port);
                return Err(MediaError::Io(error.to_string()));
            }

            let rtp_socket = tokio::net::UdpSocket::from_std(rtp_std).map_err(|error| {
                self.leased_rtp_ports.remove(&port);
                MediaError::Io(error.to_string())
            })?;
            let rtcp_socket = tokio::net::UdpSocket::from_std(rtcp_std).map_err(|error| {
                self.leased_rtp_ports.remove(&port);
                MediaError::Io(error.to_string())
            })?;
            let (rtp_tx, rtp_rx) = tokio::sync::oneshot::channel();
            let (rtcp_tx, rtcp_rx) = tokio::sync::oneshot::channel();
            let rtp_socket = Arc::new(rtp_socket);
            let rtcp_socket = Arc::new(rtcp_socket);
            self.active_sockets.insert(port, Arc::clone(&rtp_socket));
            self.active_sockets
                .insert(rtcp_port, Arc::clone(&rtcp_socket));

            let rtp_learning = config.symmetric_rtp_learning;
            tokio::spawn(relay_media_port(
                rtp_socket,
                port,
                self.clone(),
                rtp_learning,
                config.anti_spoofing,
                config.source_relearn_after_secs,
                MediaPacketKind::Rtp,
                rtp_rx,
            ));
            tokio::spawn(relay_media_port(
                rtcp_socket,
                rtcp_port,
                self.clone(),
                rtp_learning,
                config.anti_spoofing,
                config.source_relearn_after_secs,
                MediaPacketKind::Rtcp,
                rtcp_rx,
            ));
            self.active_loops.insert(port, vec![rtp_tx, rtcp_tx]);
            debug!(
                port,
                rtcp_port, "allocated media relay endpoint (lock-free)"
            );
            return Ok(RtpEndpoint::new(config.advertised_addr.clone(), port));
        }

        Err(MediaError::PortRangeExhausted {
            port_min: config.port_min,
            port_max: config.port_max,
        })
    }
}
