use crate::{CallError, CallResult};
use sip_core::SipUri;
use std::collections::HashMap;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GatewayId(String);

impl GatewayId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteTarget {
    pub gateway_id: GatewayId,
    pub host: String,
    pub port: Option<u16>,
    pub transport: Option<String>,
    /// Maximum concurrent calls allowed through this gateway.
    /// `None` means unlimited.
    pub max_capacity: Option<u32>,
    /// Caller ID rewrite mode: "passthrough", "virtual", or "random".
    pub caller_id_mode: Option<String>,
    /// Fixed virtual caller number when caller_id_mode is "virtual".
    pub virtual_caller: Option<String>,
    /// Prefix transformation rules: "abc:def" (replace), ":def" (add), "abc:" (strip).
    pub prefix_rules: Option<String>,
    /// Direction filter: "inbound", "outbound", "both", or None (no filter).
    pub direction: Option<String>,
    /// Maximum concurrent calls for this gateway's assigned numbers.
    /// `None` means unlimited.
    pub max_concurrent: Option<u32>,
    /// Current concurrent calls (for real-time limit checking).
    pub current_concurrent: u32,
}

impl RouteTarget {
    pub fn new(gateway_id: impl Into<String>, host: impl Into<String>, port: Option<u16>) -> Self {
        Self {
            gateway_id: GatewayId::new(gateway_id),
            host: host.into(),
            port,
            transport: Some("udp".to_string()),
            max_capacity: None,
            caller_id_mode: None,
            virtual_caller: None,
            prefix_rules: None,
            direction: None,
            max_concurrent: None,
            current_concurrent: 0,
        }
    }

    pub fn with_capacity(
        gateway_id: impl Into<String>,
        host: impl Into<String>,
        port: Option<u16>,
        max_capacity: u32,
    ) -> Self {
        Self {
            gateway_id: GatewayId::new(gateway_id),
            host: host.into(),
            port,
            transport: Some("udp".to_string()),
            max_capacity: Some(max_capacity),
            caller_id_mode: None,
            virtual_caller: None,
            prefix_rules: None,
            direction: None,
            max_concurrent: None,
            current_concurrent: 0,
        }
    }

    /// Check if this gateway has capacity for a call in the given direction.
    pub fn has_capacity(&self, call_direction: &str) -> bool {
        // 方向过滤
        if let Some(ref dir) = self.direction {
            if dir != "both" && dir != call_direction {
                return false;
            }
        }
        // 并发限制检查
        if let Some(max) = self.max_concurrent {
            if self.current_concurrent >= max {
                return false;
            }
        }
        // 网关级容量检查
        if let Some(max) = self.max_capacity {
            if self.current_concurrent >= max {
                return false;
            }
        }
        true
    }

    /// Apply prefix transformation rules to a destination number.
    /// Rules format: "abc:def" (replace), ":def" (add prefix), "abc:" (strip prefix).
    /// Multiple rules separated by commas. Rules are applied in order.
    pub fn apply_prefix_rules(&self, number: &str) -> String {
        let rules = match &self.prefix_rules {
            Some(r) if !r.is_empty() => r,
            _ => return number.to_string(),
        };
        let mut result = number.to_string();
        for rule in rules.split(',') {
            let rule = rule.trim();
            if rule.is_empty() {
                continue;
            }
            if let Some(colon_pos) = rule.find(':') {
                let prefix = &rule[..colon_pos];
                let replacement = &rule[colon_pos + 1..];
                if prefix.is_empty() {
                    // :def — 添加前缀
                    result = format!("{replacement}{result}");
                } else if replacement.is_empty() {
                    // abc: — 剥离前缀
                    if result.starts_with(prefix) {
                        result = result[prefix.len()..].to_string();
                    }
                } else {
                    // abc:def — 替换前缀
                    if result.starts_with(prefix) {
                        result = format!("{replacement}{}", &result[prefix.len()..]);
                    }
                }
            }
        }
        result
    }

    pub fn outbound_uri_for(&self, inbound_uri: &SipUri) -> CallResult<SipUri> {
        let user = inbound_uri
            .user
            .clone()
            .ok_or(CallError::InvalidDestinationUri)?;

        let user = self.apply_prefix_rules(&user);

        let mut params = Vec::new();
        if let Some(transport) = &self.transport {
            params.push(("transport".to_string(), Some(transport.clone())));
        }

        Ok(SipUri {
            secure: inbound_uri.secure,
            user: Some(user),
            host: self.host.clone(),
            port: self.port,
            params,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Route {
    pub id: String,
    pub prefix: String,
    pub priority: u16,
    /// Per-call cost for Lowest Cost Routing.
    /// Lower cost is preferred when prefix length and priority are equal.
    /// Defaults to `0.0` (no cost).
    pub cost: f64,
    pub target: RouteTarget,
}

impl Route {
    pub fn new(
        id: impl Into<String>,
        prefix: impl Into<String>,
        priority: u16,
        target: RouteTarget,
    ) -> Self {
        Self {
            id: id.into(),
            prefix: prefix.into(),
            priority,
            cost: 0.0,
            target,
        }
    }

    pub fn with_cost(
        id: impl Into<String>,
        prefix: impl Into<String>,
        priority: u16,
        cost: f64,
        target: RouteTarget,
    ) -> Self {
        Self {
            id: id.into(),
            prefix: prefix.into(),
            priority,
            cost,
            target,
        }
    }

    fn matches(&self, destination: &str) -> bool {
        destination.starts_with(&self.prefix)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedRoute {
    pub route_id: String,
    pub target: RouteTarget,
    pub outbound_uri: SipUri,
}

// ---------------------------------------------------------------------------
// Gateway health tracking
// ---------------------------------------------------------------------------

/// Health state for a single gateway, used for circuit-breaking.
#[derive(Debug, Clone)]
pub struct GatewayHealth {
    /// Total successful call attempts (2xx response).
    success_count: u64,
    /// Total failed call attempts (4xx/5xx/timeout).
    failure_count: u64,
    /// Consecutive failures since the last success.
    consecutive_failures: u32,
    /// When the gateway was last marked healthy.
    last_success: Option<Instant>,
    /// When the gateway was last marked unhealthy (circuit open).
    last_failure: Option<Instant>,
    /// Current active call count through this gateway.
    active_calls: u32,
    /// Circuit breaker state.
    circuit_open: bool,
}

impl Default for GatewayHealth {
    fn default() -> Self {
        Self::new()
    }
}

impl GatewayHealth {
    pub fn new() -> Self {
        Self {
            success_count: 0,
            failure_count: 0,
            consecutive_failures: 0,
            last_success: None,
            last_failure: None,
            active_calls: 0,
            circuit_open: false,
        }
    }

    /// Record a successful call through this gateway.
    pub fn record_success(&mut self) {
        self.success_count += 1;
        self.consecutive_failures = 0;
        self.last_success = Some(Instant::now());
        self.circuit_open = false;
    }

    /// Record a failed call through this gateway.
    pub fn record_failure(&mut self) {
        self.failure_count += 1;
        self.consecutive_failures += 1;
        self.last_failure = Some(Instant::now());
    }

    /// Increment active call counter.
    pub fn increment_active(&mut self) {
        self.active_calls += 1;
    }

    /// Decrement active call counter.
    pub fn decrement_active(&mut self) {
        if self.active_calls > 0 {
            self.active_calls -= 1;
        }
    }

    /// Active calls currently routed through this gateway.
    pub fn active_calls(&self) -> u32 {
        self.active_calls
    }

    /// Success rate as a fraction in [0.0, 1.0].
    pub fn success_rate(&self) -> f64 {
        let total = self.success_count + self.failure_count;
        if total == 0 {
            1.0
        } else {
            self.success_count as f64 / total as f64
        }
    }

    /// Whether the circuit breaker is currently tripped (open).
    pub fn is_circuit_open(&self) -> bool {
        self.circuit_open
    }
}

/// Thresholds that control the gateway circuit breaker.
#[derive(Debug, Clone)]
pub struct HealthThresholds {
    /// Open the circuit after this many consecutive failures.
    pub failure_threshold: u32,
    /// Half-open probe interval: after this duration, allow one trial call.
    pub recovery_interval: Duration,
    /// Minimum success rate below which a gateway is considered unhealthy.
    pub min_success_rate: f64,
    /// Minimum number of samples before success rate is evaluated.
    pub min_samples: u64,
}

impl Default for HealthThresholds {
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            recovery_interval: Duration::from_secs(30),
            min_success_rate: 0.3,
            min_samples: 10,
        }
    }
}

/// Tracks health state for all known gateways.
#[derive(Debug, Clone)]
pub struct GatewayHealthTracker {
    states: HashMap<String, GatewayHealth>,
    thresholds: HealthThresholds,
}

impl Default for GatewayHealthTracker {
    fn default() -> Self {
        Self::new(HealthThresholds::default())
    }
}

impl GatewayHealthTracker {
    pub fn new(thresholds: HealthThresholds) -> Self {
        Self {
            states: HashMap::new(),
            thresholds,
        }
    }

    /// Record a successful call outcome for the given gateway.
    pub fn record_success(&mut self, gateway_id: &str) {
        self.states
            .entry(gateway_id.to_string())
            .or_default()
            .record_success();
    }

    /// Record a failed call outcome for the given gateway.
    /// Opens the circuit if consecutive failures exceed the threshold.
    pub fn record_failure(&mut self, gateway_id: &str) {
        let health = self.states.entry(gateway_id.to_string()).or_default();
        health.record_failure();
        if health.consecutive_failures >= self.thresholds.failure_threshold {
            health.circuit_open = true;
        }
    }

    /// Increment the active call count for a gateway.
    pub fn increment_active(&mut self, gateway_id: &str) {
        self.states
            .entry(gateway_id.to_string())
            .or_default()
            .increment_active();
    }

    /// Decrement the active call count for a gateway.
    pub fn decrement_active(&mut self, gateway_id: &str) {
        if let Some(health) = self.states.get_mut(gateway_id) {
            health.decrement_active();
        }
    }

    /// Check whether the gateway should be considered available for new calls.
    /// Returns `false` if the circuit is open and the recovery interval has not elapsed.
    pub fn is_available(&self, gateway_id: &str) -> bool {
        let Some(health) = self.states.get(gateway_id) else {
            // No health data yet — allow the call.
            return true;
        };

        if health.circuit_open {
            // Half-open: allow one trial call after the recovery interval.
            if let Some(last_fail) = health.last_failure {
                if last_fail.elapsed() >= self.thresholds.recovery_interval {
                    return true;
                }
            }
            return false;
        }

        // Check success rate if we have enough samples.
        let total = health.success_count + health.failure_count;
        if total >= self.thresholds.min_samples
            && health.success_rate() < self.thresholds.min_success_rate
        {
            return false;
        }

        true
    }

    /// Check whether the gateway has reached its capacity limit.
    pub fn has_capacity(&self, gateway_id: &str, max_capacity: Option<u32>) -> bool {
        match max_capacity {
            None => true,
            Some(cap) => {
                let active = self
                    .states
                    .get(gateway_id)
                    .map(|h| h.active_calls())
                    .unwrap_or(0);
                active < cap
            }
        }
    }

    /// Get a snapshot of the health for a specific gateway.
    pub fn health(&self, gateway_id: &str) -> Option<&GatewayHealth> {
        self.states.get(gateway_id)
    }

    /// Get snapshots of all gateway health states.
    pub fn all_health(&self) -> &HashMap<String, GatewayHealth> {
        &self.states
    }
}

// ---------------------------------------------------------------------------
// Route table
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RouteTable {
    routes: Vec<Route>,
}

impl RouteTable {
    pub fn new(routes: Vec<Route>) -> Self {
        Self { routes }
    }

    pub fn clear(&mut self) {
        self.routes.clear();
    }

    pub fn add_route(&mut self, route: Route) {
        self.routes.push(route);
    }

    pub fn select(&self, destination_uri: &SipUri) -> CallResult<SelectedRoute> {
        let candidates = self.select_candidates(destination_uri)?;
        Ok(candidates.first().cloned().unwrap())
    }

    /// Select candidate routes ordered by:
    /// 1. Longest prefix match (most specific first)
    /// 2. Higher priority value (higher = more preferred)
    /// 3. Lower cost (LCR — Lowest Cost Routing)
    pub fn select_candidates(&self, destination_uri: &SipUri) -> CallResult<Vec<SelectedRoute>> {
        let destination = destination_uri
            .user
            .as_deref()
            .ok_or(CallError::InvalidDestinationUri)?;

        let mut matching_routes: Vec<&Route> = self
            .routes
            .iter()
            .filter(|route| route.matches(destination))
            .collect();

        if matching_routes.is_empty() {
            return Err(CallError::NoRouteForDestination(destination.to_string()));
        }

        matching_routes.sort_by(|left, right| {
            right
                .prefix
                .len()
                .cmp(&left.prefix.len())
                .then_with(|| right.priority.cmp(&left.priority))
                .then_with(|| left.cost.partial_cmp(&right.cost).unwrap_or(std::cmp::Ordering::Equal))
        });

        let mut candidates = Vec::with_capacity(matching_routes.len());
        for route in matching_routes {
            candidates.push(SelectedRoute {
                route_id: route.id.clone(),
                target: route.target.clone(),
                outbound_uri: route.target.outbound_uri_for(destination_uri)?,
            });
        }

        Ok(candidates)
    }

    /// Select candidate routes for a specific call direction, filtering out
    /// gateways that don't support the direction or are at capacity.
    pub fn select_candidates_for_direction(
        &self,
        destination_uri: &SipUri,
        call_direction: &str,
    ) -> CallResult<Vec<SelectedRoute>> {
        let destination = destination_uri
            .user
            .as_deref()
            .ok_or(CallError::InvalidDestinationUri)?;

        let mut matching_routes: Vec<&Route> = self
            .routes
            .iter()
            .filter(|route| route.matches(destination))
            .filter(|route| route.target.has_capacity(call_direction))
            .collect();

        if matching_routes.is_empty() {
            return Err(CallError::NoRouteForDestination(destination.to_string()));
        }

        matching_routes.sort_by(|left, right| {
            right
                .prefix
                .len()
                .cmp(&left.prefix.len())
                .then_with(|| right.priority.cmp(&left.priority))
                .then_with(|| left.cost.partial_cmp(&right.cost).unwrap_or(std::cmp::Ordering::Equal))
        });

        let mut candidates = Vec::with_capacity(matching_routes.len());
        for route in matching_routes {
            candidates.push(SelectedRoute {
                route_id: route.id.clone(),
                target: route.target.clone(),
                outbound_uri: route.target.outbound_uri_for(destination_uri)?,
            });
        }

        Ok(candidates)
    }

    /// Select candidate routes, filtering out gateways that are unhealthy
    /// (circuit breaker open) or at capacity. Falls back to all candidates
    /// if every matching gateway is unhealthy (to avoid total outage).
    pub fn select_healthy_candidates(
        &self,
        destination_uri: &SipUri,
        health: &GatewayHealthTracker,
        call_direction: Option<&str>,
    ) -> CallResult<Vec<SelectedRoute>> {
        let all_candidates = if let Some(dir) = call_direction {
            self.select_candidates_for_direction(destination_uri, dir)?
        } else {
            self.select_candidates(destination_uri)?
        };

        let healthy: Vec<SelectedRoute> = all_candidates
            .iter()
            .filter(|c| {
                let gid = c.target.gateway_id.as_str();
                health.is_available(gid) && health.has_capacity(gid, c.target.max_capacity)
            })
            .cloned()
            .collect();

        if healthy.is_empty() {
            // All gateways are unhealthy — fall back to all candidates to avoid
            // a total service outage. The caller can still attempt the call.
            warn_all_gateways_unhealthy(&all_candidates);
            Ok(all_candidates)
        } else {
            Ok(healthy)
        }
    }

    pub fn routes(&self) -> &[Route] {
        &self.routes
    }

    pub fn is_empty(&self) -> bool {
        self.routes.is_empty()
    }
}

fn warn_all_gateways_unhealthy(candidates: &[SelectedRoute]) {
    // This function exists so the warning can be logged by the caller;
    // here we simply provide a no-op placeholder that the sip-edge layer
    // can replace with actual tracing calls.
    let _ = candidates;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_target(host: &str) -> RouteTarget {
        // Derive a stable gateway_id from the host so each test target is a distinct gateway.
        let gateway_id = host.split('.').next().unwrap_or("gw");
        RouteTarget::new(gateway_id, host, Some(5060))
    }

    fn make_uri(user: &str) -> SipUri {
        SipUri {
            secure: false,
            user: Some(user.to_string()),
            host: "example.com".to_string(),
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
            Route::with_cost(
                "r1",
                "86",
                100,
                0.50,
                make_target("gw1.example.com"),
            ),
            Route::with_cost(
                "r2",
                "86",
                100,
                0.30,
                make_target("gw2.example.com"),
            ),
            Route::with_cost(
                "r3",
                "86",
                100,
                0.40,
                make_target("gw3.example.com"),
            ),
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
        let mut tracker = GatewayHealthTracker::new(HealthThresholds {
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

        // After recovery interval, half-open allows a trial
        std::thread::sleep(Duration::from_millis(2));
        assert!(tracker.is_available("gw1"));

        // Success closes the circuit
        tracker.record_success("gw1");
        assert!(tracker.is_available("gw1"));
    }

    #[test]
    fn test_capacity_control() {
        let mut tracker = GatewayHealthTracker::default();

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
        let mut tracker = GatewayHealthTracker::new(HealthThresholds {
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
    fn test_select_healthy_candidates_fallback_when_all_unhealthy() {
        let routes = vec![Route::new(
            "r1",
            "86",
            100,
            make_target("gw1.example.com"),
        )];
        let table = RouteTable::new(routes);
        let mut tracker = GatewayHealthTracker::new(HealthThresholds {
            failure_threshold: 1,
            recovery_interval: Duration::from_secs(60),
            min_success_rate: 0.0,
            min_samples: 100,
        });

        tracker.record_failure("gw1");

        // All unhealthy → fallback to all candidates
        let candidates = table
            .select_healthy_candidates(&make_uri("8613800138000"), &tracker, None)
            .unwrap();
        assert_eq!(candidates.len(), 1);
    }
}
