use prometheus_client::encoding::text::encode;
use prometheus_client::metrics::counter::Counter;
use prometheus_client::metrics::gauge::Gauge;
use prometheus_client::metrics::histogram::{exponential_buckets, Histogram};
use prometheus_client::registry::Registry;
use std::sync::OnceLock;

mod snapshots;
mod rtcp;
pub use snapshots::{CdrMetricsSnapshot, MediaMetricsSnapshot};

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
    pub cdr_queue_overflow_total: Gauge,
    pub cdr_spooled_total: Gauge,
    pub cdr_replayed_total: Gauge,
    pub cdr_spool_failures_total: Gauge,
    pub cdr_spool_pending_records: Gauge,
    pub cdr_unrecoverable_dropped_total: Gauge,
    pub media_received_packets: Gauge,
    pub media_forwarded_packets: Gauge,
    pub media_dropped_invalid_packets: Gauge,
    pub media_dropped_spoofed_packets: Gauge,
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
    pub media_rtcp_window_reports: Gauge,
    pub media_rtcp_window_average_loss_rate: Gauge,
    pub media_rtcp_window_average_jitter_ms: Gauge,
    pub media_rtcp_window_average_rtt_ms: Gauge,
    pub media_rtcp_window_r_factor: Gauge,
    pub media_rtcp_window_mos: Gauge,
    pub media_rtcp_quality_alerts: Gauge,
    pub webrtc_ice_connected: Gauge,
    pub webrtc_dtls_connected: Gauge,
    pub webrtc_dtls_failed: Gauge,
}

static METRICS: OnceLock<Metrics> = OnceLock::new();

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
        let cdr_queue_overflow_total = Gauge::default();
        registry.register(
            "cdr_queue_overflow_total",
            "CDRs diverted because the in-memory queue was full",
            cdr_queue_overflow_total.clone(),
        );
        let cdr_spooled_total = Gauge::default();
        registry.register(
            "cdr_spooled_total",
            "CDRs appended to the durable local replay spool",
            cdr_spooled_total.clone(),
        );
        let cdr_replayed_total = Gauge::default();
        registry.register(
            "cdr_replayed_total",
            "CDRs successfully replayed from the durable spool",
            cdr_replayed_total.clone(),
        );
        let cdr_spool_failures_total = Gauge::default();
        registry.register(
            "cdr_spool_failures_total",
            "CDR spool append, rotation, or decoding failures",
            cdr_spool_failures_total.clone(),
        );
        let cdr_spool_pending_records = Gauge::default();
        registry.register(
            "cdr_spool_pending_records",
            "CDR records currently waiting in the durable replay spool",
            cdr_spool_pending_records.clone(),
        );
        let cdr_unrecoverable_dropped_total = Gauge::default();
        registry.register(
            "cdr_unrecoverable_dropped_total",
            "CDRs lost after both queue delivery and durable spool append failed",
            cdr_unrecoverable_dropped_total.clone(),
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
        let media_dropped_spoofed_packets = Gauge::default();
        registry.register(
            "media_dropped_spoofed_packets",
            "Media packets dropped because the RTP source was not bound",
            media_dropped_spoofed_packets.clone(),
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

        let media_rtcp_window_reports = Gauge::default();
        registry.register(
            "media_rtcp_window_reports",
            "RTCP reports in the current quality window",
            media_rtcp_window_reports.clone(),
        );
        let media_rtcp_window_average_loss_rate = Gauge::default();
        registry.register(
            "media_rtcp_window_average_loss_rate_x10000",
            "Average RTCP fraction lost multiplied by 10000 in the current quality window",
            media_rtcp_window_average_loss_rate.clone(),
        );
        let media_rtcp_window_average_jitter_ms = Gauge::default();
        registry.register(
            "media_rtcp_window_average_jitter_ms_x100",
            "Average RTCP jitter in milliseconds multiplied by 100 in the current quality window",
            media_rtcp_window_average_jitter_ms.clone(),
        );
        let media_rtcp_window_average_rtt_ms = Gauge::default();
        registry.register(
            "media_rtcp_window_average_rtt_ms_x100",
            "Average RTCP RTT in milliseconds multiplied by 100 in the current quality window",
            media_rtcp_window_average_rtt_ms.clone(),
        );
        let media_rtcp_window_r_factor = Gauge::default();
        registry.register(
            "media_rtcp_window_r_factor_x100",
            "Estimated R-factor multiplied by 100 in the current quality window",
            media_rtcp_window_r_factor.clone(),
        );
        let media_rtcp_window_mos = Gauge::default();
        registry.register(
            "media_rtcp_window_mos_x100",
            "Estimated MOS multiplied by 100 in the current quality window",
            media_rtcp_window_mos.clone(),
        );
        let media_rtcp_quality_alerts = Gauge::default();
        registry.register(
            "media_rtcp_quality_alerts",
            "RTCP quality degradation transitions observed by media relay",
            media_rtcp_quality_alerts.clone(),
        );

        let webrtc_ice_connected = Gauge::default();
        registry.register(
            "webrtc_ice_connected",
            "Current active WebRTC ICE connections",
            webrtc_ice_connected.clone(),
        );

        let webrtc_dtls_connected = Gauge::default();
        registry.register(
            "webrtc_dtls_connected",
            "Current active WebRTC DTLS connections",
            webrtc_dtls_connected.clone(),
        );

        let webrtc_dtls_failed = Gauge::default();
        registry.register(
            "webrtc_dtls_failed",
            "Total failed WebRTC DTLS connections",
            webrtc_dtls_failed.clone(),
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
            cdr_queue_overflow_total,
            cdr_spooled_total,
            cdr_replayed_total,
            cdr_spool_failures_total,
            cdr_spool_pending_records,
            cdr_unrecoverable_dropped_total,
            media_received_packets,
            media_forwarded_packets,
            media_dropped_invalid_packets,
            media_dropped_spoofed_packets,
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
            media_rtcp_window_reports,
            media_rtcp_window_average_loss_rate,
            media_rtcp_window_average_jitter_ms,
            media_rtcp_window_average_rtt_ms,
            media_rtcp_window_r_factor,
            media_rtcp_window_mos,
            media_rtcp_quality_alerts,
            webrtc_ice_connected,
            webrtc_dtls_connected,
            webrtc_dtls_failed,
        }
    }

    pub fn update_cdr_metrics(snapshot: &CdrMetricsSnapshot) {
        let metrics = Self::global();
        metrics
            .cdr_queue_overflow_total
            .set(saturating_i64(snapshot.queue_overflow_total));
        metrics
            .cdr_spooled_total
            .set(saturating_i64(snapshot.spooled_total));
        metrics
            .cdr_replayed_total
            .set(saturating_i64(snapshot.replayed_total));
        metrics
            .cdr_spool_failures_total
            .set(saturating_i64(snapshot.spool_failures_total));
        metrics
            .cdr_spool_pending_records
            .set(saturating_i64(snapshot.pending_spool_records));
        metrics
            .cdr_unrecoverable_dropped_total
            .set(saturating_i64(snapshot.unrecoverable_dropped_total));
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
