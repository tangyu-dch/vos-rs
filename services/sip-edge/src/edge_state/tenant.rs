//! # 多租户运行时策略执行
//!
//! 本模块扩展 [`EdgeState`][super::EdgeState]，将 [`TenantContext`][crate::tenant::TenantContext]
//! 与 [`TenantPolicy`][crate::tenant::policy::TenantPolicy] 落地为运行时检查：
//!
//! - **并发上限**：按 `tenant_id` 维护活跃通话计数，超限直接 486 拒绝
//! - **CPS 限额**：按 `tenant_id` 维护滑动窗口（最近 1 秒），超限直接 503 拒绝
//! - **网关白名单**：在出口网关确定后，校验 `allowed_gateway_ids`
//! - **录音覆盖**：根据 `policy.recording_enabled` 覆盖全局录音开关
//!
//! 所有方法均 O(1) 或 O(小常数)（滑动窗口长度受 `CPS_WINDOW_SECS` 限制），
//! 可安全位于 INVITE 热路径。
//!
//! ## 计数器生命周期
//!
//! - `increment_*`：在 `remember_inbound_invite` 之后调用（A-leg 已落地）
//! - `decrement_*`：在 `teardown_call_transaction` 之前调用（仍在 BYE/CANCEL/超时清理路径）
//!
//! 若 `tenant_registry` 未注入（`tenant_enabled=false`），所有检查均返回 `None` 表示放行。

use std::time::{Duration, Instant};

use crate::tenant::TenantContext;

use super::EdgeState;

/// CPS 滑动窗口长度（秒）。
const CPS_WINDOW_SECS: u64 = 1;

impl EdgeState {
    /// 检查租户并发上限，返回 `Some(reason)` 表示呼叫应被拒绝。
    ///
    /// 仅在 `tenant_registry` 已注入且 `TenantPolicy::has_concurrency_limit` 为真时生效。
    pub(crate) fn check_tenant_concurrency(&self, ctx: &TenantContext) -> Option<&'static str> {
        let tenant_id = ctx.tenant_id.as_deref()?;
        let policy = &ctx.policy;
        if !policy.has_concurrency_limit() {
            return None;
        }
        let current = self
            .tenant_concurrency
            .get(tenant_id)
            .map(|c| *c)
            .unwrap_or(0);
        if current >= policy.max_concurrent_calls {
            Some("tenant concurrency limit exceeded")
        } else {
            None
        }
    }

    /// 检查租户 CPS 上限，返回 `Some(reason)` 表示呼叫应被拒绝。
    ///
    /// 滑动窗口：保留最近 `CPS_WINDOW_SECS` 秒内的呼叫发起时间戳，
    /// 数量超过 `max_cps` 即拒绝。
    pub(crate) fn check_tenant_cps(&self, ctx: &TenantContext) -> Option<&'static str> {
        let tenant_id = ctx.tenant_id.as_deref()?;
        let policy = &ctx.policy;
        if !policy.has_cps_limit() {
            return None;
        }
        let now = Instant::now();
        let window_start = now - Duration::from_secs(CPS_WINDOW_SECS);

        let mut entry = self
            .tenant_cps_window
            .entry(tenant_id.to_string())
            .or_default();
        entry.retain(|t| *t >= window_start);
        if entry.len() >= policy.max_cps as usize {
            Some("tenant cps limit exceeded")
        } else {
            None
        }
    }

    /// 检查出口网关是否在租户白名单内，返回 `Some(reason)` 表示呼叫应被拒绝。
    ///
    /// `gateway_id` 为 `None` 表示尚未确定出口网关（如内部分机呼叫），直接放行。
    pub(crate) fn check_tenant_gateway(
        &self,
        ctx: &TenantContext,
        gateway_id: Option<&str>,
    ) -> Option<&'static str> {
        let _ = self;
        let gateway_id = gateway_id?;
        if ctx.policy.allows_gateway(gateway_id) {
            None
        } else {
            Some("tenant gateway not allowed")
        }
    }

    /// 记录一次租户呼叫发起（用于 CPS 滑动窗口）。
    ///
    /// 应在 INVITE 通过所有限额检查后、`remember_inbound_invite` 之前调用。
    pub(crate) fn record_tenant_cps(&self, ctx: &TenantContext) {
        let Some(tenant_id) = ctx.tenant_id.as_deref() else {
            return;
        };
        if !ctx.policy.has_cps_limit() {
            return;
        }
        let now = Instant::now();
        let window_start = now - Duration::from_secs(CPS_WINDOW_SECS);
        let mut entry = self
            .tenant_cps_window
            .entry(tenant_id.to_string())
            .or_default();
        entry.retain(|t| *t >= window_start);
        entry.push(now);
    }

    /// 递增租户并发计数（在 `remember_inbound_invite` 之后调用）。
    pub(crate) fn increment_tenant_concurrency(&self, ctx: &TenantContext) {
        let Some(tenant_id) = ctx.tenant_id.as_deref() else {
            return;
        };
        if !ctx.policy.has_concurrency_limit() {
            return;
        }
        self.tenant_concurrency
            .entry(tenant_id.to_string())
            .and_modify(|c| *c = c.saturating_add(1))
            .or_insert(1);
    }

    /// 递减租户并发计数（在 `teardown_call_transaction` 之前调用）。
    ///
    /// 接受 `Option<&TenantContext>` 以兼容未关联租户的会话（如 `tenant_enabled=false`）。
    pub(crate) fn decrement_tenant_concurrency(&self, ctx: Option<&TenantContext>) {
        let Some(ctx) = ctx else {
            return;
        };
        let Some(tenant_id) = ctx.tenant_id.as_deref() else {
            return;
        };
        if !ctx.policy.has_concurrency_limit() {
            return;
        }
        if let dashmap::mapref::entry::Entry::Occupied(mut entry) =
            self.tenant_concurrency.entry(tenant_id.to_string())
        {
            if *entry.get() <= 1 {
                entry.remove();
            } else {
                *entry.get_mut() -= 1;
            }
        }
    }

    /// 根据租户策略覆盖录音开关。
    ///
    /// - `policy.recording_enabled = Some(true)` → 强制开启录音
    /// - `policy.recording_enabled = Some(false)` → 强制关闭录音
    /// - `policy.recording_enabled = None` → 沿用全局 `global_recording_enabled`
    pub(crate) fn tenant_recording_override(
        &self,
        ctx: Option<&TenantContext>,
        global_recording_enabled: bool,
    ) -> bool {
        ctx.and_then(|c| c.policy.recording_enabled)
            .unwrap_or(global_recording_enabled)
    }

    /// 返回租户计费账户 ID（若策略指定了 per-tenant 计费账户）。
    ///
    /// 优先级：`TenantPolicy::billing_account_id` > per-user/per-trunk 计费账户。
    pub(crate) fn tenant_billing_account(&self, ctx: Option<&TenantContext>) -> Option<i64> {
        ctx.and_then(|c| c.policy.billing_account_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tenant::policy::{CrossTenantPolicy, TenantPolicy};
    use call_core::{routing::RouteTable, CallManager};

    fn make_state() -> EdgeState {
        let (sender, _receiver) = tokio::sync::mpsc::channel(1);
        EdgeState::new(CallManager::new(RouteTable::default(), sender))
    }

    fn make_ctx(policy: TenantPolicy) -> TenantContext {
        TenantContext {
            tenant_id: Some("t-test".to_string()),
            tenant_name: Some("Test".to_string()),
            domain: Some("test.com".to_string()),
            policy,
        }
    }

    #[test]
    fn check_tenant_concurrency_respects_limit() {
        let state = make_state();
        let policy = TenantPolicy {
            max_concurrent_calls: 2,
            ..Default::default()
        };
        let ctx = make_ctx(policy);

        assert!(state.check_tenant_concurrency(&ctx).is_none());
        state.increment_tenant_concurrency(&ctx);
        state.increment_tenant_concurrency(&ctx);
        assert!(state.check_tenant_concurrency(&ctx).is_some());
    }

    #[test]
    fn check_tenant_cps_respects_limit() {
        let state = make_state();
        let policy = TenantPolicy {
            max_cps: 2,
            ..Default::default()
        };
        let ctx = make_ctx(policy);

        // 未发起呼叫时，CPS 检查应通过
        assert!(state.check_tenant_cps(&ctx).is_none());
        state.record_tenant_cps(&ctx);
        state.record_tenant_cps(&ctx);
        // 第 3 次应被拒绝
        assert!(state.check_tenant_cps(&ctx).is_some());
    }

    #[test]
    fn check_tenant_gateway_whitelist() {
        let state = make_state();
        let policy = TenantPolicy {
            allowed_gateway_ids: Some(vec!["gw-1".to_string()]),
            ..Default::default()
        };
        let ctx = make_ctx(policy);

        assert!(state.check_tenant_gateway(&ctx, Some("gw-1")).is_none());
        assert!(state.check_tenant_gateway(&ctx, Some("gw-2")).is_some());
        assert!(state.check_tenant_gateway(&ctx, None).is_none());
    }

    #[test]
    fn unbound_context_skips_checks() {
        let state = make_state();
        let ctx = TenantContext::anonymous();
        assert!(state.check_tenant_concurrency(&ctx).is_none());
        assert!(state.check_tenant_cps(&ctx).is_none());
        state.increment_tenant_concurrency(&ctx);
        state.decrement_tenant_concurrency(Some(&ctx));
    }

    #[test]
    fn cross_tenant_policy_default_is_same_domain() {
        assert_eq!(
            CrossTenantPolicy::default(),
            CrossTenantPolicy::AllowIfSameDomain
        );
    }

    #[test]
    fn decrement_tenant_concurrency_removes_entry_at_zero() {
        let state = make_state();
        let policy = TenantPolicy {
            max_concurrent_calls: 5,
            ..Default::default()
        };
        let ctx = make_ctx(policy);
        state.increment_tenant_concurrency(&ctx);
        assert_eq!(
            state
                .tenant_concurrency
                .get("t-test")
                .map(|c| *c)
                .unwrap_or(0),
            1
        );
        state.decrement_tenant_concurrency(Some(&ctx));
        assert!(state.tenant_concurrency.get("t-test").is_none());
    }
}
