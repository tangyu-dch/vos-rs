//! Shared RTP relay and RTCP quality metrics.

mod quality;
mod receive;

pub use quality::{RtcpQualitySnapshot, RtcpQualityWindow, RTCP_QUALITY_WINDOW_MS};
pub use receive::RtpReceiveStats;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaRelayMetrics {
    pub received_packets: u64,
    pub forwarded_packets: u64,
    pub dropped_invalid_packets: u64,
    pub dropped_no_target_packets: u64,
    pub send_errors: u64,
    pub learned_source_updates: u64,
    pub dropped_spoofed_packets: u64,
    pub rtcp_quality: RtcpQualitySnapshot,
    pub rtcp_window: RtcpQualityWindow,
    pub rtcp_quality_alerts: u64,
    pub rtcp_quality_degraded: bool,
    pub recorded_packets: u64,
    pub recording_dropped_packets: u64,
    pub recording_errors: u64,
    pub recording_queue_depth: u64,
    pub recording_queue_capacity: u64,
    pub recording_workers: u64,
    pub dtmf_events: u64,
    pub fast_path_packets: u64,
    pub webrtc_ice_connected: bool,
    pub webrtc_dtls_connected: bool,
    pub webrtc_dtls_failed: bool,
}

impl MediaRelayMetrics {
    /// Merges counters, nested RTCP aggregates, and connection flags.
    pub fn merge(&mut self, other: Self) {
        self.received_packets += other.received_packets;
        self.forwarded_packets += other.forwarded_packets;
        self.dropped_invalid_packets += other.dropped_invalid_packets;
        self.dropped_no_target_packets += other.dropped_no_target_packets;
        self.send_errors += other.send_errors;
        self.learned_source_updates += other.learned_source_updates;
        self.dropped_spoofed_packets += other.dropped_spoofed_packets;
        self.rtcp_quality.merge(other.rtcp_quality);
        self.rtcp_window.merge(other.rtcp_window);
        self.rtcp_quality_alerts += other.rtcp_quality_alerts;
        self.rtcp_quality_degraded |= other.rtcp_quality_degraded;
        self.recorded_packets += other.recorded_packets;
        self.recording_dropped_packets += other.recording_dropped_packets;
        self.recording_errors += other.recording_errors;
        self.recording_queue_depth += other.recording_queue_depth;
        self.recording_queue_capacity += other.recording_queue_capacity;
        self.recording_workers += other.recording_workers;
        self.dtmf_events += other.dtmf_events;
        self.fast_path_packets += other.fast_path_packets;
        self.webrtc_ice_connected |= other.webrtc_ice_connected;
        self.webrtc_dtls_connected |= other.webrtc_dtls_connected;
        self.webrtc_dtls_failed |= other.webrtc_dtls_failed;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_includes_security_dtmf_and_webrtc_fields() {
        let mut totals = MediaRelayMetrics {
            received_packets: 2,
            dropped_spoofed_packets: 1,
            ..MediaRelayMetrics::default()
        };
        totals.merge(MediaRelayMetrics {
            received_packets: 3,
            dropped_spoofed_packets: 4,
            dtmf_events: 5,
            webrtc_ice_connected: true,
            webrtc_dtls_failed: true,
            ..MediaRelayMetrics::default()
        });

        assert_eq!(totals.received_packets, 5);
        assert_eq!(totals.dropped_spoofed_packets, 5);
        assert_eq!(totals.dtmf_events, 5);
        assert!(totals.webrtc_ice_connected);
        assert!(totals.webrtc_dtls_failed);
    }
}
