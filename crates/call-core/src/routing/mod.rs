//! # 路由引擎与网关健康追踪
//!
//! 本模块实现了 VoIP 软交换的核心路由选择逻辑和网关健康熔断器。

pub(crate) mod health;
pub(crate) mod table;
pub(crate) mod types;

pub use health::{GatewayHealth, GatewayHealthTracker};
pub use table::RouteTable;
pub use types::{CircuitState, GatewayId, HealthThresholds, Route, RouteTarget, SelectedRoute};

#[cfg(test)]
mod tests;
