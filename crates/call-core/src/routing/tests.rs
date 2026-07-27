use super::*;
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
fn test_prefix_match_and_priority_sort() {
    let routes = vec![
        Route::new("r1", "86", 100, make_target("gw1.example.com")),
        Route::new("r2", "8613", 200, make_target("gw2.example.com")),
        Route::new("r3", "8613", 100, make_target("gw3.example.com")),
    ];
    let table = RouteTable::new(routes);
    let candidates = table.select_candidates(&make_uri("8613800138000")).unwrap();

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

    assert_eq!(candidates[0].target.host, "gw2.example.com");
    assert_eq!(candidates[1].target.host, "gw3.example.com");
    assert_eq!(candidates[2].target.host, "gw1.example.com");
}

#[test]
fn endpoint_priority_orders_failover_within_same_route() {
    let routes = vec![
        Route::new("backup", "86", 100, make_target("backup.example.com"))
            .with_endpoint_priority(10),
        Route::new("primary", "86", 100, make_target("primary.example.com"))
            .with_endpoint_priority(100),
    ];
    let candidates = RouteTable::new(routes)
        .select_candidates(&make_uri("8613800138000"))
        .expect("endpoint routes should match");

    assert_eq!(candidates[0].target.host, "primary.example.com");
    assert_eq!(candidates[1].target.host, "backup.example.com");
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

    assert!(gw1_count > gw2_count);
    assert!(gw1_count > gw3_count);
    assert!(gw2_count > 100);
    assert!(gw3_count > 100);
}
