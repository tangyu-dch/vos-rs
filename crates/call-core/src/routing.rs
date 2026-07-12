//! # 路由引擎与网关健康追踪
//!
//! 本模块实现了 VoIP 软交换的核心路由选择逻辑和网关健康熔断器。
//!
//! ## 路由选择流程
//!
//! 1. 从 `sip_routes` 表加载路由规则（含 weight 字段）
//! 2. 按被叫号码最长前缀匹配（prefix length DESC）
//! 3. 同前缀按优先级排序（priority DESC，数字越大越优先）
//! 4. 同优先级按成本升序（cost ASC，LCR 最低成本路由）
//! 5. 同等条件下按权重加权随机（weight DESC/random）
//! 6. 检查时间窗口（time_start/time_end）
//! 7. 检查网关健康状态（Circuit Breaker）
//! 8. 检查并发容量（max_capacity / current_concurrent）
//! 9. 只对最终选中的网关执行 acquire（HalfOpen probe 保护）
//!
//! ## 网关健康熔断器（Circuit Breaker）
//!
//! 状态机：
//! - **Closed**：正常状态，所有呼叫正常路由
//! - **Open**：熔断状态，拒绝所有呼叫，等待恢复间隔
//! - **HalfOpen**：半开状态，允许少量探测呼叫，成功则恢复 Closed
//!
//! 恢复条件：
//! - 连续失败次数 >= `failure_threshold` 时打开电路
//! - `recovery_interval` 后进入 HalfOpen
//! - HalfOpen 下连续 `HALF_OPEN_SUCCESS_THRESHOLD`（5）次成功恢复 Closed
//! - HalfOpen 下任何失败重新打开电路

use crate::{CallError, CallResult};
use sip_core::SipUri;
use std::collections::HashMap;
use std::time::{Duration, Instant, SystemTime};

/// HalfOpen 状态下允许通过的用户流量采样率（10%）。
const HALF_OPEN_SAMPLE_RATE: f64 = 0.10;
/// HalfOpen 状态下恢复到 Closed 所需的连续成功次数。
const HALF_OPEN_SUCCESS_THRESHOLD: u32 = 5;

/// 网关唯一标识符。
///
/// 用于在路由表和健康追踪器中标识不同的网关。
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

/// 路由目标：指向特定网关的路由条目。
///
/// 包含网关地址、容量限制、Caller ID 重写规则等配置。
/// 路由引擎根据 `RouteTarget` 构建出站 INVITE 的目标地址。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteTarget {
    /// 网关唯一标识
    pub gateway_id: GatewayId,
    /// 网关主机地址
    pub host: String,
    /// 网关 SIP 端口
    pub port: Option<u16>,
    /// 传输协议（udp/tcp/tls）
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

/// 路由条目：定义被叫号码到网关的映射规则。
///
/// 每条路由包含前缀匹配规则、优先级、成本和权重。
/// 路由引擎根据这些字段进行排序和选择：
/// - 前缀越长越优先（更精确匹配）
/// - 优先级数字越大越优先
/// - 成本越低越优先（LCR）
/// - 同等条件下按权重加权随机
#[derive(Debug, Clone, PartialEq)]
pub struct Route {
    /// 路由唯一标识
    pub id: String,
    /// 被叫号码前缀（如 "86" 表示中国大陆，"8613" 表示中国移动）
    pub prefix: String,
    /// 优先级（数字越大越优先）
    pub priority: u16,
    /// 每呼叫成本（用于最低成本路由 LCR）
    /// 当前缀长度和优先级相同时，成本越低越优先
    /// 默认为 0.0（无成本）
    pub cost: f64,
    /// 权重（用于同等条件下的加权随机负载均衡）
    /// 权重越高，被选为第一候选的概率越大
    /// 默认为 100
    pub weight: u32,
    /// 路由目标（网关地址和配置）
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
            weight: 100, // 默认权重为 100
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
            weight: 100, // 默认权重为 100
            target,
        }
    }

    pub fn with_cost_and_weight(
        id: impl Into<String>,
        prefix: impl Into<String>,
        priority: u16,
        cost: f64,
        weight: u32,
        target: RouteTarget,
    ) -> Self {
        Self {
            id: id.into(),
            prefix: prefix.into(),
            priority,
            cost,
            weight,
            target,
        }
    }

    fn matches(&self, destination: &str) -> bool {
        destination.starts_with(&self.prefix)
    }
}

/// 选中的路由：路由引擎最终选择的路由条目。
///
/// 包含路由 ID、目标网关和出站 SIP URI。
/// 用于构建出站 INVITE 请求。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedRoute {
    /// 路由 ID
    pub route_id: String,
    /// 目标网关配置
    pub target: RouteTarget,
    /// 出站 SIP URI（已应用前缀规则）
    pub outbound_uri: SipUri,
}

// ---------------------------------------------------------------------------
// Gateway health tracking
// ---------------------------------------------------------------------------

/// 网关健康熔断器状态。
///
/// 状态机转换：
/// - `Closed` → `Open`：连续失败次数 >= `failure_threshold`
/// - `Open` → `HalfOpen`：`recovery_interval` 后允许探测
/// - `HalfOpen` → `Closed`：连续成功次数 >= `HALF_OPEN_SUCCESS_THRESHOLD`
/// - `HalfOpen` → `Open`：任何失败
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    /// 正常状态，所有呼叫正常路由
    Closed,
    /// 熔断状态，拒绝所有呼叫，等待恢复间隔
    Open,
    /// 半开状态，允许少量探测呼叫
    HalfOpen,
}

/// 单个网关的健康状态，用于 Circuit Breaker 决策。
///
/// 跟踪网关的成功/失败计数、连续失败次数、最后成功/失败时间、
/// 当前活跃呼叫数和熔断器状态。
#[derive(Debug, Clone)]
pub struct GatewayHealth {
    /// 总成功呼叫数（2xx 响应）
    success_count: u64,
    /// 总失败呼叫数（4xx/5xx/timeout）
    failure_count: u64,
    /// 自上次成功以来的连续失败次数
    consecutive_failures: u32,
    /// 最后一次成功的时间
    last_success: Option<Instant>,
    /// 最后一次失败的时间（用于判断恢复间隔）
    last_failure: Option<Instant>,
    /// Current active call count through this gateway.
    active_calls: u32,
    /// Circuit breaker state.
    state: CircuitState,
    /// Consecutive successful calls during half-open recovery.
    half_open_successes: u32,
    /// Whether a half-open call is currently in flight.
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

    /// Record a successful call through this gateway.
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

    /// Record a failed call through this gateway.
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
        self.state == CircuitState::Open
    }

    /// Returns the current circuit state.
    pub fn state(&self) -> CircuitState {
        self.state
    }
}

/// 网关健康熔断器阈值配置。
///
/// 控制 Circuit Breaker 的行为参数：
/// - 连续失败多少次后打开电路
/// - 打开后多久进入 HalfOpen 探测
/// - 成功率低于多少视为不健康
#[derive(Debug, Clone)]
pub struct HealthThresholds {
    /// 连续失败次数阈值，超过后打开电路（默认 5）
    pub failure_threshold: u32,
    /// 恢复间隔：电路打开后多久进入 HalfOpen 探测（默认 30 秒）
    pub recovery_interval: Duration,
    /// 最低成功率阈值，低于此值视为不健康（默认 0.3，即 30%）
    pub min_success_rate: f64,
    /// 最少样本数，低于此数不评估成功率（默认 10）
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

impl HealthThresholds {
    /// Create thresholds from environment variables with fallback to defaults.
    pub fn from_env() -> Self {
        Self {
            failure_threshold: std::env::var("VOS_RS_CIRCUIT_BREAKER_FAILURE_THRESHOLD")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(5),
            recovery_interval: Duration::from_secs(
                std::env::var("VOS_RS_CIRCUIT_BREAKER_RECOVERY_SECS")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(30),
            ),
            min_success_rate: std::env::var("VOS_RS_CIRCUIT_BREAKER_MIN_SUCCESS_RATE")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0.3),
            min_samples: std::env::var("VOS_RS_CIRCUIT_BREAKER_MIN_SAMPLES")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(10),
        }
    }
}

/// 网关健康追踪器：管理所有网关的 Circuit Breaker 状态。
///
/// 使用 `HashMap` 存储每个网关的健康状态，在路由选择时：
/// - 过滤掉 Open 状态的网关（恢复间隔未到）
/// - 对 HalfOpen 状态的网关限制探测流量
/// - 对 Closed 状态的网关检查成功率
///
/// 线程安全：由 `EdgeState` 中的 `Mutex<GatewayHealthTracker>` 保护。
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

    pub fn restore_state(
        &mut self,
        gateway_id: &str,
        circuit_open: bool,
        consecutive_failures: i32,
        last_failure_at: Option<std::time::SystemTime>,
        half_open_successes: i32,
        active_calls: i32,
    ) {
        let health = self.states.entry(gateway_id.to_string()).or_default();
        health.state = if circuit_open {
            CircuitState::Open
        } else {
            CircuitState::Closed
        };
        health.consecutive_failures = consecutive_failures.max(0) as u32;
        health.half_open_successes = half_open_successes.max(0) as u32;
        health.active_calls = active_calls.max(0) as u32;
        health.half_open_probe_in_flight = false;
        // Restore last_failure from persisted wall-clock time so that the
        // recovery interval check works correctly after a restart.
        if let Some(sys_time) = last_failure_at {
            let now_sys = SystemTime::now();
            let elapsed = now_sys.duration_since(sys_time).unwrap_or_default();
            health.last_failure = Some(Instant::now() - elapsed);
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
            health.state = CircuitState::Open;
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

    /// Returns (circuit_open, consecutive_failures, state_str, last_failure_system_time, half_open_successes)
    /// for persistence. `last_failure_system_time` is derived from the monotonic `Instant` by
    /// computing how long ago the failure occurred and subtracting from current wall-clock time.
    pub fn get_gateway_status(
        &self,
        gateway_id: &str,
    ) -> Option<(bool, i32, String, Option<std::time::SystemTime>, i32, i32)> {
        self.states.get(gateway_id).map(|h| {
            let state_str = match h.state {
                CircuitState::Closed => "closed",
                CircuitState::Open => "open",
                CircuitState::HalfOpen => "half_open",
            };
            let last_failure_sys = h.last_failure.map(|inst| {
                let elapsed = inst.elapsed();
                std::time::SystemTime::now()
                    .checked_sub(elapsed)
                    .unwrap_or(std::time::SystemTime::now())
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

    /// Check whether the gateway should be considered available for new calls.
    /// Returns `false` if the circuit is open and the recovery interval has not elapsed.
    pub fn is_available(&self, gateway_id: &str) -> bool {
        let Some(health) = self.states.get(gateway_id) else {
            // No health data yet — allow the call.
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

        // Check success rate if we have enough samples.
        let total = health.success_count + health.failure_count;
        if total >= self.thresholds.min_samples
            && health.success_rate() < self.thresholds.min_success_rate
        {
            return false;
        }

        true
    }

    /// Attempts to reserve traffic for a gateway.
    ///
    /// Closed gateways are available subject to the success-rate guard. Open
    /// gateways enter HalfOpen after the recovery interval. HalfOpen gateways
    /// admit approximately 10% of attempts and only one probe at a time.
    pub fn try_acquire(&mut self, gateway_id: &str) -> bool {
        use rand::Rng;

        self.try_acquire_with_sample(gateway_id, rand::thread_rng().gen::<f64>())
    }

    /// Reserves an explicit active health probe.
    ///
    /// Active probes bypass the 10% user-traffic sample, but still respect the
    /// recovery interval and the single in-flight probe rule for HalfOpen.
    pub fn try_acquire_probe(&mut self, gateway_id: &str) -> bool {
        let Some(health) = self.states.get_mut(gateway_id) else {
            return true;
        };

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

    /// Deterministic variant used by integration tests and simulations.
    pub fn try_acquire_with_sample(&mut self, gateway_id: &str, sample: f64) -> bool {
        let Some(health) = self.states.get_mut(gateway_id) else {
            return true;
        };

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

    /// Returns the current state of a gateway, if it has been observed.
    pub fn circuit_state(&self, gateway_id: &str) -> Option<CircuitState> {
        self.states.get(gateway_id).map(GatewayHealth::state)
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

    /// Release a previously acquired gateway reservation (e.g. when call
    /// creation fails after `select_healthy_candidates` succeeded).
    pub fn release_acquire(&mut self, gateway_id: &str) {
        if let Some(health) = self.states.get_mut(gateway_id) {
            health.half_open_probe_in_flight = false;
        }
    }
}

// ---------------------------------------------------------------------------
// Route table
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RouteTable {
    routes: Vec<Route>,
}

fn weighted_shuffle(mut items: Vec<&Route>) -> Vec<&Route> {
    use rand::Rng;
    let mut result = Vec::with_capacity(items.len());
    let mut rng = rand::thread_rng();

    while !items.is_empty() {
        let total_weight: u32 = items.iter().map(|item| item.weight.max(1)).sum();
        if total_weight == 0 {
            result.extend(items);
            break;
        }
        let mut target = rng.gen_range(0..total_weight);
        let mut chosen_idx = 0;
        for (idx, item) in items.iter().enumerate() {
            let w = item.weight.max(1);
            if target < w {
                chosen_idx = idx;
                break;
            }
            target -= w;
        }
        result.push(items.remove(chosen_idx));
    }
    result
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
    /// 4. Weight-based random shuffle for equivalent routes
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
                .then_with(|| {
                    left.cost
                        .partial_cmp(&right.cost)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
        });

        // 相同条件（前缀长度、优先级、成本）下的加权随机负载洗牌
        let mut grouped_routes = Vec::new();
        let mut current_group = Vec::new();

        for route in matching_routes {
            if current_group.is_empty() {
                current_group.push(route);
            } else {
                let first = current_group[0];
                let is_equivalent = first.prefix.len() == route.prefix.len()
                    && first.priority == route.priority
                    && (first.cost - route.cost).abs() < 1e-9;
                if is_equivalent {
                    current_group.push(route);
                } else {
                    grouped_routes.push(weighted_shuffle(current_group));
                    current_group = vec![route];
                }
            }
        }
        if !current_group.is_empty() {
            grouped_routes.push(weighted_shuffle(current_group));
        }

        let final_routes: Vec<&Route> = grouped_routes.into_iter().flatten().collect();

        let mut candidates = Vec::with_capacity(final_routes.len());
        for route in final_routes {
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
                .then_with(|| {
                    left.cost
                        .partial_cmp(&right.cost)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
        });

        // 相同条件（前缀长度、优先级、成本）下的加权随机负载洗牌
        let mut grouped_routes = Vec::new();
        let mut current_group = Vec::new();

        for route in matching_routes {
            if current_group.is_empty() {
                current_group.push(route);
            } else {
                let first = current_group[0];
                let is_equivalent = first.prefix.len() == route.prefix.len()
                    && first.priority == route.priority
                    && (first.cost - route.cost).abs() < 1e-9;
                if is_equivalent {
                    current_group.push(route);
                } else {
                    grouped_routes.push(weighted_shuffle(current_group));
                    current_group = vec![route];
                }
            }
        }
        if !current_group.is_empty() {
            grouped_routes.push(weighted_shuffle(current_group));
        }

        let final_routes: Vec<&Route> = grouped_routes.into_iter().flatten().collect();

        let mut candidates = Vec::with_capacity(final_routes.len());
        for route in final_routes {
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
    ///
    /// Only the first (best) candidate has `try_acquire` called, so that
    /// HalfOpen probe-in-flight flags are not leaked for unused candidates.
    pub fn select_healthy_candidates(
        &self,
        destination_uri: &SipUri,
        health: &mut GatewayHealthTracker,
        call_direction: Option<&str>,
    ) -> CallResult<Vec<SelectedRoute>> {
        let all_candidates = if let Some(dir) = call_direction {
            self.select_candidates_for_direction(destination_uri, dir)?
        } else {
            self.select_candidates(destination_uri)?
        };

        // First pass: filter by capacity and availability (no acquire).
        let available: Vec<SelectedRoute> = all_candidates
            .iter()
            .filter(|c| {
                let gid = c.target.gateway_id.as_str();
                health.has_capacity(gid, c.target.max_capacity) && health.is_available(gid)
            })
            .cloned()
            .collect();

        if available.is_empty() {
            warn_all_gateways_unhealthy(&all_candidates);
            return Err(CallError::GatewayUnavailable(
                destination_uri.user.clone().unwrap_or_default(),
            ));
        }

        // Second pass: acquire only on the first (selected) candidate.
        let first_gid = available[0].target.gateway_id.as_str();
        if !health.try_acquire(first_gid) {
            // First candidate not acquirable (e.g. HalfOpen probe already in flight).
            // Try subsequent candidates.
            for c in available.iter().skip(1) {
                let gid = c.target.gateway_id.as_str();
                if health.try_acquire(gid) {
                    // Reorder: put the acquired candidate first.
                    let mut result: Vec<SelectedRoute> = Vec::with_capacity(available.len());
                    result.push(c.clone());
                    for other in available.iter() {
                        if other.target.gateway_id.as_str() != gid {
                            result.push(other.clone());
                        }
                    }
                    return Ok(result);
                }
            }
            return Err(CallError::GatewayUnavailable(
                destination_uri.user.clone().unwrap_or_default(),
            ));
        }

        Ok(available)
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
    use std::time::SystemTime;

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
            .select_healthy_candidates(&make_uri("8613800138000"), &mut tracker, None)
            .unwrap();
        assert_eq!(healthy.len(), 1);
        assert_eq!(healthy[0].target.host, "gw2.example.com");
    }

    #[test]
    fn test_select_healthy_candidates_rejects_when_all_unhealthy() {
        let routes = vec![Route::new("r1", "86", 100, make_target("gw1.example.com"))];
        let table = RouteTable::new(routes);
        let mut tracker = GatewayHealthTracker::new(HealthThresholds {
            failure_threshold: 1,
            recovery_interval: Duration::from_secs(60),
            min_success_rate: 0.0,
            min_samples: 100,
        });

        tracker.record_failure("gw1");

        let result =
            table.select_healthy_candidates(&make_uri("8613800138000"), &mut tracker, None);
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
        let mut tracker = GatewayHealthTracker::new(HealthThresholds {
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
        let mut tracker = GatewayHealthTracker::new(HealthThresholds {
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
        let mut tracker = GatewayHealthTracker::new(HealthThresholds {
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
        let mut tracker = GatewayHealthTracker::new(HealthThresholds {
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
        let mut tracker = GatewayHealthTracker::new(HealthThresholds {
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
        let mut tracker = GatewayHealthTracker::new(HealthThresholds {
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
}
