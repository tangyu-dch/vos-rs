use serde::Deserialize;

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct MediaMetricsSnapshot {
    pub(crate) received_packets: u64,
    pub(crate) forwarded_packets: u64,
    pub(crate) dropped_invalid_packets: u64,
    #[serde(default)]
    pub(crate) dropped_spoofed_packets: u64,
    pub(crate) dropped_no_target_packets: u64,
    pub(crate) send_errors: u64,
    pub(crate) recorded_packets: u64,
    pub(crate) recording_dropped_packets: u64,
    pub(crate) recording_errors: u64,
    pub(crate) recording_queue_depth: u64,
    pub(crate) recording_queue_capacity: u64,
    pub(crate) recording_workers: u64,
    pub(crate) dtmf_events: u64,
    pub(crate) rtcp_quality: RtcpQualitySnapshot,
    #[serde(default)]
    pub(crate) rtcp_window: RtcpQualityWindow,
    #[serde(default)]
    pub(crate) rtcp_quality_alerts: u64,
    #[serde(default)]
    pub webrtc_ice_connected: u64,
    #[serde(default)]
    pub webrtc_dtls_connected: u64,
    #[serde(default)]
    pub webrtc_dtls_failed: u64,
}

#[derive(Debug, Default, Deserialize)]
pub struct CdrMetricsSnapshot {
    pub(crate) queue_overflow_total: u64,
    pub(crate) spooled_total: u64,
    pub(crate) replayed_total: u64,
    pub(crate) spool_failures_total: u64,
    pub(crate) pending_spool_records: u64,
    pub(crate) unrecoverable_dropped_total: u64,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RtcpQualitySnapshot {
    pub(crate) reports: u64,
    pub(crate) max_jitter: Option<u32>,
    pub(crate) max_rtt_ms: Option<u32>,
}

#[allow(dead_code)]
#[derive(Debug, Default, Deserialize)]
pub(crate) struct RtcpQualityWindow {
    pub(crate) reports: u64,
    pub(crate) samples: u64,
    pub(crate) average_fraction_lost: Option<u8>,
    pub(crate) average_jitter: Option<u32>,
    pub(crate) average_rtt_ms: Option<u32>,
    pub(crate) r_factor_x100: Option<u16>,
    pub(crate) mos_x100: Option<u16>,
}
