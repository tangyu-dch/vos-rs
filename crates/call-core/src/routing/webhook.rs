//! 动态 HTTP Webhook 路由模块。
//!
//! 允许呼叫路由引擎向外部 HTTP Webhook 发起异步路由请求，以根据第三方业务逻辑
//! 动态决策出站网关目标，并可与静态 prefix LCR 路由进行混合及降级匹配。

use super::types::{RouteTarget, SelectedRoute};
use crate::{CallError, CallResult};
use serde::{Deserialize, Serialize};
use sip_core::SipUri;
use std::collections::HashMap;

/// Webhook 动态路由配置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebhookRouteConfig {
    /// Webhook 目标 URL。
    pub url: String,
    /// HTTP 请求超时时间（毫秒），默认 3000ms。
    pub timeout_ms: u64,
    /// 鉴权 Token（如 Bearer token 或 Secret Key）。
    pub auth_token: Option<String>,
    /// 当 Webhook 异常/超时/请求失败时，是否降级回退到 Prefix LCR 路由。
    pub fallback_to_lcr: bool,
}

impl WebhookRouteConfig {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            timeout_ms: 3000,
            auth_token: None,
            fallback_to_lcr: true,
        }
    }

    pub fn with_timeout(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = timeout_ms;
        self
    }

    pub fn with_auth(mut self, auth_token: impl Into<String>) -> Self {
        self.auth_token = Some(auth_token.into());
        self
    }

    pub fn with_fallback(mut self, fallback_to_lcr: bool) -> Self {
        self.fallback_to_lcr = fallback_to_lcr;
        self
    }
}

/// Webhook 路由请求上下文。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebhookRouteRequest {
    /// SIP Call-ID。
    pub call_id: String,
    /// 主叫号码。
    pub caller: String,
    /// 被叫号码。
    pub callee: String,
    /// 呼叫方向（如 `"inbound"`、`"outbound"`）。
    pub direction: String,
    /// 附加元数据或自定义 SIP Headers。
    pub metadata: HashMap<String, String>,
}

/// Webhook 路由决策返回。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebhookRouteResponse {
    /// Webhook 控制动作。
    pub action: WebhookRouteAction,
    /// 动态返回的候选网关列表。
    #[serde(default)]
    pub targets: Vec<RouteTarget>,
    /// 拒绝原因（当 `action = Reject` 时有效）。
    pub reject_reason: Option<String>,
    /// 建议的 SIP 错误响应状态码（如 404/603）。
    pub reject_sip_code: Option<u16>,
}

/// Webhook 路由决策动作类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebhookRouteAction {
    /// 使用 Webhook 返回的网关列表进行路由。
    Route,
    /// 直接拒绝/挂断该呼叫。
    Reject,
    /// 回退到系统的 Prefix LCR 路由算法。
    FallbackToLcr,
}

/// Dynamic HTTP Webhook Router 评估与构建器。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WebhookRouter {
    config: Option<WebhookRouteConfig>,
}

impl WebhookRouter {
    pub fn new(config: Option<WebhookRouteConfig>) -> Self {
        Self { config }
    }

    pub fn config(&self) -> Option<&WebhookRouteConfig> {
        self.config.as_ref()
    }

    pub fn is_enabled(&self) -> bool {
        self.config.is_some()
    }

    /// 构建 Webhook 路由请求体。
    pub fn build_request(
        &self,
        call_id: &str,
        caller: &str,
        callee: &str,
        direction: &str,
        metadata: HashMap<String, String>,
    ) -> WebhookRouteRequest {
        WebhookRouteRequest {
            call_id: call_id.to_string(),
            caller: caller.to_string(),
            callee: callee.to_string(),
            direction: direction.to_string(),
            metadata,
        }
    }

    /// 解析来自 Webhook 服务的 JSON 响应。
    pub fn parse_response(&self, json_payload: &str) -> CallResult<WebhookRouteResponse> {
        serde_json::from_str(json_payload).map_err(|e| {
            CallError::WebhookRoutingError(format!("Invalid webhook route response: {e}"))
        })
    }

    /// 将 Webhook 响应转换为选中的候选 SelectedRoute 列表。
    pub fn evaluate_response(
        &self,
        destination_uri: &SipUri,
        response: &WebhookRouteResponse,
    ) -> CallResult<Vec<SelectedRoute>> {
        match response.action {
            WebhookRouteAction::Route => {
                if response.targets.is_empty() {
                    return Err(CallError::NoRouteForDestination(
                        destination_uri
                            .user
                            .as_deref()
                            .unwrap_or_default()
                            .to_string(),
                    ));
                }
                let mut candidates = Vec::with_capacity(response.targets.len());
                for (idx, target) in response.targets.iter().enumerate() {
                    let route_id = format!("webhook_dyn_{}", idx + 1);
                    let outbound_uri = target.outbound_uri_for(destination_uri)?;
                    candidates.push(SelectedRoute {
                        route_id,
                        target: target.clone(),
                        outbound_uri,
                    });
                }
                Ok(candidates)
            }
            WebhookRouteAction::Reject => {
                let reason = response
                    .reject_reason
                    .clone()
                    .unwrap_or_else(|| "Rejected by Webhook route policy".to_string());
                Err(CallError::NoRouteForDestination(reason))
            }
            WebhookRouteAction::FallbackToLcr => Err(CallError::NoRouteForDestination(
                "Webhook requested FallbackToLcr".to_string(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_webhook_route_request_building() {
        let router = WebhookRouter::new(Some(WebhookRouteConfig::new("https://example.com/route")));
        let req = router.build_request("call-1", "1001", "1002", "inbound", HashMap::new());
        assert_eq!(req.call_id, "call-1");
        assert_eq!(req.caller, "1001");
    }

    #[test]
    fn test_webhook_route_response_parse_and_evaluate() -> Result<(), Box<dyn std::error::Error>> {
        let json = r#"{
            "action": "route",
            "targets": [
                {
                    "gateway_id": "gw1",
                    "host": "192.168.1.100",
                    "port": 5060,
                    "current_concurrent": 0
                }
            ]
        }"#;

        let router = WebhookRouter::new(Some(WebhookRouteConfig::new("http://localhost/route")));
        let resp = router.parse_response(json)?;
        assert_eq!(resp.action, WebhookRouteAction::Route);

        let sip_uri = SipUri::parse("sip:1002@domain.com")?;
        let candidates = router.evaluate_response(&sip_uri, &resp)?;
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].target.gateway_id.as_str(), "gw1");
        Ok(())
    }
}
