#[cfg(test)]
mod tests {
    use crate::routing::table::RouteTable;
    use crate::routing::types::{Route, RouteTarget};
    use crate::routing::webhook::{
        WebhookRouteAction, WebhookRouteConfig, WebhookRouteResponse, WebhookRouter,
    };
    use sip_core::SipUri;

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
    fn test_webhook_routing_decision_overrides_lcr() -> Result<(), Box<dyn std::error::Error>> {
        let routes = vec![Route::new(
            "r1",
            "86",
            100,
            make_target("static.example.com"),
        )];
        let mut table = RouteTable::new(routes);

        let router = WebhookRouter::new(Some(WebhookRouteConfig::new("http://localhost/route")));
        table.set_webhook_router(router);

        let webhook_resp = WebhookRouteResponse {
            action: WebhookRouteAction::Route,
            targets: vec![RouteTarget::new(
                "dyn_gw",
                "dynamic.example.com",
                Some(5060),
            )],
            reject_reason: None,
            reject_sip_code: None,
        };

        let candidates = table.select_candidates_with_webhook(
            &make_uri("8613800138000"),
            Some(&webhook_resp),
            Some("inbound"),
        )?;

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].target.host, "dynamic.example.com");
        Ok(())
    }

    #[test]
    fn test_webhook_routing_fallback_to_lcr() -> Result<(), Box<dyn std::error::Error>> {
        let routes = vec![Route::new(
            "r1",
            "86",
            100,
            make_target("static.example.com"),
        )];
        let table = RouteTable::new(routes);

        let webhook_resp = WebhookRouteResponse {
            action: WebhookRouteAction::FallbackToLcr,
            targets: vec![],
            reject_reason: None,
            reject_sip_code: None,
        };

        let candidates = table.select_candidates_with_webhook(
            &make_uri("8613800138000"),
            Some(&webhook_resp),
            Some("inbound"),
        )?;

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].target.host, "static.example.com");
        Ok(())
    }
}
