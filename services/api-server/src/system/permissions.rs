//! HTTP 资源到最小动作权限点的稳定映射。

/// 仅要求有效登录身份、不参与角色权限分配的内部授权标记。
pub(crate) const AUTHENTICATED_ONLY_PERMISSION: &str = "$authenticated";

/// 已注册的受保护路由与少量 Copilot 内部权限目标。
///
/// 先匹配此清单再映射权限，避免新增子路由因 `starts_with` 继承父级权限而意外放行。
const KNOWN_PERMISSION_TARGETS: &[(&str, &str)] = &[
    ("GET", "/rwi/v1/ws"),
    ("GET", "/api/v1/auth/me"),
    ("GET", "/api/v1/access-control"),
    ("GET", "/api/v1/access-control/accounts"),
    ("GET", "/api/v1/access-control/role-permissions"),
    ("POST", "/api/v1/access-control/users"),
    ("PUT", "/api/v1/access-control/users/:username"),
    ("DELETE", "/api/v1/access-control/users/:username"),
    ("POST", "/api/v1/access-control/roles"),
    ("PUT", "/api/v1/access-control/roles/:role_key"),
    ("DELETE", "/api/v1/access-control/roles/:role_key"),
    ("PUT", "/api/v1/access-control/roles/user-assignments"),
    ("PUT", "/api/v1/access-control/roles/:role_key/permissions"),
    ("PUT", "/api/v1/access-control/menus/:item_key"),
    ("GET", "/api/v1/notifications"),
    ("GET", "/api/v1/notifications/unread-count"),
    ("POST", "/api/v1/notifications/read-all"),
    ("POST", "/api/v1/notifications/scan"),
    ("POST", "/api/v1/notifications/:id/read"),
    ("GET|POST", "/api/v1/announcements"),
    ("POST", "/api/v1/announcements/:id/publish"),
    ("GET|PUT|DELETE", "/api/v1/announcements/:id"),
    ("GET", "/api/v1/my-announcements"),
    ("POST", "/api/v1/my-announcements/:id/read"),
    ("GET", "/api/v1/my-announcements/:id"),
    ("GET", "/api/v1/overview/summary"),
    ("GET", "/api/v1/overview/trends"),
    ("GET", "/api/v1/overview/node-traffic"),
    ("GET", "/api/v1/overview/monitoring-extras"),
    ("GET", "/api/v1/overview/events"),
    ("GET|POST", "/api/v1/extensions"),
    ("POST", "/api/v1/extensions/import"),
    ("GET", "/api/v1/extensions/import-template"),
    ("GET|PUT|DELETE", "/api/v1/extensions/:username"),
    ("GET|PUT", "/api/v1/extensions/:username/outbound-policy"),
    ("GET", "/api/v1/registrations"),
    ("GET|POST", "/api/v1/numbers"),
    ("POST", "/api/v1/numbers/import"),
    ("GET", "/api/v1/numbers/import-template"),
    ("PUT|DELETE", "/api/v1/numbers/:number"),
    ("PUT", "/api/v1/numbers/:number/owner"),
    ("GET|PUT", "/api/v1/numbers/:number/allocations"),
    ("GET|PUT", "/api/v1/numbers/:number/did-destination"),
    ("GET|POST", "/api/v1/trunks"),
    ("GET|PUT|DELETE", "/api/v1/trunks/:id"),
    ("GET|PUT", "/api/v1/trunks/:id/ip-rules"),
    ("GET|PUT", "/api/v1/trunks/:id/egress-endpoints"),
    ("GET|PUT", "/api/v1/trunks/:id/outbound-policy"),
    ("GET|POST", "/api/v1/caller-pools"),
    ("PUT|DELETE", "/api/v1/caller-pools/:id"),
    ("GET|PUT", "/api/v1/caller-pools/:id/members"),
    ("GET|POST", "/api/v1/egress-groups"),
    ("PUT|DELETE", "/api/v1/egress-groups/:id"),
    ("GET|PUT", "/api/v1/egress-groups/:id/members"),
    ("GET", "/api/v1/outbound-policies"),
    (
        "GET|PUT|DELETE",
        "/api/v1/outbound-policies/:source_type/:source_id",
    ),
    ("GET|POST", "/api/v1/did-destinations"),
    ("PUT|DELETE", "/api/v1/did-destinations/:number"),
    ("GET|POST", "/api/v1/routing/rules"),
    ("POST", "/api/v1/routing/rules/import"),
    ("GET", "/api/v1/routing/rules/import-template"),
    ("PUT|DELETE", "/api/v1/routing/rules/:id"),
    ("GET", "/api/v1/routing/simulations"),
    ("GET", "/api/v1/calls"),
    ("GET", "/api/v1/calls/active"),
    ("GET", "/api/v1/calls/:call_id"),
    ("GET", "/api/v1/calls/:call_id/media"),
    ("GET", "/api/v1/calls/:call_id/dtmf"),
    ("GET", "/api/v1/calls/:call_id/sipflow"),
    ("GET", "/api/v1/calls/:call_id/recording"),
    ("POST", "/api/v1/calls/:call_id/actions/terminate"),
    ("POST", "/api/v1/calls/:call_id/actions/play"),
    ("POST", "/api/v1/calls/:call_id/actions/stop-play"),
    ("POST", "/api/v1/calls/:call_id/actions/mute"),
    ("POST", "/api/v1/calls/:call_id/actions/unmute"),
    ("POST", "/api/v1/calls/:call_id/actions/monitor"),
    ("POST", "/api/v1/calls/:call_id/actions/stop-monitor"),
    ("GET", "/api/v1/reports/summary"),
    ("GET", "/api/v1/reports/export"),
    ("GET|POST", "/api/v1/billing/rates"),
    ("POST", "/api/v1/billing/rates/import"),
    ("GET", "/api/v1/billing/rates/import-template"),
    ("PUT|DELETE", "/api/v1/billing/rates/:id"),
    ("GET", "/api/v1/billing/accounts"),
    ("POST", "/api/v1/billing/accounts/:username/credit"),
    ("GET", "/api/v1/billing/transactions"),
    ("POST", "/api/v1/billing/reconciliations"),
    ("GET|POST", "/api/v1/security/anti-fraud/policies"),
    ("PUT|DELETE", "/api/v1/security/anti-fraud/policies/:id"),
    ("GET", "/api/v1/security/anti-fraud/settings"),
    ("PUT", "/api/v1/security/anti-fraud/settings/:key"),
    ("GET", "/api/v1/security/anti-fraud/deepfake-logs"),
    ("GET", "/api/v1/security/audit-logs"),
    ("POST", "/api/v1/copilot/chat"),
    ("GET|POST", "/api/v1/copilot/sessions"),
    ("GET|PUT|DELETE", "/api/v1/copilot/sessions/:id"),
    ("POST", "/api/v1/copilot/sessions/:id/chat"),
    ("POST", "/api/v1/copilot/sessions/:id/chat/stream"),
    ("GET", "/api/v1/copilot/sessions/:id/actions"),
    (
        "POST",
        "/api/v1/copilot/sessions/:id/actions/:action_id/approve",
    ),
    (
        "POST",
        "/api/v1/copilot/sessions/:id/actions/:action_id/reject",
    ),
    ("GET|POST", "/api/v1/llm-configs"),
    ("GET", "/api/v1/llm-configs/active"),
    ("PUT|DELETE", "/api/v1/llm-configs/:id"),
    ("POST", "/api/v1/llm-configs/:id/activate"),
    ("GET|POST", "/api/v1/infrastructure/settings"),
    ("GET|PUT", "/api/v1/infrastructure/media-cluster"),
    ("GET", "/api/v1/infrastructure/sip-cluster"),
    (
        "POST",
        "/api/v1/infrastructure/sip-cluster/nodes/:node_id/:action",
    ),
    ("GET", "/api/v1/infrastructure/media/metrics"),
    ("GET|POST", "/api/v1/call-center/queues"),
    ("PUT|DELETE", "/api/v1/call-center/queues/:id"),
    ("GET|POST", "/api/v1/call-center/agents"),
    ("PUT|DELETE", "/api/v1/call-center/agents/:id"),
    ("GET|POST", "/api/v1/ivr/menus"),
    ("GET|PUT|DELETE", "/api/v1/ivr/menus/:id"),
    ("GET", "/api/v1/ivr/prompts"),
    ("POST", "/api/v1/ivr/prompts/upload"),
    ("GET|DELETE", "/api/v1/ivr/prompts/:filename"),
    ("GET|POST", "/api/v1/tenants"),
    ("GET|PUT|DELETE", "/api/v1/tenants/:id"),
    ("POST", "/api/v1/tenants/:id/enabled"),
    ("PUT", "/api/v1/tenants/:id/billing-account"),
    ("GET|POST", "/api/system/configs"),
    ("GET|PUT", "/api/system/media-cluster"),
    ("GET", "/api/system/sip-cluster/status"),
    ("POST", "/api/system/sip-cluster/nodes/:node_id/:action"),
    ("GET", "/api/dashboard/stats"),
    ("GET", "/api/dashboard/trend"),
    ("GET", "/api/dashboard/events"),
    ("GET", "/api/cdrs"),
    ("GET", "/api/cdrs/:call_id"),
    ("GET", "/api/cdrs/:call_id/dtmf"),
    ("GET|POST", "/api/users"),
    ("PUT|DELETE", "/api/users/:username"),
    ("POST", "/api/users/import"),
    ("GET", "/api/users/import-template"),
    ("GET|POST", "/api/gateways"),
    ("PUT|DELETE", "/api/gateways/:id"),
    ("POST", "/api/gateways/import"),
    ("GET|POST", "/api/routes"),
    ("PUT|DELETE", "/api/routes/:id"),
    ("POST", "/api/routes/import"),
    ("GET", "/api/routes/import-template"),
    ("GET", "/api/registrations"),
    ("GET", "/api/recordings/:call_id/audio"),
    ("GET", "/api/reports/summary"),
    ("GET", "/api/reports/export"),
    ("GET", "/api/reports/daily"),
    ("GET|POST", "/api/rates"),
    ("PUT|DELETE", "/api/rates/:id"),
    ("POST", "/api/rates/import"),
    ("GET", "/api/rates/import-template"),
    ("GET", "/api/accounts"),
    ("POST", "/api/accounts/:username/credit"),
    ("GET", "/api/ledger"),
    ("POST", "/api/billing/reconcile"),
    ("GET", "/api/calls/active"),
    ("POST", "/api/calls/:call_id/terminate"),
    ("GET", "/api/route-preview"),
    ("GET", "/api/media/metrics"),
    ("GET|POST", "/api/numbers"),
    ("PUT|DELETE", "/api/numbers/:number"),
    ("POST", "/api/numbers/import"),
    ("GET", "/api/numbers/import-template"),
    ("GET|POST", "/api/anti-fraud/rules"),
    ("PUT|DELETE", "/api/anti-fraud/rules/:id"),
    ("GET", "/api/anti-fraud/config"),
    ("PUT", "/api/anti-fraud/config/:key"),
    ("GET", "/api/audit-logs"),
];

/// 返回受保护接口所需的权限点。未识别的接口默认拒绝。
pub fn required_permission(method: &str, path: &str) -> Option<&'static str> {
    if !is_known_permission_target(method, path) {
        return None;
    }
    if path == "/rwi/v1/ws" {
        return Some("calls.monitor");
    }
    if path == "/api/v1/auth/me" {
        return Some("session.read");
    }
    if path == "/api/v1/access-control" {
        return Some("access.view");
    }
    if path == "/api/v1/access-control/accounts" {
        return Some("access.accounts.view");
    }
    if path == "/api/v1/access-control/role-permissions" {
        return Some("access.roles.view");
    }
    if path.starts_with("/api/v1/access-control/users") {
        return Some(match method {
            "GET" => "access.accounts.view",
            "POST" => "access.accounts.create",
            "PUT" => "access.accounts.update",
            "DELETE" => "access.accounts.delete",
            _ => return None,
        });
    }
    if path == "/api/v1/access-control/roles/user-assignments" {
        return Some("access.roles.assign");
    }
    if path.ends_with("/permissions") && path.starts_with("/api/v1/access-control/roles/") {
        return Some("access.roles.permissions");
    }
    if path.starts_with("/api/v1/access-control/roles") {
        return Some(match method {
            "GET" => "access.roles.view",
            "POST" => "access.roles.create",
            "PUT" => "access.roles.update",
            "DELETE" => "access.roles.delete",
            _ => return None,
        });
    }
    if path.starts_with("/api/v1/access-control/menus") {
        return Some(if method == "GET" {
            "access.view"
        } else {
            "access.menus"
        });
    }
    if path == "/api/v1/notifications/scan" {
        return Some("notifications.scan");
    }
    if path.starts_with("/api/v1/notifications") {
        return Some(if method == "GET" {
            "notifications.view"
        } else {
            "notifications.read"
        });
    }
    if path.starts_with("/api/v1/my-announcements") {
        return Some(AUTHENTICATED_ONLY_PERMISSION);
    }
    if path.starts_with("/api/v1/announcements") {
        return Some(if path.ends_with("/publish") {
            "announcements.publish"
        } else {
            match method {
                "GET" => "announcements.view",
                "POST" => "announcements.create",
                "PUT" => "announcements.update",
                "DELETE" => "announcements.delete",
                _ => return None,
            }
        });
    }
    if path.starts_with("/api/v1/copilot") {
        return Some(if path.contains("/actions/") && method == "POST" {
            "copilot.execute"
        } else {
            "copilot.use"
        });
    }
    if path.starts_with("/api/v1/llm-configs") {
        return Some(if path.ends_with("/activate") {
            "llm.activate"
        } else {
            match method {
                "GET" => "llm.view",
                "POST" => "llm.create",
                "PUT" => "llm.update",
                "DELETE" => "llm.delete",
                _ => return None,
            }
        });
    }
    if path.starts_with("/api/v1/overview") || path.starts_with("/api/dashboard") {
        return Some("overview.view");
    }
    if path.starts_with("/api/v1/extensions/") && path.ends_with("/outbound-policy") {
        return termination_permission(method);
    }
    if path.starts_with("/api/v1/extensions") || path.starts_with("/api/users") {
        return resource_permission(method, path, "extensions");
    }
    if path.starts_with("/api/v1/registrations") || path.starts_with("/api/registrations") {
        return Some("registrations.view");
    }
    if path.starts_with("/api/v1/numbers/")
        && (path.ends_with("/owner")
            || path.ends_with("/allocations")
            || path.ends_with("/did-destination"))
    {
        return termination_permission(method);
    }
    if path.starts_with("/api/v1/numbers") || path.starts_with("/api/numbers") {
        return resource_permission(method, path, "numbers");
    }
    if path.starts_with("/api/v1/trunks/")
        && (path.contains("/ip-rules")
            || path.contains("/egress-endpoints")
            || path.contains("/outbound-policy"))
    {
        return Some(if method == "GET" {
            "termination.view"
        } else {
            "termination.manage"
        });
    }
    if path.starts_with("/api/v1/trunks") || path.starts_with("/api/gateways") {
        return resource_permission(method, path, "trunks");
    }
    if path.starts_with("/api/v1/routing/simulations") || path == "/api/route-preview" {
        return Some("routing.simulate");
    }
    if path.starts_with("/api/v1/routing/rules") || path.starts_with("/api/routes") {
        return resource_permission(method, path, "routing");
    }
    if is_termination_path(path) {
        return Some(if method == "GET" {
            "termination.view"
        } else {
            "termination.manage"
        });
    }
    if path.starts_with("/api/v1/calls") || is_legacy_call_path(path) {
        return call_permission(method, path);
    }
    if path.starts_with("/api/v1/reports")
        || path.starts_with("/api/cdrs")
        || path.starts_with("/api/recordings")
        || path.starts_with("/api/reports")
    {
        return Some(if path.contains("export") {
            "calls.export"
        } else {
            "calls.view"
        });
    }
    if path.starts_with("/api/v1/billing/accounts") || path.starts_with("/api/accounts") {
        return Some(if method == "GET" && path.contains("export") {
            "billing.accounts.export"
        } else if method == "GET" {
            "billing.accounts.view"
        } else {
            "billing.accounts.credit"
        });
    }
    if path.starts_with("/api/v1/billing/rates") || path.starts_with("/api/rates") {
        return resource_permission(method, path, "billing.rates");
    }
    if path.starts_with("/api/v1/billing/transactions") || path.starts_with("/api/ledger") {
        return Some(if path.contains("export") {
            "billing.ledger.export"
        } else {
            "billing.ledger.view"
        });
    }
    if path.starts_with("/api/v1/billing/reconciliations")
        || path.starts_with("/api/billing/reconcile")
    {
        return Some("billing.reconcile");
    }
    if path.starts_with("/api/v1/security/audit-logs") || path.starts_with("/api/audit-logs") {
        return Some("security.audit");
    }
    if path.starts_with("/api/v1/security") || path.starts_with("/api/anti-fraud") {
        return Some(if method == "GET" {
            "security.view"
        } else {
            "security.manage"
        });
    }
    if path == "/api/v1/infrastructure/settings" || path == "/api/system/configs" {
        return Some(if method == "GET" {
            "settings.view"
        } else {
            "settings.manage"
        });
    }
    if path.starts_with("/api/v1/infrastructure") || path.starts_with("/api/system") {
        return Some(if method == "GET" {
            "infrastructure.view"
        } else {
            "infrastructure.manage"
        });
    }
    if path.starts_with("/api/v1/call-center/queues") {
        return resource_permission(method, path, "queues");
    }
    if path.starts_with("/api/v1/call-center/agents") {
        return resource_permission(method, path, "agents");
    }
    if path.starts_with("/api/v1/ivr/prompts") {
        return Some(if method == "GET" {
            "ivr.view"
        } else {
            "ivr.prompts"
        });
    }
    if path.starts_with("/api/v1/ivr") {
        return resource_permission(method, path, "ivr");
    }
    if path.starts_with("/api/v1/tenants/") && path.ends_with("/enabled") {
        return Some(if method == "GET" {
            "tenants.view"
        } else {
            "tenants.update"
        });
    }
    if path.starts_with("/api/v1/tenants") {
        return resource_permission(method, path, "tenants");
    }
    if path.starts_with("/api/v1/settings") {
        return Some(if method == "GET" {
            "settings.view"
        } else {
            "settings.manage"
        });
    }
    None
}

fn is_known_permission_target(method: &str, path: &str) -> bool {
    KNOWN_PERMISSION_TARGETS.iter().any(|(methods, template)| {
        methods.split('|').any(|candidate| candidate == method) && path_matches(path, template)
    })
}

fn path_matches(path: &str, template: &str) -> bool {
    let route_path = path.split('?').next().unwrap_or(path);
    let mut actual = route_path.split('/').filter(|segment| !segment.is_empty());
    let mut expected = template.split('/').filter(|segment| !segment.is_empty());
    loop {
        match (actual.next(), expected.next()) {
            (Some(value), Some(pattern)) if pattern.starts_with(':') || value == pattern => {}
            (None, None) => return true,
            _ => return false,
        }
    }
}

fn resource_permission(method: &str, path: &str, resource: &str) -> Option<&'static str> {
    let action = if method == "GET" && path.contains("export") {
        "export"
    } else if path.contains("import") {
        "import"
    } else if method == "GET" {
        "view"
    } else if method == "POST" {
        "create"
    } else if matches!(method, "PUT" | "PATCH") {
        "update"
    } else if method == "DELETE" {
        "delete"
    } else {
        return None;
    };
    permission_key(resource, action)
}

fn permission_key(resource: &str, action: &str) -> Option<&'static str> {
    match (resource, action) {
        ("extensions", "view") => Some("extensions.view"),
        ("extensions", "create") => Some("extensions.create"),
        ("extensions", "update") => Some("extensions.update"),
        ("extensions", "delete") => Some("extensions.delete"),
        ("extensions", "import") => Some("extensions.import"),
        ("extensions", "export") => Some("extensions.export"),
        ("numbers", "view") => Some("numbers.view"),
        ("numbers", "create") => Some("numbers.create"),
        ("numbers", "update") => Some("numbers.update"),
        ("numbers", "delete") => Some("numbers.delete"),
        ("numbers", "import") => Some("numbers.import"),
        ("numbers", "export") => Some("numbers.export"),
        ("trunks", "view") => Some("trunks.view"),
        ("trunks", "create") => Some("trunks.create"),
        ("trunks", "update") => Some("trunks.update"),
        ("trunks", "delete") => Some("trunks.delete"),
        ("trunks", "import") => Some("trunks.import"),
        ("trunks", "export") => Some("trunks.export"),
        ("routing", "view") => Some("routing.view"),
        ("routing", "create") => Some("routing.create"),
        ("routing", "update") => Some("routing.update"),
        ("routing", "delete") => Some("routing.delete"),
        ("routing", "import") => Some("routing.import"),
        ("routing", "export") => Some("routing.export"),
        ("billing.rates", "view") => Some("billing.rates.view"),
        ("billing.rates", "create") => Some("billing.rates.create"),
        ("billing.rates", "update") => Some("billing.rates.update"),
        ("billing.rates", "delete") => Some("billing.rates.delete"),
        ("billing.rates", "import") => Some("billing.rates.import"),
        ("billing.rates", "export") => Some("billing.rates.export"),
        ("queues", "view") => Some("queues.view"),
        ("queues", "create") => Some("queues.create"),
        ("queues", "update") => Some("queues.update"),
        ("queues", "delete") => Some("queues.delete"),
        ("queues", "export") => Some("queues.export"),
        ("agents", "view") => Some("agents.view"),
        ("agents", "create") => Some("agents.create"),
        ("agents", "update") => Some("agents.update"),
        ("agents", "delete") => Some("agents.delete"),
        ("agents", "export") => Some("agents.export"),
        ("ivr", "view") => Some("ivr.view"),
        ("ivr", "create") => Some("ivr.create"),
        ("ivr", "update") => Some("ivr.update"),
        ("ivr", "delete") => Some("ivr.delete"),
        ("tenants", "view") => Some("tenants.view"),
        ("tenants", "create") => Some("tenants.create"),
        ("tenants", "update") => Some("tenants.update"),
        ("tenants", "delete") => Some("tenants.delete"),
        ("tenants", "export") => Some("tenants.export"),
        _ => None,
    }
}

fn call_permission(method: &str, path: &str) -> Option<&'static str> {
    if method == "GET" {
        return Some(if path.contains("export") {
            "calls.export"
        } else {
            "calls.view"
        });
    }
    if path.contains("terminate") {
        Some("calls.terminate")
    } else if path.contains("play") {
        Some("calls.play")
    } else if path.contains("mute") {
        Some("calls.mute")
    } else if path.contains("monitor") {
        Some("calls.monitor")
    } else {
        None
    }
}

fn termination_permission(method: &str) -> Option<&'static str> {
    match method {
        "GET" => Some("termination.view"),
        "POST" | "PUT" | "PATCH" | "DELETE" => Some("termination.manage"),
        _ => None,
    }
}

fn is_termination_path(path: &str) -> bool {
    [
        "/api/v1/caller-pools",
        "/api/v1/egress-groups",
        "/api/v1/outbound-policies",
        "/api/v1/did-destinations",
    ]
    .iter()
    .any(|prefix| path.starts_with(prefix))
}

fn is_legacy_call_path(path: &str) -> bool {
    path.starts_with("/api/calls") || path == "/api/media/metrics"
}

#[cfg(test)]
mod tests {
    use super::{required_permission, AUTHENTICATED_ONLY_PERMISSION};

    #[test]
    fn call_buttons_have_independent_permissions() {
        assert_eq!(
            required_permission("POST", "/api/v1/calls/id/actions/terminate"),
            Some("calls.terminate")
        );
        assert_eq!(
            required_permission("POST", "/api/v1/calls/id/actions/monitor"),
            Some("calls.monitor")
        );
    }

    #[test]
    fn resource_buttons_have_independent_permissions() {
        assert_eq!(
            required_permission("POST", "/api/v1/extensions"),
            Some("extensions.create")
        );
        assert_eq!(
            required_permission("DELETE", "/api/v1/extensions/alice"),
            Some("extensions.delete")
        );
        assert_eq!(
            required_permission("POST", "/api/v1/billing/accounts/a/credit"),
            Some("billing.accounts.credit")
        );
        assert_eq!(
            required_permission("GET", "/api/v1/call-center/queues?export=true"),
            Some("queues.export")
        );
        assert_eq!(
            required_permission("GET", "/api/v1/call-center/agents?export=true"),
            Some("agents.export")
        );
        assert_eq!(
            required_permission("GET", "/api/v1/tenants?export=true"),
            Some("tenants.export")
        );
        assert_eq!(
            required_permission("GET", "/api/v1/billing/accounts?export=true"),
            Some("billing.accounts.export")
        );
        assert_eq!(
            required_permission("GET", "/api/v1/billing/transactions?export=true"),
            Some("billing.ledger.export")
        );
    }

    #[test]
    fn role_management_actions_have_independent_permissions() {
        assert_eq!(
            required_permission("GET", "/api/v1/access-control/role-permissions"),
            Some("access.roles.view")
        );
        assert_eq!(
            required_permission("POST", "/api/v1/access-control/roles"),
            Some("access.roles.create")
        );
        assert_eq!(
            required_permission("PUT", "/api/v1/access-control/roles/operator"),
            Some("access.roles.update")
        );
        assert_eq!(
            required_permission("DELETE", "/api/v1/access-control/roles/operator"),
            Some("access.roles.delete")
        );
        assert_eq!(
            required_permission("PUT", "/api/v1/access-control/roles/operator/permissions"),
            Some("access.roles.permissions")
        );
        assert_eq!(
            required_permission("PUT", "/api/v1/access-control/roles/user-assignments"),
            Some("access.roles.assign")
        );
    }

    #[test]
    fn account_management_actions_have_independent_permissions() {
        assert_eq!(
            required_permission("GET", "/api/v1/access-control/accounts"),
            Some("access.accounts.view")
        );
        assert_eq!(
            required_permission("POST", "/api/v1/access-control/users"),
            Some("access.accounts.create")
        );
        assert_eq!(
            required_permission("PUT", "/api/v1/access-control/users/alice"),
            Some("access.accounts.update")
        );
        assert_eq!(
            required_permission("DELETE", "/api/v1/access-control/users/alice"),
            Some("access.accounts.delete")
        );
    }

    #[test]
    fn model_management_actions_have_independent_permissions() {
        assert_eq!(
            required_permission("GET", "/api/v1/llm-configs"),
            Some("llm.view")
        );
        assert_eq!(
            required_permission("POST", "/api/v1/llm-configs"),
            Some("llm.create")
        );
        assert_eq!(
            required_permission("PUT", "/api/v1/llm-configs/1"),
            Some("llm.update")
        );
        assert_eq!(
            required_permission("DELETE", "/api/v1/llm-configs/1"),
            Some("llm.delete")
        );
        assert_eq!(
            required_permission("POST", "/api/v1/llm-configs/1/activate"),
            Some("llm.activate")
        );
    }

    #[test]
    fn announcement_management_actions_have_independent_permissions() {
        let cases = [
            ("GET", "/api/v1/announcements", "announcements.view"),
            ("POST", "/api/v1/announcements", "announcements.create"),
            ("PUT", "/api/v1/announcements/1", "announcements.update"),
            ("DELETE", "/api/v1/announcements/1", "announcements.delete"),
            (
                "POST",
                "/api/v1/announcements/1/publish",
                "announcements.publish",
            ),
            (
                "POST",
                "/api/v1/my-announcements/1/read",
                AUTHENTICATED_ONLY_PERMISSION,
            ),
        ];
        for (method, path, permission) in cases {
            assert_eq!(required_permission(method, path), Some(permission));
        }
    }

    #[test]
    fn settings_and_termination_subresources_use_their_own_domains() {
        assert_eq!(
            required_permission("GET", "/api/v1/infrastructure/settings"),
            Some("settings.view")
        );
        assert_eq!(
            required_permission("POST", "/api/v1/infrastructure/settings"),
            Some("settings.manage")
        );
        assert_eq!(
            required_permission("POST", "/api/system/configs"),
            Some("settings.manage")
        );
        assert_eq!(
            required_permission("PUT", "/api/v1/extensions/alice/outbound-policy"),
            Some("termination.manage")
        );
        assert_eq!(
            required_permission("GET", "/api/v1/numbers/1001/allocations"),
            Some("termination.view")
        );
    }

    #[test]
    fn special_actions_are_not_mapped_to_create_or_view() {
        assert_eq!(
            required_permission("POST", "/api/v1/tenants/acme/enabled"),
            Some("tenants.update")
        );
        assert_eq!(
            required_permission("GET", "/api/v1/calls/active?export=true"),
            Some("calls.export")
        );
    }

    #[test]
    fn every_protected_route_family_has_an_explicit_permission() {
        let samples = [
            ("GET", "/api/v1/auth/me"),
            ("GET", "/api/v1/access-control"),
            ("POST", "/api/v1/access-control/users"),
            ("DELETE", "/api/v1/access-control/users/operator"),
            ("DELETE", "/api/v1/access-control/roles/operator"),
            ("PUT", "/api/v1/access-control/roles/user-assignments"),
            ("PUT", "/api/v1/access-control/roles/operator/permissions"),
            ("GET", "/api/v1/notifications"),
            ("POST", "/api/v1/notifications/read-all"),
            ("POST", "/api/v1/notifications/scan"),
            ("POST", "/api/v1/announcements/1/publish"),
            ("GET", "/api/v1/my-announcements/1"),
            ("POST", "/api/v1/copilot/chat"),
            ("POST", "/api/v1/copilot/sessions/id/actions/action/approve"),
            ("PUT", "/api/v1/llm-configs/1"),
            ("GET", "/api/v1/overview/summary"),
            ("GET", "/api/v1/extensions/import-template"),
            ("GET", "/api/v1/registrations"),
            ("DELETE", "/api/v1/numbers/1001"),
            ("PUT", "/api/v1/trunks/gw/ip-rules"),
            ("POST", "/api/v1/trunks"),
            ("GET", "/api/v1/routing/simulations"),
            ("POST", "/api/v1/routing/rules/import"),
            ("PUT", "/api/v1/caller-pools/id/members"),
            ("GET", "/api/v1/calls/id/sipflow"),
            ("POST", "/api/v1/calls/id/actions/stop-monitor"),
            ("GET", "/api/v1/reports/export"),
            ("POST", "/api/v1/billing/accounts/alice/credit"),
            ("DELETE", "/api/v1/billing/rates/1"),
            ("GET", "/api/v1/billing/transactions"),
            ("POST", "/api/v1/billing/reconciliations"),
            ("GET", "/api/v1/security/audit-logs"),
            ("PUT", "/api/v1/security/anti-fraud/settings/enabled"),
            ("POST", "/api/v1/infrastructure/settings"),
            ("POST", "/api/v1/infrastructure/sip-cluster/nodes/a/restart"),
            ("DELETE", "/api/v1/call-center/queues/id"),
            ("PUT", "/api/v1/call-center/agents/id"),
            ("POST", "/api/v1/ivr/prompts/upload"),
            ("DELETE", "/api/v1/ivr/menus/id"),
            ("POST", "/api/v1/tenants/id/enabled"),
            ("GET", "/api/dashboard/stats"),
            ("GET", "/api/cdrs"),
            ("POST", "/api/users/import"),
            ("PUT", "/api/gateways/id"),
            ("GET", "/api/reports/summary"),
            ("GET", "/api/accounts"),
            ("POST", "/api/billing/reconcile"),
            ("POST", "/api/calls/id/terminate"),
            ("GET", "/api/media/metrics"),
            ("PUT", "/api/anti-fraud/config/enabled"),
            ("GET", "/api/audit-logs"),
            ("GET", "/rwi/v1/ws"),
        ];

        for (method, path) in samples {
            assert!(
                required_permission(method, path).is_some(),
                "受保护接口缺少权限映射: {method} {path}"
            );
        }
    }

    #[test]
    fn unknown_protected_route_is_denied_by_default() {
        assert_eq!(required_permission("GET", "/api/v1/not-configured"), None);
        assert_eq!(
            required_permission("GET", "/api/v1/security/not-configured"),
            None
        );
        assert_eq!(
            required_permission("POST", "/api/v1/extensions/alice"),
            None
        );
    }
}
