use tracing::{debug, info, warn};

use crate::edge_state::{ForkDialogState, PendingDatagram};
use crate::sip::outbound;
use crate::sip::response;

use super::super::super::{prepare_rewritten_sdp, register_relay_target, response_for_media_error};
use super::OutboundContext;

/// 构建改写后的 SDP，分发出站 INVITE（支持 forking 与单腿两种模式）。
pub(super) async fn build_and_send_outbound(
    ctx: &OutboundContext<'_>,
    calculated_max_duration: Option<u32>,
    lease_call_id: Option<call_core::CallId>,
) -> Vec<PendingDatagram> {
    let Some(outbound_invite) = ctx.outbound_invite.as_ref() else {
        return vec![PendingDatagram::new(
            ctx.peer.to_string(),
            ctx.response.clone(),
        )];
    };

    let rewritten_sdp = match prepare_rewritten_sdp(
        &ctx.request.headers,
        &ctx.request.body,
        &ctx.edge_state.media_relay,
        &ctx.edge_config.media,
        "inbound INVITE offer",
        ctx.session_id,
    ) {
        Ok(rewritten_sdp) => rewritten_sdp,
        Err(error) => {
            warn!(%error, "rejecting INVITE after media negotiation failure");
            if let Some(call_id) = lease_call_id.as_ref() {
                ctx.edge_state
                    .call_manager
                    .terminate_call_with_reason(call_id.as_str(), &error.to_string());
                crate::resource_lease::release(ctx.edge_state, call_id);
            }
            return vec![PendingDatagram::new(
                ctx.peer.to_string(),
                response_for_media_error(ctx.request, &error),
            )];
        }
    };
    if let Some(rewritten_sdp) = &rewritten_sdp {
        if let Some(caller_rtp) = &rewritten_sdp.original_endpoint {
            register_relay_target(
                &ctx.edge_state.media_relay,
                &rewritten_sdp.relay_endpoint,
                caller_rtp,
                "gateway-to-caller RTP",
            );
        }
    }

    let internal_call_id = ctx
        .request
        .headers
        .get("call-id")
        .map(|v| v.as_str().to_string())
        .unwrap_or_default();

    let mut candidates = Vec::new();
    if let Some(call) = ctx
        .edge_state
        .call_manager
        .get(&call_core::CallId::new(internal_call_id.clone()))
    {
        candidates = call.candidates.clone();
    }

    ctx.edge_state.remember_inbound_invite(
        ctx.session_id.to_string(),
        ctx.request,
        ctx.peer,
        outbound_invite.outbound_uri.clone(),
        rewritten_sdp
            .as_ref()
            .and_then(|sdp| sdp.original_endpoint.clone()),
        rewritten_sdp.as_ref().map(|sdp| sdp.relay_endpoint.clone()),
        calculated_max_duration,
    );

    let mut datagrams = vec![PendingDatagram::new(
        ctx.peer.to_string(),
        ctx.response.clone(),
    )];
    let path = if let Some(ref contact) = ctx.registered_contact {
        contact.path.as_slice()
    } else {
        &[]
    };

    let forking_enabled = ctx
        .request
        .headers
        .get("x-forking-enabled")
        .map(|v| v.as_str().trim().to_lowercase() == "true")
        .unwrap_or(false)
        || ctx
            .request
            .headers
            .get("x-call-forking")
            .map(|v| v.as_str().trim().to_lowercase() == "true")
            .unwrap_or(false)
        || ctx
            .inbound_did_destination
            .as_ref()
            .is_some_and(|d| d.target_type == "extension_group");

    let managed_resources = crate::resource_lease::requires_single_leg(
        ctx.edge_state,
        &call_core::CallId::new(internal_call_id.clone()),
    );
    if forking_enabled && candidates.len() > 1 && !managed_resources {
        fork_outbound_invites(
            ctx,
            outbound_invite,
            &rewritten_sdp,
            &candidates,
            path,
            &mut datagrams,
        );
    } else {
        send_single_outbound_invite(
            ctx,
            outbound_invite,
            &rewritten_sdp,
            &internal_call_id,
            path,
            &mut datagrams,
        );
    }

    datagrams
}

/// Fork 模式：向多个候选目标并行发送出站 INVITE（最多 3 个）。
fn fork_outbound_invites(
    ctx: &OutboundContext<'_>,
    outbound_invite: &response::OutboundInvitePlan,
    rewritten_sdp: &Option<super::super::super::RewrittenSdp>,
    candidates: &[call_core::SelectedRoute],
    path: &[String],
    datagrams: &mut Vec<PendingDatagram>,
) {
    let fork_candidates = candidates.iter().take(3).cloned().collect::<Vec<_>>();
    let Some(gateway_dialog_template) = ctx
        .edge_state
        .inbound_transactions
        .get(ctx.session_id)
        .map(|transaction| transaction.dialogs.gateway.clone())
    else {
        warn!(
            session_id = ctx.session_id,
            "fork session disappeared before candidate setup"
        );
        return;
    };

    for candidate in &fork_candidates {
        let external_call_id = uuid::Uuid::new_v4().to_string();
        let target = outbound::target_addr_for(&candidate.outbound_uri);
        let gateway_local_tag = format!("vosrs-b-{}", uuid::Uuid::new_v4().simple());
        let mut fork_dialog = gateway_dialog_template.clone();
        fork_dialog.call_id = external_call_id.clone();
        fork_dialog.local_tag = gateway_local_tag.clone();
        fork_dialog.remote_uri = candidate.outbound_uri.clone();
        fork_dialog.remote_tag = None;
        fork_dialog.local_cseq = 1;
        fork_dialog.remote_cseq = None;
        fork_dialog.route_set = path.to_vec();
        fork_dialog.remote_target = candidate.outbound_uri.clone();
        fork_dialog.peer = Some(target.clone());
        let gateway_id = candidate.target.gateway_id.as_str().to_string();
        if !ctx.edge_state.inbound_transactions.insert_fork_dialog(
            ctx.session_id,
            ForkDialogState {
                dialog: fork_dialog,
                gateway_id: gateway_id.clone(),
            },
        ) {
            warn!(
                session_id = ctx.session_id,
                external_call_id, "failed to register fork dialog"
            );
            continue;
        }
        let bytes = outbound::build_b2bua_outbound_invite(
            ctx.request,
            &candidate.outbound_uri,
            &ctx.edge_config.advertised_addr,
            rewritten_sdp
                .as_ref()
                .map(|sdp| sdp.body.as_slice())
                .unwrap_or(ctx.request.body.as_ref()),
            ctx.edge_config.session_expires_gateway,
            path,
            &external_call_id,
            &gateway_local_tag,
            outbound_invite.caller_identity.as_ref(),
        );
        datagrams.push(PendingDatagram::new(target, bytes));

        if !gateway_id.is_empty() {
            ctx.edge_state.gateway_health.increment_active(&gateway_id);
            let status = ctx
                .edge_state
                .gateway_health
                .get_gateway_status(&gateway_id);
            crate::timers::persist_gateway_health(ctx.edge_state, gateway_id.clone(), status);
        }
    }
}

/// 单腿模式：向唯一目标发送出站 INVITE，处理拓扑隐藏 Call-ID 映射。
fn send_single_outbound_invite(
    ctx: &OutboundContext<'_>,
    outbound_invite: &response::OutboundInvitePlan,
    rewritten_sdp: &Option<super::super::super::RewrittenSdp>,
    internal_call_id: &str,
    path: &[String],
    datagrams: &mut Vec<PendingDatagram>,
) {
    let external_call_id = uuid::Uuid::new_v4().to_string();
    ctx.edge_state
        .bind_gateway_dialog(ctx.session_id, &external_call_id);
    let gateway_local_tag = ctx
        .edge_state
        .inbound_transactions
        .get(internal_call_id)
        .map(|transaction| transaction.dialogs.gateway.local_tag.clone())
        .unwrap_or_else(|| format!("vosrs-b-{}", uuid::Uuid::new_v4().simple()));
    debug!(
        internal_call_id,
        external_call_id, "topology hiding: registered Call-ID mapping"
    );

    let target = if let Some(ref contact) = ctx.registered_contact {
        contact.received_from.clone()
    } else if let Some(ref override_addr) = outbound_invite.target_override_addr {
        override_addr.clone()
    } else {
        outbound::target_addr_for(&outbound_invite.outbound_uri)
    };
    info!(
        internal_call_id,
        external_call_id,
        gateway_id = %outbound_invite.gateway_id,
        outbound_uri = %outbound_invite.outbound_uri,
        target = %target,
        "bench: sending outbound INVITE to gateway"
    );

    let bytes = outbound::build_b2bua_outbound_invite(
        ctx.request,
        &outbound_invite.outbound_uri,
        &ctx.edge_config.advertised_addr,
        rewritten_sdp
            .as_ref()
            .map(|sdp| sdp.body.as_slice())
            .unwrap_or(ctx.request.body.as_ref()),
        ctx.edge_config.session_expires_gateway,
        path,
        &external_call_id,
        &gateway_local_tag,
        outbound_invite.caller_identity.as_ref(),
    );
    datagrams.push(PendingDatagram::new(target, bytes));

    if !outbound_invite.gateway_id.is_empty() {
        ctx.edge_state
            .gateway_health
            .increment_active(&outbound_invite.gateway_id);
        let status = ctx
            .edge_state
            .gateway_health
            .get_gateway_status(&outbound_invite.gateway_id);
        crate::timers::persist_gateway_health(
            ctx.edge_state,
            outbound_invite.gateway_id.clone(),
            status,
        );
    }
}
