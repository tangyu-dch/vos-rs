use std::net::SocketAddr;

use sip_core::SipRequest;
use tracing::warn;

use crate::config::EdgeConfig;
use crate::edge_state::{EdgeState, PendingDatagram};
use crate::sip::registrar::RegistrationContact;
use crate::sip::response;
use crate::tenant::TenantContext;

mod balance;
mod dispatch;
mod lease;

use balance::{enforce_balance_check, BalanceCheckOutcome};
use dispatch::build_and_send_outbound;
use lease::{acquire_resource_lease, LeaseOutcome};

/// 出站分发上下文，封装从上游解析阶段累积的共享状态。
pub(super) struct OutboundContext<'a> {
    pub request: &'a SipRequest,
    pub peer: SocketAddr,
    pub edge_state: &'a EdgeState,
    pub edge_config: &'a EdgeConfig,
    pub session_id: &'a str,
    pub egress_trunk_id: &'a Option<String>,
    pub billing_account: &'a Option<String>,
    pub caller_domain: &'a Option<String>,
    pub inbound_did_destination: &'a Option<cdr_core::DidDestination>,
    pub registered_contact: &'a Option<RegistrationContact>,
    pub response: Vec<u8>,
    pub outbound_invite: Option<response::OutboundInvitePlan>,
    /// 已解析的租户上下文（来自 resolution 阶段，避免二次查询）。
    pub tenant_ctx: &'a TenantContext,
}

/// 执行计费账户设置、余额校验、资源租约、SDP 改写与出站 INVITE 分发。
pub(super) async fn dispatch_outbound_invite(mut ctx: OutboundContext<'_>) -> Vec<PendingDatagram> {
    set_billing_account(&ctx);

    if let Some(rejection) = check_gateway_domain_mismatch(&mut ctx) {
        return rejection;
    }

    apply_registered_contact_override(&mut ctx);

    let (calculated_max_duration, billing_pulse) = match enforce_balance_check(&ctx).await {
        BalanceCheckOutcome::Continue {
            calculated_max_duration,
            billing_pulse,
        } => (calculated_max_duration, billing_pulse),
        BalanceCheckOutcome::Reject(datagrams) => return datagrams,
    };

    let lease_call_id =
        match acquire_resource_lease(&mut ctx, calculated_max_duration, billing_pulse).await {
            LeaseOutcome::Acquired(call_id) => Some(call_id),
            LeaseOutcome::Rejected(datagrams) => return datagrams,
        };

    build_and_send_outbound(&ctx, calculated_max_duration, lease_call_id).await
}

/// 设置计费账户与租户上下文到 CallManager，供 CDR 与结算使用。
fn set_billing_account(ctx: &OutboundContext<'_>) {
    if ctx.outbound_invite.is_some() {
        if let Some(call_id) = ctx.request.headers.get("call-id") {
            let call_id = call_core::CallId::new(call_id.as_str());
            ctx.edge_state
                .call_manager
                .set_billing_account(&call_id, ctx.billing_account.clone());
            // 将 TenantContext 上的 tenant_id 注入到 Call，
            // 使结算阶段能按租户查找专属费率。
            let tenant_id = if ctx.tenant_ctx.is_bound() {
                ctx.tenant_ctx.tenant_id.clone()
            } else {
                None
            };
            ctx.edge_state
                .call_manager
                .set_call_tenant(&call_id, tenant_id);
        }
    }
}

/// 检查网关域是否与主叫域匹配，返回 `Some(datagrams)` 表示拒绝。
fn check_gateway_domain_mismatch(ctx: &mut OutboundContext<'_>) -> Option<Vec<PendingDatagram>> {
    if let Some(ref mut plan) = ctx.outbound_invite {
        if ctx.registered_contact.is_none() && !plan.gateway_id.is_empty() {
            if let Some(ref caller_dom) = ctx.caller_domain {
                if plan.gateway_id.contains('.') && !plan.gateway_id.contains(caller_dom) {
                    warn!(
                        gateway_id = %plan.gateway_id,
                        caller_domain = %caller_dom,
                        "tenant domain mismatch for outbound gateway"
                    );
                    return Some(vec![PendingDatagram::new(
                        ctx.peer.to_string(),
                        response::build_response_with_owned_headers(
                            ctx.request,
                            403,
                            "Forbidden - Gateway Domain Mismatch",
                            &[(
                                "X-VOS-RS-Error".to_string(),
                                "Gateway is not allowed for this tenant domain".to_string(),
                            )],
                            "",
                        ),
                    )]);
                }
            }
        }
    }
    None
}

/// 将已注册联系人的 received_from 写入出站计划的 target_override_addr。
fn apply_registered_contact_override(ctx: &mut OutboundContext<'_>) {
    if let Some(ref contact) = ctx.registered_contact {
        if let Some(ref mut plan) = ctx.outbound_invite {
            plan.target_override_addr = Some(contact.received_from.clone());
        }
    }
}
