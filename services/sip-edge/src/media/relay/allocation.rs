use super::*;

impl MediaRelayState {
    #[cfg(test)]
    pub fn allocate_endpoint(&self, config: &MediaConfig) -> Result<RtpEndpoint, MediaError> {
        self.allocate_endpoint_for_call(config, "local-allocation")
    }

    pub fn allocate_endpoint_for_call(
        &self,
        config: &MediaConfig,
        call_id: &str,
    ) -> Result<RtpEndpoint, MediaError> {
        #[derive(serde::Deserialize)]
        struct AllocateEndpointResp {
            port: u16,
        }

        let selected_node = if let MediaRelayMode::Pool { pool } = &self.mode {
            if call_id.is_empty() {
                return Err(MediaError::Io("媒体池分配必须提供 Call-ID".to_string()));
            }
            let (node_index, node) = pool
                .node_for_call(call_id)
                .ok_or_else(|| MediaError::Io("没有健康的媒体节点".to_string()))?;
            Some((Arc::clone(pool), node_index, node))
        } else {
            None
        };

        if let Some((pool, node_index, node)) = &selected_node {
            if node.is_local() {
                let mut node_config = config.clone();
                node_config.advertised_addr = node.config.advertised_addr.clone();
                node_config.port_min = node.config.port_min;
                node_config.port_max = node.config.port_max;
                let result = self.allocate_local_endpoint(&node_config);
                match result {
                    Ok(endpoint) => {
                        pool.record_allocation(call_id, *node_index, endpoint.port);
                        return Ok(endpoint);
                    }
                    Err(error) => {
                        pool.cancel_unallocated_call(call_id);
                        return Err(error);
                    }
                }
            }
            let mut node_config = config.clone();
            node_config.advertised_addr = node.config.advertised_addr.clone();
            node_config.port_min = node.config.port_min;
            node_config.port_max = node.config.port_max;
            let request_body = serde_json::json!({ "config": node_config });
            let port_result = if let Some(uds_path) = node.uds_path().filter(|_| node.is_uds()) {
                self.call_uds(uds_path, "allocate_endpoint", request_body)
                    .and_then(|value| {
                        value
                            .get("port")
                            .and_then(serde_json::Value::as_u64)
                            .map(|port| port as u16)
                            .ok_or_else(|| "UDS response missing port".to_string())
                    })
                    .map_err(MediaError::Io)
            } else {
                let control_url = node.control_url().ok_or_else(|| {
                    MediaError::Io(format!("远程媒体节点 {} 缺少控制地址", node.config.id))
                })?;
                let url = format!("{control_url}/allocate_endpoint");
                let handle = tokio::runtime::Handle::current();
                tokio::task::block_in_place(|| {
                    handle.block_on(async {
                        let mut request = node.client.post(url);
                        if !node.config.control_token.is_empty() {
                            request =
                                request.header("X-VOS-Media-Token", &node.config.control_token);
                        }
                        request
                            .json(&request_body)
                            .send()
                            .await?
                            .json::<Result<AllocateEndpointResp, String>>()
                            .await
                    })
                })
                .map_err(|error| MediaError::Io(error.to_string()))
                .and_then(|response| response.map_err(MediaError::Io))
                .map(|response| response.port)
            };
            let port = match port_result {
                Ok(port) => port,
                Err(error) => {
                    pool.cancel_unallocated_call(call_id);
                    return Err(error);
                }
            };
            pool.record_allocation(call_id, *node_index, port);
            return Ok(RtpEndpoint::new(node.config.advertised_addr.clone(), port));
        }

        self.allocate_local_endpoint(config)
    }

    fn allocate_local_endpoint(&self, config: &MediaConfig) -> Result<RtpEndpoint, MediaError> {
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
                    self.leased_rtp_ports.remove(port);
                    continue;
                }
            };

            let rtcp_port = rtcp_port_for(port).unwrap_or(port + 1);
            let rtcp_addr = SocketAddr::from(([0, 0, 0, 0], rtcp_port));
            let rtcp_std = match std::net::UdpSocket::bind(rtcp_addr) {
                Ok(socket) => socket,
                Err(_) => {
                    self.leased_rtp_ports.remove(port);
                    continue;
                }
            };

            if let Err(error) = rtp_std.set_nonblocking(true) {
                self.leased_rtp_ports.remove(port);
                return Err(MediaError::Io(error.to_string()));
            }
            if let Err(error) = rtcp_std.set_nonblocking(true) {
                self.leased_rtp_ports.remove(port);
                return Err(MediaError::Io(error.to_string()));
            }

            let rtp_socket = tokio::net::UdpSocket::from_std(rtp_std).map_err(|error| {
                self.leased_rtp_ports.remove(port);
                MediaError::Io(error.to_string())
            })?;
            let rtcp_socket = tokio::net::UdpSocket::from_std(rtcp_std).map_err(|error| {
                self.leased_rtp_ports.remove(port);
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
