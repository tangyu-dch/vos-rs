use super::*;
use sip_core::SipUri;
use std::time::{Duration, SystemTime};

fn make_target(host: &str) -> RouteTarget {
    // Derive a stable gateway_id from the host so each test target is a distinct gateway.
    let gateway_id = host.split('.').next().unwrap_or("gw");
    RouteTarget::new(gateway_id, host, Some(5060))
}

fn make_uri(user: &str) -> SipUri {
    SipUri {
        secure: false,
        user: Some(user.to_string().into()),
        host: "example.com".to_string().into(),
        port: None,
        params: vec![],
    }
}

#[test]
fn test_prefix_match_and_priority_sort() {
    let routes = vec![
        Route::new("r1", "86", 100, make_target("gw1.example.com")),
        Route::new("r2", "8613", 200, make_target("gw2.example.com")),
        Route::new("r3", "8613", 100, make_target("gw3.example.com")),
    ];
    let table = RouteTable::new(routes);
    let candidates = table.select_candidates(&make_uri("8613800138000")).unwrap();

    // Longest prefix first (8613), then higher priority
    assert_eq!(candidates[0].target.host, "gw2.example.com");
    assert_eq!(candidates[1].target.host, "gw3.example.com");
    assert_eq!(candidates[2].target.host, "gw1.example.com");
}

#[test]
fn test_lcr_cost_sort() {
    let routes = vec![
        Route::with_cost("r1", "86", 100, 0.50, make_target("gw1.example.com")),
        Route::with_cost("r2", "86", 100, 0.30, make_target("gw2.example.com")),
        Route::with_cost("r3", "86", 100, 0.40, make_target("gw3.example.com")),
    ];
    let table = RouteTable::new(routes);
    let candidates = table.select_candidates(&make_uri("8613800138000")).unwrap();

    // Same prefix, same priority → sorted by cost ascending (LCR)
    assert_eq!(candidates[0].target.host, "gw2.example.com"); // cost 0.30
    assert_eq!(candidates[1].target.host, "gw3.example.com"); // cost 0.40
    assert_eq!(candidates[2].target.host, "gw1.example.com"); // cost 0.50
}

#[test]
fn test_gateway_health_circuit_breaker() {
    let tracker = GatewayHealthTracker::new(HealthThresholds {
        failure_threshold: 3,
        recovery_interval: Duration::from_millis(1),
        min_success_rate: 0.0,
        min_samples: 100,
    });

    // Initially available (no health data)
    assert!(tracker.is_available("gw1"));

    // Record failures
    tracker.record_failure("gw1");
    tracker.record_failure("gw1");
    assert!(tracker.is_available("gw1")); // 2 < 3 threshold

    tracker.record_failure("gw1");
    assert!(!tracker.is_available("gw1")); // circuit open

    // After recovery interval, the circuit enters half-open.
    std::thread::sleep(Duration::from_millis(2));
    assert!(tracker.is_available("gw1"));
    assert_eq!(tracker.circuit_state("gw1"), Some(CircuitState::Open));

    // A sampled request enters half-open, and five consecutive successes close it.
    assert!(tracker.try_acquire_with_sample("gw1", 0.01));
    assert_eq!(tracker.circuit_state("gw1"), Some(CircuitState::HalfOpen));
    for _ in 0..4 {
        tracker.record_success("gw1");
        assert!(tracker.try_acquire_with_sample("gw1", 0.01));
    }
    tracker.record_success("gw1");
    assert_eq!(tracker.circuit_state("gw1"), Some(CircuitState::Closed));
}

#[test]
fn test_capacity_control() {
    let tracker = GatewayHealthTracker::default();

    // Unlimited capacity
    assert!(tracker.has_capacity("gw1", None));

    // 2 max capacity
    tracker.increment_active("gw1");
    tracker.increment_active("gw1");
    assert!(!tracker.has_capacity("gw1", Some(2)));
    assert!(tracker.has_capacity("gw1", Some(3)));

    tracker.decrement_active("gw1");
    assert!(tracker.has_capacity("gw1", Some(2)));
}

#[test]
fn test_select_healthy_candidates_filters_unhealthy() {
    let routes = vec![
        Route::new("r1", "86", 100, make_target("gw1.example.com")),
        Route::new("r2", "86", 100, make_target("gw2.example.com")),
    ];
    let table = RouteTable::new(routes);
    let tracker = GatewayHealthTracker::new(HealthThresholds {
        failure_threshold: 1,
        recovery_interval: Duration::from_secs(60),
        min_success_rate: 0.0,
        min_samples: 100,
    });

    // Mark gw1 as unhealthy
    tracker.record_failure("gw1");

    let healthy = table
        .select_healthy_candidates(&make_uri("8613800138000"), &tracker, None)
        .unwrap();
    assert_eq!(healthy.len(), 1);
    assert_eq!(healthy[0].target.host, "gw2.example.com");
}

#[test]
fn test_select_healthy_candidates_rejects_when_all_unhealthy() {
    let routes = vec![Route::new("r1", "86", 100, make_target("gw1.example.com"))];
    let table = RouteTable::new(routes);
    let tracker = GatewayHealthTracker::new(HealthThresholds {
        failure_threshold: 1,
        recovery_interval: Duration::from_secs(60),
        min_success_rate: 0.0,
        min_samples: 100,
    });

    tracker.record_failure("gw1");

    let result = table.select_healthy_candidates(&make_uri("8613800138000"), &tracker, None);
    assert!(result.is_err());
}

#[test]
fn test_weighted_load_balancing() {
    let routes = vec![
        Route::with_cost_and_weight("r1", "86", 100, 0.5, 200, make_target("gw1.example.com")),
        Route::with_cost_and_weight("r2", "86", 100, 0.5, 100, make_target("gw2.example.com")),
        Route::with_cost_and_weight("r3", "86", 100, 0.5, 100, make_target("gw3.example.com")),
    ];
    let table = RouteTable::new(routes);

    let mut gw1_count = 0;
    let mut gw2_count = 0;
    let mut gw3_count = 0;

    for _ in 0..1000 {
        let candidates = table.select_candidates(&make_uri("8613800138000")).unwrap();
        assert_eq!(candidates.len(), 3);
        match candidates[0].target.host.as_str() {
            "gw1.example.com" => gw1_count += 1,
            "gw2.example.com" => gw2_count += 1,
            "gw3.example.com" => gw3_count += 1,
            _ => {}
        }
    }

    println!(
        "WEIGHT_TEST: gw1 = {}, gw2 = {}, gw3 = {}",
        gw1_count, gw2_count, gw3_count
    );
    assert!(gw1_count > gw2_count);
    assert!(gw1_count > gw3_count);
    assert!(gw2_count > 100);
    assert!(gw3_count > 100);
}

#[test]
fn test_half_open_failure_reopens_circuit() {
    let tracker = GatewayHealthTracker::new(HealthThresholds {
        failure_threshold: 2,
        recovery_interval: Duration::from_millis(1),
        min_success_rate: 0.0,
        min_samples: 100,
    });

    // Open the circuit
    tracker.record_failure("gw1");
    tracker.record_failure("gw1");
    assert!(!tracker.is_available("gw1"));

    // Wait for recovery, enter half-open
    std::thread::sleep(Duration::from_millis(2));
    assert!(tracker.try_acquire_with_sample("gw1", 0.01));
    assert_eq!(tracker.circuit_state("gw1"), Some(CircuitState::HalfOpen));

    // Failure in half-open re-opens circuit
    tracker.record_failure("gw1");
    assert_eq!(tracker.circuit_state("gw1"), Some(CircuitState::Open));
    assert!(!tracker.is_available("gw1"));
}

#[test]
fn test_half_open_probe_in_flight_blocks_acquire() {
    let tracker = GatewayHealthTracker::new(HealthThresholds {
        failure_threshold: 1,
        recovery_interval: Duration::from_millis(1),
        min_success_rate: 0.0,
        min_samples: 100,
    });

    // Open circuit
    tracker.record_failure("gw1");
    std::thread::sleep(Duration::from_millis(2));

    // First acquire succeeds (enters half-open, marks probe in flight)
    assert!(tracker.try_acquire_with_sample("gw1", 0.01));
    // Second acquire fails (probe already in flight)
    assert!(!tracker.try_acquire_with_sample("gw1", 0.01));

    // After success, probe is released
    tracker.record_success("gw1");
    assert!(tracker.try_acquire_with_sample("gw1", 0.01));
}

#[test]
fn test_restore_state_with_last_failure() {
    let tracker = GatewayHealthTracker::new(HealthThresholds {
        failure_threshold: 1,
        recovery_interval: Duration::from_millis(10),
        min_success_rate: 0.0,
        min_samples: 100,
    });

    // Simulate a recent failure (1ms ago)
    let last_failure = SystemTime::now() - Duration::from_millis(1);
    tracker.restore_state("gw1", true, 3, Some(last_failure), 0, 0);

    // Circuit is open but recovery interval hasn't elapsed
    assert_eq!(tracker.circuit_state("gw1"), Some(CircuitState::Open));
    assert!(!tracker.is_available("gw1"));

    // After waiting, recovery interval elapsed
    std::thread::sleep(Duration::from_millis(15));
    assert!(tracker.is_available("gw1"));
}

#[test]
fn test_release_acquire_resets_probe_flag() {
    let tracker = GatewayHealthTracker::new(HealthThresholds {
        failure_threshold: 1,
        recovery_interval: Duration::from_millis(1),
        min_success_rate: 0.0,
        min_samples: 100,
    });

    // Open circuit and enter half-open
    tracker.record_failure("gw1");
    std::thread::sleep(Duration::from_millis(2));
    assert!(tracker.try_acquire_with_sample("gw1", 0.01));

    // Probe is in flight, can't acquire again
    assert!(!tracker.try_acquire_with_sample("gw1", 0.01));

    // Release the acquire
    tracker.release_acquire("gw1");

    // Now can acquire again
    assert!(tracker.try_acquire_with_sample("gw1", 0.01));
}

#[test]
fn test_success_rate_filter() {
    let tracker = GatewayHealthTracker::new(HealthThresholds {
        failure_threshold: 100,
        recovery_interval: Duration::from_secs(60),
        min_success_rate: 0.5,
        min_samples: 5,
    });

    // Record enough samples to trigger success rate check
    for _ in 0..3 {
        tracker.record_success("gw1");
    }
    for _ in 0..7 {
        tracker.record_failure("gw1");
    }

    // Success rate is 30% < 50% threshold
    assert!(!tracker.is_available("gw1"));
}

#[test]
fn test_get_gateway_status_full() {
    let tracker = GatewayHealthTracker::new(HealthThresholds {
        failure_threshold: 3,
        recovery_interval: Duration::from_millis(1),
        min_success_rate: 0.0,
        min_samples: 100,
    });

    tracker.record_failure("gw1");
    tracker.record_failure("gw1");
    tracker.record_failure("gw1");

    let status = tracker.get_gateway_status("gw1").unwrap();
    assert!(status.0); // circuit_open
    assert_eq!(status.1, 3); // consecutive_failures
    assert_eq!(status.2, "open"); // state_str
    assert!(status.3.is_some()); // last_failure_at
    assert_eq!(status.4, 0); // half_open_successes
}
