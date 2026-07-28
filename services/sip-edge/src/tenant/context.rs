//! 租户上下文：从 SIP 请求解析得到的 tenant 标识与策略快照。

use super::policy::{CrossTenantPolicy, TenantPolicy};

/// 单次呼叫的租户上下文。
///
/// 在 INVITE 入站时构造，贯穿 CallSession / CDR / 计费链路。
/// `tenant_id` 为 None 表示该呼叫属于"默认租户"（未配置多租户隔离）。
#[derive(Debug, Clone, Default)]
pub struct TenantContext {
    /// 租户唯一标识（通常为 UUID 或短字符串）。
    pub tenant_id: Option<String>,
    /// 租户显示名（用于日志/CDR）。
    pub tenant_name: Option<String>,
    /// SIP From 头中解析的域。
    pub domain: Option<String>,
    /// 该租户的运行时策略快照（避免运行时反复查表）。
    pub policy: TenantPolicy,
}

impl TenantContext {
    /// 创建一个默认租户上下文（未关联任何租户）。
    pub fn anonymous() -> Self {
        Self::default()
    }

    /// 从 SIP From 域构造上下文（不关联 tenant_id，仅记录域）。
    pub fn from_domain(domain: String) -> Self {
        Self {
            tenant_id: None,
            tenant_name: None,
            domain: Some(domain),
            policy: TenantPolicy::default(),
        }
    }

    /// 是否已关联到具体租户。
    pub fn is_bound(&self) -> bool {
        self.tenant_id.is_some()
    }

    /// 返回 tenant_id 切片（用于日志/CDR）。
    pub fn tenant_id_str(&self) -> &str {
        self.tenant_id.as_deref().unwrap_or("default")
    }

    /// 返回 tenant_name 切片（用于 CDR/审计日志的可读字段）。
    pub fn tenant_name_str(&self) -> &str {
        self.tenant_name.as_deref().unwrap_or("")
    }

    /// 返回 SIP From 域（用于跨租户策略判断与日志）。
    pub fn domain_str(&self) -> &str {
        self.domain.as_deref().unwrap_or("")
    }

    /// 是否允许跨租户呼叫到指定目标域。
    pub fn allows_cross_tenant_call_to(&self, target_domain: &str) -> bool {
        match self.policy.cross_tenant_policy {
            CrossTenantPolicy::Allow => true,
            CrossTenantPolicy::Deny => false,
            CrossTenantPolicy::AllowIfSameDomain => self
                .domain
                .as_deref()
                .map(|d| d.eq_ignore_ascii_case(target_domain))
                .unwrap_or(false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anonymous_context_is_unbound() {
        let ctx = TenantContext::anonymous();
        assert!(!ctx.is_bound());
        assert_eq!(ctx.tenant_id_str(), "default");
    }

    #[test]
    fn from_domain_records_domain_only() {
        let ctx = TenantContext::from_domain("example.com".to_string());
        assert!(!ctx.is_bound());
        assert_eq!(ctx.domain.as_deref(), Some("example.com"));
    }

    #[test]
    fn bound_context_reports_tenant_id() {
        let ctx = TenantContext {
            tenant_id: Some("t-001".to_string()),
            tenant_name: Some("Acme Corp".to_string()),
            domain: Some("acme.com".to_string()),
            policy: TenantPolicy::default(),
        };
        assert!(ctx.is_bound());
        assert_eq!(ctx.tenant_id_str(), "t-001");
    }

    #[test]
    fn cross_tenant_allow_policy_permits_any_target() {
        let ctx = TenantContext {
            tenant_id: Some("t-001".to_string()),
            policy: TenantPolicy {
                cross_tenant_policy: CrossTenantPolicy::Allow,
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(ctx.allows_cross_tenant_call_to("other.com"));
        assert!(ctx.allows_cross_tenant_call_to("acme.com"));
    }

    #[test]
    fn cross_tenant_deny_policy_blocks_all() {
        let ctx = TenantContext {
            tenant_id: Some("t-001".to_string()),
            policy: TenantPolicy {
                cross_tenant_policy: CrossTenantPolicy::Deny,
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(!ctx.allows_cross_tenant_call_to("other.com"));
        assert!(!ctx.allows_cross_tenant_call_to("acme.com"));
    }

    #[test]
    fn cross_tenant_same_domain_policy_allows_only_matching() {
        let ctx = TenantContext {
            tenant_id: Some("t-001".to_string()),
            domain: Some("acme.com".to_string()),
            policy: TenantPolicy {
                cross_tenant_policy: CrossTenantPolicy::AllowIfSameDomain,
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(ctx.allows_cross_tenant_call_to("acme.com"));
        assert!(!ctx.allows_cross_tenant_call_to("other.com"));
    }
}
