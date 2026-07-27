#[cfg(test)]
mod tests {
    use crate::routing::{
        CircuitState, GatewayHealthTracker, HealthThresholds, Route, RouteTable, RouteTarget,
    };
    use sip_core::SipUri;
    use std::time::Duration;

    fn make_target(host: &str) -> RouteTarget {
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
    fn test_gateway_health_circuit_breaker() {
        let tracker = GatewayHealthTracker::new(HealthThresholds {
            failure_threshold: 3,
            recovery_interval: Duration::from_millis(1),
            min_success_rate: 0.0,
            min_samples: 100,
        });

        assert!(tracker.is_available("gw1"));

        tracker.record_failure("gw1");
        tracker.record_failure("gw1");
        assert!(tracker.is_available("gw1"));

        tracker.record_failure("gw1");
        assert!(!tracker.is_available("gw1"));

        std::thread::sleep(Duration::from_millis(2));
        assert!(tracker.is_available("gw1"));
        assert_eq!(tracker.circuit_state("gw1"), Some(CircuitState::Open));

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

        assert!(tracker.has_capacity("gw1", None));

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
    fn test_restore_state_with_last_failure() {
        let tracker = GatewayHealthTracker::default();
        let now = std::time::SystemTime::now();
        tracker.restore_state("gw1", true, 3, Some(now), 0, 1);

        assert_eq!(tracker.circuit_state("gw1"), Some(CircuitState::Open));
        let status = tracker.get_gateway_status("gw1").unwrap();
        assert!(status.0);
        assert_eq!(status.1, 3);
        assert_eq!(status.5, 1);
    }

    #[test]
    fn test_half_open_failure_reopens_circuit() {
        let tracker = GatewayHealthTracker::new(HealthThresholds {
            failure_threshold: 2,
            recovery_interval: Duration::from_millis(1),
            min_success_rate: 0.0,
            min_samples: 100,
        });

        tracker.record_failure("gw1");
        tracker.record_failure("gw1");
        assert_eq!(tracker.circuit_state("gw1"), Some(CircuitState::Open));

        std::thread::sleep(Duration::from_millis(2));
        assert!(tracker.try_acquire_with_sample("gw1", 0.01));
        assert_eq!(tracker.circuit_state("gw1"), Some(CircuitState::HalfOpen));

        tracker.record_failure("gw1");
        assert_eq!(tracker.circuit_state("gw1"), Some(CircuitState::Open));
    }

    #[test]
    fn test_half_open_probe_in_flight_blocks_acquire() {
        let tracker = GatewayHealthTracker::new(HealthThresholds {
            failure_threshold: 1,
            recovery_interval: Duration::from_millis(1),
            min_success_rate: 0.0,
            min_samples: 100,
        });

        tracker.record_failure("gw1");
        std::thread::sleep(Duration::from_millis(2));

        assert!(tracker.try_acquire_probe("gw1"));
        assert!(!tracker.try_acquire_probe("gw1"));

        tracker.record_probe_success("gw1");
        assert_eq!(tracker.circuit_state("gw1"), Some(CircuitState::Closed));
    }

    #[test]
    fn test_release_acquire_resets_probe_flag() {
        let tracker = GatewayHealthTracker::new(HealthThresholds {
            failure_threshold: 1,
            recovery_interval: Duration::from_millis(1),
            min_success_rate: 0.0,
            min_samples: 100,
        });

        tracker.record_failure("gw1");
        std::thread::sleep(Duration::from_millis(2));

        assert!(tracker.try_acquire_probe("gw1"));
        assert!(!tracker.try_acquire_probe("gw1"));

        tracker.release_acquire("gw1");
        assert!(tracker.try_acquire_probe("gw1"));
    }

    #[test]
    fn test_success_rate_filter() {
        let tracker = GatewayHealthTracker::new(HealthThresholds {
            failure_threshold: 100,
            recovery_interval: Duration::from_secs(60),
            min_success_rate: 0.5,
            min_samples: 10,
        });

        for _ in 0..3 {
            tracker.record_success("gw1");
        }
        for _ in 0..7 {
            tracker.record_failure("gw1");
        }

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
        assert!(status.0);
        assert_eq!(status.1, 3);
        assert_eq!(status.2, "open");
        assert!(status.3.is_some());
        assert_eq!(status.4, 0);
    }

    #[test]
    fn test_options_probe_success_resets_half_open_to_closed_immediately() {
        let tracker = GatewayHealthTracker::new(HealthThresholds {
            failure_threshold: 2,
            recovery_interval: Duration::from_millis(1),
            min_success_rate: 0.0,
            min_samples: 100,
        });

        tracker.record_failure("gw1");
        tracker.record_failure("gw1");
        assert_eq!(tracker.circuit_state("gw1"), Some(CircuitState::Open));

        std::thread::sleep(Duration::from_millis(2));
        assert!(tracker.try_acquire_probe("gw1"));
        assert_eq!(tracker.circuit_state("gw1"), Some(CircuitState::HalfOpen));

        tracker.record_probe_success("gw1");
        assert_eq!(tracker.circuit_state("gw1"), Some(CircuitState::Closed));
        let status = tracker.get_gateway_status("gw1").unwrap();
        assert!(!status.0);
        assert_eq!(status.1, 0);
        assert_eq!(status.2, "closed");
    }
}
