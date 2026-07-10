use prometheus_client::encoding::text::encode;
use prometheus_client::metrics::counter::Counter;
use prometheus_client::metrics::gauge::Gauge;
use prometheus_client::metrics::histogram::{exponential_buckets, Histogram};
use prometheus_client::registry::Registry;
use serde::Deserialize;
use std::sync::OnceLock;

#[allow(dead_code)]
pub struct Metrics {
    pub registry: Registry,
    pub http_requests_total: Counter,
    pub http_request_duration_seconds: Histogram,
    pub active_calls: Gauge,
    pub total_calls_today: Counter,
    pub answered_calls_today: Counter,
    pub failed_calls_today: Counter,
    pub avg_mos: Gauge,
    pub avg_loss_rate: Gauge,
    pub avg_jitter_ms: Gauge,
    pub registered_users: Gauge,
    pub active_gateways: Gauge,
    #[allow(dead_code)]
    pub recordings_total: Counter,
    #[allow(dead_code)]
    pub cdr_processed_total: Counter,
    pub media_received_packets: Gauge,
    pub media_forwarded_packets: Gauge,
    pub media_dropped_invalid_packets: Gauge,
    pub media_dropped_no_target_packets: Gauge,
    pub media_send_errors: Gauge,
    pub media_recorded_packets: Gauge,
    pub media_recording_dropped_packets: Gauge,
    pub media_recording_errors: Gauge,
    pub media_recording_queue_depth: Gauge,
    pub media_recording_queue_capacity: Gauge,
    pub media_recording_workers: Gauge,
    pub media_dtmf_events: Gauge,
    pub media_rtcp_reports: Gauge,
    pub media_rtcp_max_jitter: Gauge,
    pub media_rtcp_max_rtt_ms: Gauge,
}

static METRICS: OnceLock<Metrics> = OnceLock::new();

#[derive(Debug, Deserialize)]
pub struct MediaMetricsSnapshot {
    received_packets: u64,
    forwarded_packets: u64,
    dropped_invalid_packets: u64,
    dropped_no_target_packets: u64,
    send_errors: u64,
    recorded_packets: u64,
    recording_dropped_packets: u64,
    recording_errors: u64,
    recording_queue_depth: u64,
    recording_queue_capacity: u64,
    recording_workers: u64,
    dtmf_events: u64,
    rtcp_quality: RtcpQualitySnapshot,
}

#[derive(Debug, Deserialize)]
struct RtcpQualitySnapshot {
    reports: u64,
    max_jitter: Option<u32>,
    max_rtt_ms: Option<u32>,
}

impl Metrics {
    pub fn global() -> &'static Metrics {
        METRICS.get_or_init(Self::new)
    }

    fn new() -> Self {
        let mut registry = Registry::default();

        let http_requests_total = Counter::default();
        registry.register(
            "http_requests_total",
            "Total HTTP requests",
            http_requests_total.clone(),
        );

        let http_request_duration_seconds = Histogram::new(exponential_buckets(0.005, 2.0, 10));
        registry.register(
            "http_request_duration_seconds",
            "HTTP request duration",
            http_request_duration_seconds.clone(),
        );

        let active_calls = Gauge::default();
        registry.register("active_calls", "Current active calls", active_calls.clone());

        let total_calls_today = Counter::default();
        registry.register(
            "total_calls_today",
            "Total calls today",
            total_calls_today.clone(),
        );

        let answered_calls_today = Counter::default();
        registry.register(
            "answered_calls_today",
            "Answered calls today",
            answered_calls_today.clone(),
        );

        let failed_calls_today = Counter::default();
        registry.register(
            "failed_calls_today",
            "Failed calls today",
            failed_calls_today.clone(),
        );

        let avg_mos = Gauge::default();
        registry.register("avg_mos", "Average MOS score", avg_mos.clone());

        let avg_loss_rate = Gauge::default();
        registry.register(
            "avg_loss_rate",
            "Average packet loss rate",
            avg_loss_rate.clone(),
        );

        let avg_jitter_ms = Gauge::default();
        registry.register(
            "avg_jitter_ms",
            "Average jitter in ms",
            avg_jitter_ms.clone(),
        );

        let registered_users = Gauge::default();
        registry.register(
            "registered_users",
            "Currently registered users",
            registered_users.clone(),
        );

        let active_gateways = Gauge::default();
        registry.register(
            "active_gateways",
            "Active gateways",
            active_gateways.clone(),
        );

        let recordings_total = Counter::default();
        registry.register(
            "recordings_total",
            "Total recordings created",
            recordings_total.clone(),
        );

        let cdr_processed_total = Counter::default();
        registry.register(
            "cdr_processed_total",
            "Total CDR records processed",
            cdr_processed_total.clone(),
        );

        let media_received_packets = Gauge::default();
        registry.register(
            "media_received_packets",
            "RTP and RTCP packets received by sip-edge media relay",
            media_received_packets.clone(),
        );

        let media_forwarded_packets = Gauge::default();
        registry.register(
            "media_forwarded_packets",
            "RTP and RTCP packets forwarded by sip-edge media relay",
            media_forwarded_packets.clone(),
        );

        let media_dropped_invalid_packets = Gauge::default();
        registry.register(
            "media_dropped_invalid_packets",
            "Invalid media packets dropped by sip-edge media relay",
            media_dropped_invalid_packets.clone(),
        );

        let media_dropped_no_target_packets = Gauge::default();
        registry.register(
            "media_dropped_no_target_packets",
            "Media packets dropped because no relay target was available",
            media_dropped_no_target_packets.clone(),
        );

        let media_send_errors = Gauge::default();
        registry.register(
            "media_send_errors",
            "Media packet send errors in sip-edge media relay",
            media_send_errors.clone(),
        );

        let media_recorded_packets = Gauge::default();
        registry.register(
            "media_recorded_packets",
            "RTP packets accepted by the recording queue",
            media_recorded_packets.clone(),
        );

        let media_recording_dropped_packets = Gauge::default();
        registry.register(
            "media_recording_dropped_packets",
            "RTP packets dropped because the recording queue was full",
            media_recording_dropped_packets.clone(),
        );

        let media_recording_errors = Gauge::default();
        registry.register(
            "media_recording_errors",
            "Recording write or worker errors",
            media_recording_errors.clone(),
        );

        let media_recording_queue_depth = Gauge::default();
        registry.register(
            "media_recording_queue_depth",
            "Current number of commands queued in sip-edge recording workers",
            media_recording_queue_depth.clone(),
        );

        let media_recording_queue_capacity = Gauge::default();
        registry.register(
            "media_recording_queue_capacity",
            "Total command capacity across sip-edge recording worker queues",
            media_recording_queue_capacity.clone(),
        );

        let media_recording_workers = Gauge::default();
        registry.register(
            "media_recording_workers",
            "Number of sip-edge recording workers",
            media_recording_workers.clone(),
        );

        let media_dtmf_events = Gauge::default();
        registry.register(
            "media_dtmf_events",
            "RFC 2833 telephone-event DTMF packets detected by media relay",
            media_dtmf_events.clone(),
        );

        let media_rtcp_reports = Gauge::default();
        registry.register(
            "media_rtcp_reports",
            "RTCP reports observed by media relay",
            media_rtcp_reports.clone(),
        );

        let media_rtcp_max_jitter = Gauge::default();
        registry.register(
            "media_rtcp_max_jitter",
            "Maximum RTCP jitter observed by media relay",
            media_rtcp_max_jitter.clone(),
        );

        let media_rtcp_max_rtt_ms = Gauge::default();
        registry.register(
            "media_rtcp_max_rtt_ms",
            "Maximum estimated RTCP RTT in milliseconds observed by media relay",
            media_rtcp_max_rtt_ms.clone(),
        );

        Self {
            registry,
            http_requests_total,
            http_request_duration_seconds,
            active_calls,
            total_calls_today,
            answered_calls_today,
            failed_calls_today,
            avg_mos,
            avg_loss_rate,
            avg_jitter_ms,
            registered_users,
            active_gateways,
            recordings_total,
            cdr_processed_total,
            media_received_packets,
            media_forwarded_packets,
            media_dropped_invalid_packets,
            media_dropped_no_target_packets,
            media_send_errors,
            media_recorded_packets,
            media_recording_dropped_packets,
            media_recording_errors,
            media_recording_queue_depth,
            media_recording_queue_capacity,
            media_recording_workers,
            media_dtmf_events,
            media_rtcp_reports,
            media_rtcp_max_jitter,
            media_rtcp_max_rtt_ms,
        }
    }

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
    }

    pub fn encode_metrics() -> String {
        let metrics = Self::global();
        let mut buffer = String::new();
        if encode(&mut buffer, &metrics.registry).is_err() {
            return String::new();
        }
        buffer
    }
}

fn saturating_i64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
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
        });

        let output = Metrics::encode_metrics();
        assert!(output.contains("media_received_packets 10"));
        assert!(output.contains("media_recording_dropped_packets 4"));
        assert!(output.contains("media_recording_queue_depth 12"));
        assert!(output.contains("media_rtcp_max_rtt_ms 22"));
    }
}
