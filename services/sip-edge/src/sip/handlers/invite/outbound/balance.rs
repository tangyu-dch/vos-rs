use tracing::warn;

use crate::edge_state::PendingDatagram;
use crate::sip::response;

use super::OutboundContext;

/// 余额校验结果。
pub(super) enum BalanceCheckOutcome {
    Continue {
        calculated_max_duration: Option<u32>,
        billing_pulse: Option<(u32, f64)>,
    },
    Reject(Vec<PendingDatagram>),
}

/// 从 Redis 校验余额与费率，计算最大通话时长与计费脉冲。
pub(super) async fn enforce_balance_check(ctx: &OutboundContext<'_>) -> BalanceCheckOutcome {
    let mut calculated_max_duration: Option<u32> = ctx
        .request
        .headers
        .get("x-test-max-duration")
        .and_then(|v| v.as_str().trim().parse::<u32>().ok());
    let mut billing_pulse: Option<(u32, f64)> = None;

    // 呼叫热路径只从 Redis 读取余额和费率，不回退查询 PostgreSQL。
    if !ctx.edge_config.balance_enforcement_enabled
        || ctx.edge_config.redis_url.is_none()
        || ctx.outbound_invite.is_none()
    {
        return BalanceCheckOutcome::Continue {
            calculated_max_duration,
            billing_pulse,
        };
    }

    let callee = ctx.request.uri.user.as_deref().unwrap_or("");
    let Some(caller_user) = ctx.billing_account.as_deref() else {
        // 中继来源未绑定计费账户时拒绝呼叫，避免静默放行不计费。
        // 分机来源的 billing_account 在 resolution 阶段一定为 Some，此处 None 仅出现在中继未绑账户的场景。
        warn!("pre-call billing account is not bound to the access trunk, rejecting call");
        return BalanceCheckOutcome::Reject(vec![PendingDatagram::new(
            ctx.peer.to_string(),
            response::error_for_call_error(
                ctx.request,
                &call_core::CallError::GatewayUnavailable("中继未绑定计费账户".to_string()),
            ),
        )]);
    };
    // 从 TenantContext 提取 tenant_id，按租户查找专属费率（未命中时 redis_balance_check 会自动回退到全局）。
    let tenant_id = if ctx.tenant_ctx.is_bound() {
        ctx.tenant_ctx.tenant_id.as_deref()
    } else {
        None
    };

    match ctx
        .edge_state
        .redis_balance_check(caller_user, callee, tenant_id)
        .await
    {
        Some(check) if !check.account_found => {
            warn!(caller = %caller_user, "pre-call billing account is missing from Redis cache");
            BalanceCheckOutcome::Reject(vec![PendingDatagram::new(
                ctx.peer.to_string(),
                response::error_for_call_error(
                    ctx.request,
                    &call_core::CallError::GatewayUnavailable("计费账户未配置".to_string()),
                ),
            )])
        }
        Some(check) if !check.rate_found => {
            warn!(caller = %caller_user, callee, "pre-call billing rate is not configured");
            BalanceCheckOutcome::Reject(vec![PendingDatagram::new(
                ctx.peer.to_string(),
                response::error_for_call_error(
                    ctx.request,
                    &call_core::CallError::GatewayUnavailable("被叫号码未配置费率".to_string()),
                ),
            )])
        }
        Some(check) if !check.has_balance => {
            warn!(caller = %caller_user, balance = check.balance, credit_limit = check.credit_limit, interval = check.billing_interval_secs, price = check.price_per_interval, "pre-call Redis balance check failed");
            BalanceCheckOutcome::Reject(vec![PendingDatagram::new(
                ctx.peer.to_string(),
                response::error_for_call_error(
                    ctx.request,
                    &call_core::CallError::GatewayUnavailable("余额不足".to_string()),
                ),
            )])
        }
        Some(check) if check.price_per_interval > 0.0 => {
            billing_pulse = Some((check.billing_interval_secs, check.price_per_interval));
            calculated_max_duration = crate::billing::maximum_duration_secs(
                check.balance + check.credit_limit,
                check.billing_interval_secs,
                check.price_per_interval,
            );
            BalanceCheckOutcome::Continue {
                calculated_max_duration,
                billing_pulse,
            }
        }
        Some(_) => BalanceCheckOutcome::Continue {
            calculated_max_duration,
            billing_pulse,
        },
        None => {
            warn!(caller = %caller_user, "Redis balance check unavailable, rejecting call");
            BalanceCheckOutcome::Reject(vec![PendingDatagram::new(
                ctx.peer.to_string(),
                response::error_for_call_error(
                    ctx.request,
                    &call_core::CallError::GatewayUnavailable("计费服务暂不可用".to_string()),
                ),
            )])
        }
    }
}
