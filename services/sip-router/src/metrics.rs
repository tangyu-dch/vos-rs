use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

static UDP_RECEIVED: AtomicU64 = AtomicU64::new(0);
static UDP_ROUTED: AtomicU64 = AtomicU64::new(0);
static UDP_DROPPED: AtomicU64 = AtomicU64::new(0);
static UDP_ERRORS: AtomicU64 = AtomicU64::new(0);
static TCP_ACTIVE: AtomicI64 = AtomicI64::new(0);
static TCP_REJECTED: AtomicU64 = AtomicU64::new(0);
static TCP_FRAMES: AtomicU64 = AtomicU64::new(0);
static SECURITY_REJECTED: AtomicU64 = AtomicU64::new(0);
static DISCOVERED_NODES: AtomicI64 = AtomicI64::new(0);
static ACTIVE_TRANSACTIONS: AtomicI64 = AtomicI64::new(0);
static REDIS_ERRORS: AtomicU64 = AtomicU64::new(0);

pub(crate) fn udp_received() {
    UDP_RECEIVED.fetch_add(1, Ordering::Relaxed);
}
pub(crate) fn udp_routed() {
    UDP_ROUTED.fetch_add(1, Ordering::Relaxed);
}
pub(crate) fn udp_dropped() {
    UDP_DROPPED.fetch_add(1, Ordering::Relaxed);
}
pub(crate) fn udp_error() {
    UDP_ERRORS.fetch_add(1, Ordering::Relaxed);
}
pub(crate) fn tcp_opened() {
    TCP_ACTIVE.fetch_add(1, Ordering::Relaxed);
}
pub(crate) fn tcp_closed() {
    TCP_ACTIVE.fetch_sub(1, Ordering::Relaxed);
}
pub(crate) fn tcp_rejected() {
    TCP_REJECTED.fetch_add(1, Ordering::Relaxed);
}
pub(crate) fn tcp_frame() {
    TCP_FRAMES.fetch_add(1, Ordering::Relaxed);
}
pub(crate) fn security_rejected() {
    SECURITY_REJECTED.fetch_add(1, Ordering::Relaxed);
}
pub(crate) fn discovered_nodes(count: usize) {
    DISCOVERED_NODES.store(count as i64, Ordering::Relaxed);
}
pub(crate) fn active_transactions(count: usize) {
    ACTIVE_TRANSACTIONS.store(count as i64, Ordering::Relaxed);
}
pub(crate) fn redis_error() {
    REDIS_ERRORS.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn render() -> String {
    format!(
        concat!(
            "# TYPE vos_rs_sip_router_udp_received_total counter\nvos_rs_sip_router_udp_received_total {}\n",
            "# TYPE vos_rs_sip_router_udp_routed_total counter\nvos_rs_sip_router_udp_routed_total {}\n",
            "# TYPE vos_rs_sip_router_udp_dropped_total counter\nvos_rs_sip_router_udp_dropped_total {}\n",
            "# TYPE vos_rs_sip_router_udp_errors_total counter\nvos_rs_sip_router_udp_errors_total {}\n",
            "# TYPE vos_rs_sip_router_tcp_active gauge\nvos_rs_sip_router_tcp_active {}\n",
            "# TYPE vos_rs_sip_router_tcp_rejected_total counter\nvos_rs_sip_router_tcp_rejected_total {}\n",
            "# TYPE vos_rs_sip_router_tcp_frames_total counter\nvos_rs_sip_router_tcp_frames_total {}\n",
            "# TYPE vos_rs_sip_router_security_rejected_total counter\nvos_rs_sip_router_security_rejected_total {}\n",
            "# TYPE vos_rs_sip_router_discovered_nodes gauge\nvos_rs_sip_router_discovered_nodes {}\n",
            "# TYPE vos_rs_sip_router_active_transactions gauge\nvos_rs_sip_router_active_transactions {}\n",
            "# TYPE vos_rs_sip_router_redis_errors_total counter\nvos_rs_sip_router_redis_errors_total {}\n"
        ),
        UDP_RECEIVED.load(Ordering::Relaxed), UDP_ROUTED.load(Ordering::Relaxed),
        UDP_DROPPED.load(Ordering::Relaxed), UDP_ERRORS.load(Ordering::Relaxed),
        TCP_ACTIVE.load(Ordering::Relaxed), TCP_REJECTED.load(Ordering::Relaxed),
        TCP_FRAMES.load(Ordering::Relaxed), SECURITY_REJECTED.load(Ordering::Relaxed),
        DISCOVERED_NODES.load(Ordering::Relaxed),
        ACTIVE_TRANSACTIONS.load(Ordering::Relaxed), REDIS_ERRORS.load(Ordering::Relaxed),
    )
}
