use super::snapshots::MediaMetricsSnapshot;
#[cfg(test)]
use super::snapshots::{CdrMetricsSnapshot, RtcpQualitySnapshot, RtcpQualityWindow};
use super::{saturating_i64, Metrics};

impl Metrics {
    pub fn update_media_metrics(snapshot: &MediaMetricsSnapshot) {
        let metrics = Self::global();
        metrics
            .media_received_packets
            .set(saturating_i64(snapshot.received_packets));
        metrics
            .media_forwarded_packets
            .set(saturating_i64(snapshot.forwarded_packets));
        metrics
            .media_dropped_invalid_packets
            .set(saturating_i64(snapshot.dropped_invalid_packets));
        metrics
            .media_dropped_spoofed_packets
            .set(saturating_i64(snapshot.dropped_spoofed_packets));
        metrics
            .media_dropped_no_target_packets
            .set(saturating_i64(snapshot.dropped_no_target_packets));
        metrics
            .media_send_errors
            .set(saturating_i64(snapshot.send_errors));
        metrics
            .media_recorded_packets
            .set(saturating_i64(snapshot.recorded_packets));
        metrics
            .media_recording_dropped_packets
            .set(saturating_i64(snapshot.recording_dropped_packets));
        metrics
            .media_recording_errors
            .set(saturating_i64(snapshot.recording_errors));
        metrics
            .media_recording_queue_depth
            .set(saturating_i64(snapshot.recording_queue_depth));
        metrics
            .media_recording_queue_capacity
            .set(saturating_i64(snapshot.recording_queue_capacity));
        metrics
            .media_recording_workers
            .set(saturating_i64(snapshot.recording_workers));
        metrics
            .media_dtmf_events
            .set(saturating_i64(snapshot.dtmf_events));
        metrics
            .media_rtcp_reports
            .set(saturating_i64(snapshot.rtcp_quality.reports));
        metrics.media_rtcp_max_jitter.set(i64::from(
            snapshot.rtcp_quality.max_jitter.unwrap_or_default(),
        ));
        metrics.media_rtcp_max_rtt_ms.set(i64::from(
            snapshot.rtcp_quality.max_rtt_ms.unwrap_or_default(),
        ));
        metrics
            .media_rtcp_window_reports
            .set(saturating_i64(snapshot.rtcp_window.reports));
        metrics.media_rtcp_window_average_loss_rate.set(
            i64::from(
                snapshot
                    .rtcp_window
                    .average_fraction_lost
                    .unwrap_or_default(),
            ) * 10_000
                / 255,
        );
        metrics
            .media_rtcp_window_average_jitter_ms
            .set(i64::from(snapshot.rtcp_window.average_jitter.unwrap_or_default()) * 100 / 8);
        metrics.media_rtcp_window_average_rtt_ms.set(i64::from(
            snapshot.rtcp_window.average_rtt_ms.unwrap_or_default() * 100,
        ));
        metrics.media_rtcp_window_r_factor.set(i64::from(
            snapshot.rtcp_window.r_factor_x100.unwrap_or_default(),
        ));
        metrics
            .media_rtcp_window_mos
            .set(i64::from(snapshot.rtcp_window.mos_x100.unwrap_or_default()));
        metrics
            .media_rtcp_quality_alerts
            .set(saturating_i64(snapshot.rtcp_quality_alerts));
        metrics
            .webrtc_ice_connected
            .set(saturating_i64(snapshot.webrtc_ice_connected));
        metrics
            .webrtc_dtls_connected
            .set(saturating_i64(snapshot.webrtc_dtls_connected));
        metrics
            .webrtc_dtls_failed
            .set(saturating_i64(snapshot.webrtc_dtls_failed));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_media_metrics_are_encoded_for_prometheus() {
        Metrics::update_media_metrics(&MediaMetricsSnapshot {
            received_packets: 10,
            forwarded_packets: 9,
            dropped_invalid_packets: 1,
            dropped_spoofed_packets: 0,
            dropped_no_target_packets: 2,
            send_errors: 3,
            recorded_packets: 8,
            recording_dropped_packets: 4,
            recording_errors: 5,
            recording_queue_depth: 12,
            recording_queue_capacity: 4096,
            recording_workers: 4,
            dtmf_events: 6,
            rtcp_quality: RtcpQualitySnapshot {
                reports: 7,
                max_jitter: Some(11),
                max_rtt_ms: Some(22),
            },
            rtcp_window: RtcpQualityWindow::default(),
            rtcp_quality_alerts: 0,
            webrtc_ice_connected: 0,
            webrtc_dtls_connected: 0,
            webrtc_dtls_failed: 0,
        });

        let output = Metrics::encode_metrics();
        assert!(output.contains("media_received_packets 10"));
        assert!(output.contains("media_recording_dropped_packets 4"));
        assert!(output.contains("media_recording_queue_depth 12"));
        assert!(output.contains("media_rtcp_max_rtt_ms 22"));
    }

    #[test]
    fn test_cdr_pipeline_metrics_are_encoded_for_prometheus() {
        Metrics::update_cdr_metrics(&CdrMetricsSnapshot {
            queue_overflow_total: 3,
            spooled_total: 4,
            replayed_total: 2,
            spool_failures_total: 1,
            pending_spool_records: 2,
            unrecoverable_dropped_total: 0,
        });

        let output = Metrics::encode_metrics();
        assert!(output.contains("cdr_queue_overflow_total 3"));
        assert!(output.contains("cdr_spool_pending_records 2"));
        assert!(output.contains("cdr_unrecoverable_dropped_total 0"));
    }
}
