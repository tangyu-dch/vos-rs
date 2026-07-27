use super::health_types::CircuitState;
use std::time::Instant;

/// HalfOpen 状态下允许通过的用户流量采样率（10%）。
pub(crate) const HALF_OPEN_SAMPLE_RATE: f64 = 0.10;
/// HalfOpen 状态下恢复到 Closed 所需的连续成功次数。
pub(crate) const HALF_OPEN_SUCCESS_THRESHOLD: u32 = 5;

/// 单个网关的健康状态，用于 Circuit Breaker 决策。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GatewayHealth {
    pub(crate) success_count: u64,
    pub(crate) failure_count: u64,
    pub(crate) consecutive_failures: u32,
    #[serde(skip, default)]
    pub(crate) last_success: Option<Instant>,
    #[serde(skip, default)]
    pub(crate) last_failure: Option<Instant>,
    pub(crate) active_calls: u32,
    pub(crate) state: CircuitState,
    pub(crate) half_open_successes: u32,
    pub(crate) half_open_probe_in_flight: bool,
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

    pub fn record_probe_success(&mut self) {
        self.success_count += 1;
        self.last_success = Some(Instant::now());
        self.half_open_probe_in_flight = false;
        self.consecutive_failures = 0;
        self.half_open_successes = 0;
        if self.state == CircuitState::HalfOpen || self.state == CircuitState::Open {
            self.state = CircuitState::Closed;
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
