use tracing::warn;

use crate::edge_state::PendingDatagram;
use crate::sip::response;

use super::OutboundContext;

/// 资源租约结果。
pub(super) enum LeaseOutcome {
    Acquired(call_core::CallId),
    Rejected(Vec<PendingDatagram>),
}

/// 申请资源租约，处理 caller pool failover。
pub(super) async fn acquire_resource_lease(
    ctx: &mut OutboundContext<'_>,
    calculated_max_duration: Option<u32>,
    billing_pulse: Option<(u32, f64)>,
) -> LeaseOutcome {
    let Some(call_id) = ctx
        .request
        .headers
        .get("call-id")
        .map(|value| call_core::CallId::new(value.as_str()))
        .filter(|_| ctx.outbound_invite.is_some())
    else {
        return LeaseOutcome::Acquired(call_core::CallId::new(""));
    };

    ctx.edge_state.call_manager.set_cdr_audit_context(
        &call_id,
        ctx.egress_trunk_id.clone(),
        billing_pulse.map(|pulse| pulse.0),
        billing_pulse.map(|pulse| pulse.1),
    );

    let lease_error = loop {
        match crate::resource_lease::acquire(ctx.edge_state, &call_id, calculated_max_duration)
            .await
        {
            Ok(()) => break None,
            Err(
                error @ (crate::resource_lease::LeaseError::NumberBusy
                | crate::resource_lease::LeaseError::TrunkAtCapacity),
            ) => {
                let Some(next) = ctx.edge_state.call_manager.advance_caller_pool(&call_id) else {
                    break Some(error);
                };
                if let Some(plan) = ctx.outbound_invite.as_mut() {
                    plan.outbound_uri = next.outbound_uri;
                    plan.gateway_id = next
                        .caller_identity
                        .as_ref()
                        .map(|identity| identity.owner_gateway_id.as_str().to_string())
                        .unwrap_or_default();
                    plan.caller_identity = next.caller_identity;
                    plan.target_override_addr = None;
                }
                warn!(
                    call_id = %call_id.as_str(),
                    gateway_id = %ctx.outbound_invite.as_ref().map(|plan| plan.gateway_id.as_str()).unwrap_or(""),
                    reason = %error,
                    "caller pool member resources at capacity, trying next member"
                );
            }
            Err(error) => break Some(error),
        }
    };

    if let Some(error) = lease_error {
        warn!(call_id = %call_id.as_str(), %error, "outbound call resource lease rejected");
        ctx.edge_state
            .call_manager
            .terminate_call_with_reason(call_id.as_str(), &error.to_string());
        return LeaseOutcome::Rejected(vec![PendingDatagram::new(
            ctx.peer.to_string(),
            response::error_for_call_error(
                ctx.request,
                &call_core::CallError::GatewayUnavailable(error.to_string()),
            ),
        )]);
    }

    LeaseOutcome::Acquired(call_id)
}
