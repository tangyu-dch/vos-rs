use super::*;

impl MediaRelayState {
    pub fn target_for_port(&self, relay_port: u16) -> Option<SocketAddr> {
        self.targets.get(&relay_port).map(|entry| *entry)
    }

    pub fn metrics_for_port(&self, relay_port: u16) -> MediaRelayMetrics {
        if let Some(target) = self.remote_target_for_port(relay_port) {
            return self
                .call_remote_target(
                    target,
                    "metrics_for_port",
                    serde_json::json!({ "port": relay_port }),
                )
                .ok()
                .and_then(|value| serde_json::from_value::<Option<MediaRelayMetrics>>(value).ok())
                .flatten()
                .unwrap_or_default();
        }
        self.metrics
            .get(&relay_port)
            .map(|entry| *entry)
            .unwrap_or_default()
    }

    #[allow(dead_code)]
    pub fn record_rtcp_reports_for_test(&self, relay_port: u16, quality: RtcpQualitySnapshot) {
        self.record_metric(relay_port, |metrics| {
            metrics.rtcp_quality = quality;
        });
    }

    pub fn metrics_totals(&self) -> MediaRelayMetrics {
        if let MediaRelayMode::Pool { pool } = &self.mode {
            let mut totals = MediaRelayMetrics::default();
            for node in pool.nodes() {
                if node.is_local() {
                    continue;
                }
                let target = match super::remote_target_for_node(node) {
                    Some(target) => target,
                    None => continue,
                };
                if let Ok(value) =
                    self.call_remote_target(target, "metrics_totals", serde_json::json!({}))
                {
                    if let Ok(metrics) = serde_json::from_value::<MediaRelayMetrics>(value) {
                        totals.merge(metrics);
                    }
                }
            }
            for entry in self.metrics.iter() {
                totals.merge(*entry.value());
            }
            totals.recording_workers = self.recording_pool.worker_count() as u64;
            totals.recording_queue_capacity = self.recording_pool.total_capacity() as u64;
            totals.recording_queue_depth = self.recording_pool.queued_commands() as u64;
            return totals;
        }
        let mut totals = MediaRelayMetrics::default();
        for entry in self.metrics.iter() {
            totals.merge(*entry.value());
        }
        totals.recording_workers = self.recording_pool.worker_count() as u64;
        totals.recording_queue_capacity = self.recording_pool.total_capacity() as u64;
        totals.recording_queue_depth = self.recording_pool.queued_commands() as u64;
        totals
    }

    pub fn pair_ports(&self, first_port: u16, second_port: u16) {
        if let MediaRelayMode::Pool { pool } = &self.mode {
            let first_node = pool.node_for_port(first_port);
            let second_node = pool.node_for_port(second_port);
            let same_node = matches!((&first_node, &second_node), (Some(left), Some(right)) if left.config.id == right.config.id);
            if !same_node {
                tracing::error!(first_port, second_port, "拒绝跨媒体节点配对 RTP 端口");
                return;
            }
            if let Some(target) = self.remote_target_for_port(first_port) {
                let _ = self.call_remote_target(
                    target,
                    "pair_ports",
                    serde_json::json!({ "port_a": first_port, "port_b": second_port }),
                );
                return;
            }
        }
        self.peer_ports.insert(first_port, second_port);
        self.peer_ports.insert(second_port, first_port);

        if let (Some(first_rtcp_port), Some(second_rtcp_port)) =
            (rtcp_port_for(first_port), rtcp_port_for(second_port))
        {
            self.peer_ports.insert(first_rtcp_port, second_rtcp_port);
            self.peer_ports.insert(second_rtcp_port, first_rtcp_port);
        }
        self.mark_relay_features_changed(first_port);
        self.mark_relay_features_changed(second_port);
        if let Some(first_rtcp_port) = rtcp_port_for(first_port) {
            self.mark_relay_features_changed(first_rtcp_port);
        }
        if let Some(second_rtcp_port) = rtcp_port_for(second_port) {
            self.mark_relay_features_changed(second_rtcp_port);
        }
    }

    pub fn peer_port_for(&self, relay_port: u16) -> Option<u16> {
        self.peer_ports.get(&relay_port).map(|entry| *entry)
    }

    pub(super) fn learn_symmetric_source(
        &self,
        relay_port: u16,
        source: SocketAddr,
    ) -> Option<SymmetricSourceUpdate> {
        let peer_port = *self.peer_ports.get(&relay_port)?;
        let previous_target = self.targets.insert(peer_port, source);
        if previous_target == Some(source) {
            return None;
        }
        self.source_bindings
            .insert(relay_port, SourceBinding { address: source });
        self.mark_relay_features_changed(peer_port);
        self.mark_relay_features_changed(relay_port);

        self.metrics
            .entry(relay_port)
            .or_default()
            .learned_source_updates += 1;

        Some(SymmetricSourceUpdate {
            source_port: relay_port,
            target_port: peer_port,
            previous_target,
            learned_target: source,
        })
    }

    pub(super) fn record_metric(
        &self,
        relay_port: u16,
        update: impl FnOnce(&mut MediaRelayMetrics),
    ) {
        update(self.metrics.entry(relay_port).or_default().value_mut());
    }

    pub(super) fn record_rtcp_reports(&self, relay_port: u16, summary: &MediaPacketSummary) {
        if summary.rtcp_quality.reports == 0 {
            return;
        }

        self.record_metric(relay_port, |metrics| {
            metrics.rtcp_quality.merge(summary.rtcp_quality);
            metrics.rtcp_window.observe(summary.rtcp_quality);
            let degraded = metrics.rtcp_window.is_degraded();
            if degraded && !metrics.rtcp_quality_degraded {
                metrics.rtcp_quality_alerts += 1;
                warn!(
                    relay_port,
                    mos_x100 = ?metrics.rtcp_window.mos_x100,
                    r_factor_x100 = ?metrics.rtcp_window.r_factor_x100,
                    average_loss_x10000 = ?metrics
                        .rtcp_window
                        .average_fraction_lost
                        .map(|value| u32::from(value) * 10_000 / 255),
                    "RTCP quality degraded"
                );
            }
            metrics.rtcp_quality_degraded = degraded;
        });
    }
}
