use crate::{CallError, CallResult};
use sip_core::SipUri;
use std::time::Duration;

/// 网关唯一标识符。
///
/// 用于在路由表和健康追踪器中标识不同的网关。
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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
/// 路由引擎根据这些字段进行排序 and 选择：
/// - 前缀越长越优先（更精确匹配）
/// - 优先级数字越大越优先
/// - 成本越低越优先（LCR）
/// - 同等条件下按权重加权随机
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
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
            weight: 100,
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
            weight: 100,
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

    pub(crate) fn matches(&self, destination: &str) -> bool {
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

/// 网关健康熔断器状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum CircuitState {
    /// 正常状态，所有呼叫正常路由
    Closed,
    /// 熔断状态，拒绝所有呼叫，等待恢复间隔
    Open,
    /// 半开状态，允许少量探测呼叫
    HalfOpen,
}

/// 网关健康熔断器阈值配置。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct HealthThresholds {
    /// 连续失败次数阈值，超过后打开电路（默认 5）
    pub failure_threshold: u32,
    /// 恢复间隔：电路打开后多久进入 HalfOpen 探测
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
