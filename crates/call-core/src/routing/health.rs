use super::types::{CircuitState, HealthThresholds};
use std::collections::HashMap;
use std::time::{Instant, SystemTime};

/// HalfOpen 状态下允许通过的用户流量采样率（10%）。
const HALF_OPEN_SAMPLE_RATE: f64 = 0.10;
/// HalfOpen 状态下恢复到 Closed 所需的连续成功次数。
const HALF_OPEN_SUCCESS_THRESHOLD: u32 = 5;

/// 单个网关的健康状态，用于 Circuit Breaker 决策。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GatewayHealth {
    success_count: u64,
    failure_count: u64,
    consecutive_failures: u32,
    #[serde(skip, default)]
    last_success: Option<Instant>,
    #[serde(skip, default)]
    last_failure: Option<Instant>,
    active_calls: u32,
    state: CircuitState,
    half_open_successes: u32,
    half_open_probe_in_flight: bool,
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
            state: CircuitState::Closed,
            half_open_successes: 0,
            half_open_probe_in_flight: false,
        }
    }

    pub fn record_success(&mut self) {
        self.success_count += 1;
        self.last_success = Some(Instant::now());
        match self.state {
            CircuitState::HalfOpen => {
                self.half_open_probe_in_flight = false;
                self.half_open_successes += 1;
                if self.half_open_successes >= HALF_OPEN_SUCCESS_THRESHOLD {
                    self.state = CircuitState::Closed;
                    self.consecutive_failures = 0;
                    self.half_open_successes = 0;
                }
            }
            CircuitState::Closed => {
                self.consecutive_failures = 0;
            }
            CircuitState::Open => {}
        }
    }

    pub fn record_failure(&mut self) {
        self.failure_count += 1;
        self.consecutive_failures += 1;
        self.last_failure = Some(Instant::now());
        self.half_open_probe_in_flight = false;
        self.half_open_successes = 0;
        if self.state == CircuitState::HalfOpen {
            self.state = CircuitState::Open;
        }
    }

    pub fn increment_active(&mut self) {
        self.active_calls += 1;
    }

    pub fn decrement_active(&mut self) {
        if self.active_calls > 0 {
            self.active_calls -= 1;
        }
    }

    pub fn active_calls(&self) -> u32 {
        self.active_calls
    }

    pub fn success_rate(&self) -> f64 {
        let total = self.success_count + self.failure_count;
        if total == 0 {
            1.0
        } else {
            self.success_count as f64 / total as f64
        }
    }

    pub fn is_circuit_open(&self) -> bool {
        self.state == CircuitState::Open
    }

    pub fn state(&self) -> CircuitState {
        self.state
    }
}

/// 网关健康追踪器：管理所有网关的 Circuit Breaker 状态。
#[derive(Debug, Clone)]
pub struct GatewayHealthTracker {
    states: dashmap::DashMap<String, GatewayHealth>,
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
            states: dashmap::DashMap::new(),
            thresholds,
        }
    }

    pub fn restore_state(
        &self,
        gateway_id: &str,
        circuit_open: bool,
        consecutive_failures: i32,
        last_failure_at: Option<SystemTime>,
        half_open_successes: i32,
        active_calls: i32,
    ) {
        let mut health = self.states.entry(gateway_id.to_string()).or_default();
        health.state = if circuit_open {
            CircuitState::Open
        } else {
            CircuitState::Closed
        };
        health.consecutive_failures = consecutive_failures.max(0) as u32;
        health.half_open_successes = half_open_successes.max(0) as u32;
        health.active_calls = active_calls.max(0) as u32;
        health.half_open_probe_in_flight = false;
        if let Some(sys_time) = last_failure_at {
            let now_sys = SystemTime::now();
            let elapsed = now_sys.duration_since(sys_time).unwrap_or_default();
            health.last_failure = Some(Instant::now() - elapsed);
        }
    }

    pub fn record_success(&self, gateway_id: &str) {
        self.states
            .entry(gateway_id.to_string())
            .or_default()
            .record_success();
    }

    pub fn record_failure(&self, gateway_id: &str) {
        let mut health = self.states.entry(gateway_id.to_string()).or_default();
        health.record_failure();
        if health.consecutive_failures >= self.thresholds.failure_threshold {
            health.state = CircuitState::Open;
        }
    }

    pub fn increment_active(&self, gateway_id: &str) {
        self.states
            .entry(gateway_id.to_string())
            .or_default()
            .increment_active();
    }

    pub fn decrement_active(&self, gateway_id: &str) {
        if let Some(mut health) = self.states.get_mut(gateway_id) {
            health.decrement_active();
        }
    }

    pub fn get_gateway_status(
        &self,
        gateway_id: &str,
    ) -> Option<(bool, i32, String, Option<SystemTime>, i32, i32)> {
        self.states.get(gateway_id).map(|h| {
            let state_str = match h.state {
                CircuitState::Closed => "closed",
                CircuitState::Open => "open",
                CircuitState::HalfOpen => "half_open",
            };
            let last_failure_sys = h.last_failure.map(|inst| {
                let elapsed = inst.elapsed();
                SystemTime::now()
                    .checked_sub(elapsed)
                    .unwrap_or_else(SystemTime::now)
            });
            (
                h.state == CircuitState::Open,
                h.consecutive_failures as i32,
                state_str.to_string(),
                last_failure_sys,
                h.half_open_successes as i32,
                h.active_calls as i32,
            )
        })
    }

    pub fn is_available(&self, gateway_id: &str) -> bool {
        let Some(health) = self.states.get(gateway_id) else {
            return true;
        };

        if health.state == CircuitState::Open {
            if let Some(last_fail) = health.last_failure {
                if last_fail.elapsed() >= self.thresholds.recovery_interval {
                    return true;
                }
            }
            return false;
        }

        let total = health.success_count + health.failure_count;
        if total >= self.thresholds.min_samples
            && health.success_rate() < self.thresholds.min_success_rate
        {
            return false;
        }

        true
    }

    pub fn try_acquire(&self, gateway_id: &str) -> bool {
        use rand::Rng;

        self.try_acquire_with_sample(gateway_id, rand::thread_rng().gen::<f64>())
    }

    pub fn try_acquire_probe(&self, gateway_id: &str) -> bool {
        let mut health = self.states.entry(gateway_id.to_string()).or_default();

        if health.state == CircuitState::Open {
            let recovered = health.last_failure.is_some_and(|last_failure| {
                last_failure.elapsed() >= self.thresholds.recovery_interval
            });
            if !recovered {
                return false;
            }
            health.state = CircuitState::HalfOpen;
            health.half_open_successes = 0;
            health.half_open_probe_in_flight = false;
        }

        if health.state == CircuitState::HalfOpen {
            if health.half_open_probe_in_flight {
                return false;
            }
            health.half_open_probe_in_flight = true;
        }
        true
    }

    pub fn try_acquire_with_sample(&self, gateway_id: &str, sample: f64) -> bool {
        let mut health = self.states.entry(gateway_id.to_string()).or_default();

        if health.state == CircuitState::Open {
            let recovered = health.last_failure.is_some_and(|last_failure| {
                last_failure.elapsed() >= self.thresholds.recovery_interval
            });
            if !recovered {
                return false;
            }
            health.state = CircuitState::HalfOpen;
            health.half_open_successes = 0;
            health.half_open_probe_in_flight = false;
        }

        if health.state == CircuitState::HalfOpen {
            if health.half_open_probe_in_flight || sample >= HALF_OPEN_SAMPLE_RATE {
                return false;
            }
            health.half_open_probe_in_flight = true;
            return true;
        }

        let total = health.success_count + health.failure_count;
        total < self.thresholds.min_samples
            || health.success_rate() >= self.thresholds.min_success_rate
    }

    pub fn circuit_state(&self, gateway_id: &str) -> Option<CircuitState> {
        self.states.get(gateway_id).map(|h| h.value().state())
    }

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

    pub fn health(&self, gateway_id: &str) -> Option<GatewayHealth> {
        self.states.get(gateway_id).map(|h| h.value().clone())
    }

    pub fn all_health(&self) -> HashMap<String, GatewayHealth> {
        self.states
            .iter()
            .map(|r| (r.key().clone(), r.value().clone()))
            .collect()
    }

    pub fn release_acquire(&self, gateway_id: &str) {
        if let Some(mut health) = self.states.get_mut(gateway_id) {
            health.half_open_probe_in_flight = false;
        }
    }
}
