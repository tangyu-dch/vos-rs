//! # 路由引擎与网关健康追踪
//!
//! 本模块实现了 VoIP 软交换的核心路由选择逻辑、网关健康熔断器以及动态 HTTP Webhook 路由。

pub(crate) mod gateway_health;
pub(crate) mod health;
pub(crate) mod health_types;
pub(crate) mod table;
pub(crate) mod trie;
pub(crate) mod types;
pub(crate) mod webhook;

pub use gateway_health::GatewayHealth;
pub use health::GatewayHealthTracker;
pub use health_types::{CircuitState, HealthThresholds};
pub use table::RouteTable;
pub use types::{GatewayId, Route, RouteTarget, SelectedRoute};
pub use webhook::{
    WebhookRouteAction, WebhookRouteConfig, WebhookRouteRequest, WebhookRouteResponse,
    WebhookRouter,
};

#[cfg(test)]
mod health_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod webhook_tests;
