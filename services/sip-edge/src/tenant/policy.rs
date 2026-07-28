//! 租户级运行时策略。

/// 跨租户呼叫策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CrossTenantPolicy {
    /// 允许跨租户呼叫（开放模式，适合单一运营商场景）。
    Allow,
    /// 拒绝所有跨租户呼叫（严格隔离模式）。
    Deny,
    /// 仅允许同域内呼叫（默认模式，向后兼容现有 check_cross_tenant 行为）。
    AllowIfSameDomain,
}

impl Default for CrossTenantPolicy {
    fn default() -> Self {
        Self::AllowIfSameDomain
    }
}

/// 租户级运行时策略快照。
///
/// 在 INVITE 入站时从 TenantRegistry 加载，与 TenantContext 一起贯穿呼叫生命周期。
/// 所有 `0` 值表示"无限制"。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, Default)]
pub struct TenantPolicy {
    /// 最大并发通话数（0 = 不限制）。
    pub max_concurrent_calls: u32,
    /// 最大 CPS（每秒呼叫数，0 = 不限制）。
    pub max_cps: u32,
    /// 跨租户呼叫策略。
    pub cross_tenant_policy: CrossTenantPolicy,
    /// 是否启用录音（覆盖全局录音开关）。
    pub recording_enabled: Option<bool>,
    /// 允许的网关 ID 列表（None = 全部允许）。
    pub allowed_gateway_ids: Option<Vec<String>>,
    /// 计费账户 ID（覆盖 per-user 计费账户）。
    pub billing_account_id: Option<i64>,
}

impl TenantPolicy {
    /// 是否允许使用指定网关。
    pub fn allows_gateway(&self, gateway_id: &str) -> bool {
        self.allowed_gateway_ids
            .as_ref()
            .map(|list| list.iter().any(|g| g == gateway_id))
            .unwrap_or(true)
    }

    /// 是否受并发上限约束。
    pub fn has_concurrency_limit(&self) -> bool {
        self.max_concurrent_calls > 0
    }

    /// 是否受 CPS 上限约束。
    pub fn has_cps_limit(&self) -> bool {
        self.max_cps > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_policy_is_unrestricted() {
        let policy = TenantPolicy::default();
        assert!(!policy.has_concurrency_limit());
        assert!(!policy.has_cps_limit());
        assert!(policy.allows_gateway("any-gateway"));
        assert_eq!(
            policy.cross_tenant_policy,
            CrossTenantPolicy::AllowIfSameDomain
        );
    }

    #[test]
    fn gateway_allowlist_enforced() {
        let policy = TenantPolicy {
            allowed_gateway_ids: Some(vec!["gw-1".to_string(), "gw-2".to_string()]),
            ..Default::default()
        };
        assert!(policy.allows_gateway("gw-1"));
        assert!(!policy.allows_gateway("gw-3"));
    }

    #[test]
    fn concurrency_limit_detected() {
        let policy = TenantPolicy {
            max_concurrent_calls: 100,
            ..Default::default()
        };
        assert!(policy.has_concurrency_limit());
        assert!(!policy.has_cps_limit());
    }

    #[test]
    fn cps_limit_detected() {
        let policy = TenantPolicy {
            max_cps: 10,
            ..Default::default()
        };
        assert!(!policy.has_concurrency_limit());
        assert!(policy.has_cps_limit());
    }

    #[test]
    fn cross_tenant_policy_serializes_to_snake_case() {
        let json = serde_json::to_string(&CrossTenantPolicy::AllowIfSameDomain).unwrap();
        assert_eq!(json, "\"allow_if_same_domain\"");

        let deny: CrossTenantPolicy = serde_json::from_str("\"deny\"").unwrap();
        assert_eq!(deny, CrossTenantPolicy::Deny);
    }
}
