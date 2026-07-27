use std::time::Duration;

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
