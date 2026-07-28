use super::*;

#[cfg(test)]
use crate::media::metrics::RtcpQualitySnapshot;

impl MediaRelayState {
    pub fn target_for_port(&self, relay_port: u16) -> Option<SocketAddr> {
        self.targets.get(&relay_port).map(|entry| *entry)
    }

    pub fn metrics_for_port(&self, relay_port: u16) -> MediaRelayMetrics {
        let mut m = self
            .metrics
            .get(&relay_port)
            .map(|entry| *entry)
            .unwrap_or_default();
        if let Some(session) = self.webrtc_sessions.get(&relay_port) {
            m.webrtc_ice_connected = session
                .ice_connected
                .load(std::sync::atomic::Ordering::Acquire);
            m.webrtc_dtls_connected = session
                .dtls_connected
                .load(std::sync::atomic::Ordering::Acquire);
            m.webrtc_dtls_failed = session
                .dtls_failed
                .load(std::sync::atomic::Ordering::Acquire);
        }
        m
    }

    /// 测试辅助方法：直接注入 RTCP 质量快照，用于验证指标聚合与降级告警逻辑。
    #[cfg(test)]
    pub fn record_rtcp_reports_for_test(&self, relay_port: u16, quality: RtcpQualitySnapshot) {
        self.record_metric(relay_port, |metrics| {
            metrics.rtcp_quality = quality;
        });
    }

    pub fn metrics_totals(&self) -> MediaRelayMetrics {
        let mut totals = MediaRelayMetrics::default();
        for entry in self.metrics.iter() {
            totals.merge(*entry.value());
        }
        for session in self.webrtc_sessions.iter() {
            totals.webrtc_ice_connected |= session
                .ice_connected
                .load(std::sync::atomic::Ordering::Acquire);
            totals.webrtc_dtls_connected |= session
                .dtls_connected
                .load(std::sync::atomic::Ordering::Acquire);
            totals.webrtc_dtls_failed |= session
                .dtls_failed
                .load(std::sync::atomic::Ordering::Acquire);
        }
        totals.recording_workers = self.recording_pool.worker_count() as u64;
        totals.recording_queue_capacity = self.recording_pool.total_capacity() as u64;
        totals.recording_queue_depth = self.recording_pool.queued_commands() as u64;
        totals
    }

    pub fn pair_ports(&self, first_port: u16, second_port: u16) {
        self.peer_ports.insert(first_port, second_port);
        self.peer_ports.insert(second_port, first_port);

        let first_codec = self
            .codecs
            .get(&first_port)
            .map(|e| *e)
            .unwrap_or(rtp_core::AudioCodec::Pcmu);
        let second_codec = self
            .codecs
            .get(&second_port)
            .map(|e| *e)
            .unwrap_or(rtp_core::AudioCodec::Pcmu);

        let first_session =
            media_core::RtpPortSession::new(first_port, Some(second_port), first_codec);
        let second_session =
            media_core::RtpPortSession::new(second_port, Some(first_port), second_codec);
        self.port_sessions.insert(first_port, first_session);
        self.port_sessions.insert(second_port, second_session);

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

    #[cfg(test)]
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
        if let Some(session) = self.port_sessions.get(peer_port) {
            session.set_target(source);
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_rtcp_reports_for_test_updates_quality_metrics() {
        let state = MediaRelayState::default();
        let relay_port = 40000u16;

        let snapshot = RtcpQualitySnapshot {
            reports: 1,
            sender_reports: 1,
            receiver_reports: 0,
            report_blocks: 0,
            last_fraction_lost: Some(64),
            max_fraction_lost: Some(64),
            last_cumulative_lost: Some(10),
            max_cumulative_lost: Some(10),
            last_jitter: Some(30),
            max_jitter: Some(30),
            last_sender_report: Some(0x12345678),
            delay_since_last_sender_report: Some(15),
            last_rtt_ms: Some(50),
            max_rtt_ms: Some(50),
        };
        state.record_rtcp_reports_for_test(relay_port, snapshot);

        let metrics = state.metrics_for_port(relay_port);
        assert_eq!(metrics.rtcp_quality.reports, 1);
        assert_eq!(metrics.rtcp_quality.sender_reports, 1);
        assert_eq!(metrics.rtcp_quality.last_jitter, Some(30));
        assert_eq!(metrics.rtcp_quality.last_rtt_ms, Some(50));
        assert!(metrics.rtcp_quality.reports > 0);
    }
}
