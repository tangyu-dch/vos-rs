use prometheus_client::encoding::text::encode;
use prometheus_client::metrics::counter::Counter;
use prometheus_client::metrics::family::Family;
use prometheus_client::metrics::gauge::Gauge;
use prometheus_client::metrics::histogram::{exponential_buckets, Histogram};
use prometheus_client::registry::Registry;
use std::sync::OnceLock;

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

        let http_request_duration_seconds =
            Histogram::new(exponential_buckets(0.005, 2.0, 10));
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
        registry.register("avg_loss_rate", "Average packet loss rate", avg_loss_rate.clone());

        let avg_jitter_ms = Gauge::default();
        registry.register("avg_jitter_ms", "Average jitter in ms", avg_jitter_ms.clone());

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
        }
    }

    pub fn encode_metrics() -> String {
        let metrics = Self::global();
        let mut buffer = String::new();
        encode(&mut buffer, &metrics.registry).unwrap();
        buffer
    }
}
