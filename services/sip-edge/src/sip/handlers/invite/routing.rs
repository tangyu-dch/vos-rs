use std::net::SocketAddr;
use std::str::FromStr;

use call_core::{CallDirection, CallSource};
use sip_core::{SipRequest, SipUri};
use tracing::warn;

use crate::config::EdgeConfig;
use crate::edge_state::{EdgeState, PendingDatagram};
use crate::sip::registrar::RegistrationContact;
use crate::sip::response;

use super::resolution::CallResolution;

/// 路由准备阶段的结果。
pub(super) struct RoutePreparation {
    pub registered_contact: Option<RegistrationContact>,
}

/// Webhook 控制模式与 DID 目标检查。
///
/// 返回 `Ok(RoutePreparation)` 表示继续后续路由分发，返回 `Err(datagrams)` 表示被拦截或转交。
pub(super) async fn prepare_webhook_and_did(
    request: &SipRequest,
    peer: SocketAddr,
    edge_state: &EdgeState,
    edge_config: &EdgeConfig,
    from_gw: bool,
    inbound_did_destination: &Option<cdr_core::DidDestination>,
    request_number: &str,
) -> Result<RoutePreparation, Vec<PendingDatagram>> {
    if edge_config.webhooks.control_mode == "http" || edge_config.webhooks.control_mode == "nats" {
        if let Some(edge_state_arc) = edge_state.self_weak.get().and_then(|w| w.upgrade()) {
            return Err(
                crate::sip::handlers::interactive_control::handle_interactive_webhook_call(
                    request.clone(),
                    peer,
                    &edge_state_arc,
                    edge_config,
                )
                .await,
            );
        } else {
            warn!("self_weak not initialized inside handle_invite_request for VCI control; falling back to standard routing");
        }
    }

    let registered_contact = edge_state.lookup_destination_contact(&request.uri).await;

    if from_gw
        && registered_contact.is_none()
        && inbound_did_destination
            .as_ref()
            .is_some_and(|destination| destination.target_type == "extension")
    {
        warn!(
            trunk_id = "",
            did = request_number,
            "DID target extension is not registered"
        );
        return Err(vec![PendingDatagram::new(
            peer.to_string(),
            response::build_response_with_owned_headers(
                request,
                480,
                "Temporarily Unavailable - DID Extension Offline",
                &[],
                "",
            ),
        )]);
    }

    Ok(RoutePreparation { registered_contact })
}

/// 路由分发结果，包含发给主叫的响应与出站 INVITE 计划。
pub(super) struct RoutingOutcome {
    pub response: Vec<u8>,
    pub outbound_invite: Option<response::OutboundInvitePlan>,
}

/// DID/extension_group/gateway 路由分发。
///
/// 返回 `Ok(RoutingOutcome)` 表示路由成功，返回 `Err(datagrams)` 表示转交或拒绝。
pub(super) async fn dispatch_routing(
    request: &SipRequest,
    peer: SocketAddr,
    edge_state: &EdgeState,
    edge_config: &EdgeConfig,
    resolution: &CallResolution,
    registered_contact: &Option<RegistrationContact>,
) -> Result<RoutingOutcome, Vec<PendingDatagram>> {
    let source = &resolution.source;
    let call_direction = resolution.call_direction;
    let inbound_did_destination = &resolution.inbound_did_destination;

    let handling = if let Some(ref did_dest) = inbound_did_destination {
        if did_dest.target_type == "ivr" {
            return Err(crate::sip::handlers::ivr::handle_ivr_locally(
                request.clone(),
                peer,
                edge_state,
                edge_config,
                did_dest,
            )
            .await);
        } else if did_dest.target_type == "extension_group" {
            route_extension_group(request, peer, edge_state, source, call_direction, did_dest)
                .await?
        } else {
            // 标准分机
            route_standard_extension(
                request,
                edge_state,
                source,
                call_direction,
                registered_contact,
            )
        }
    } else if let Some(ref contact) = registered_contact {
        if let Ok(outbound_uri) = SipUri::from_str(&contact.uri) {
            response::response_for_invite_to_uri_with_direction(
                request,
                &edge_state.call_manager,
                outbound_uri,
                call_direction,
            )
        } else {
            response::response_for_request_with_health_and_direction(
                request,
                &edge_state.call_manager,
                Some(source),
                Some(&edge_state.gateway_health),
                call_direction,
            )
        }
    } else {
        response::response_for_request_with_health_and_direction(
            request,
            &edge_state.call_manager,
            Some(source),
            Some(&edge_state.gateway_health),
            call_direction,
        )
    };

    Ok(RoutingOutcome {
        response: handling.response,
        outbound_invite: handling.outbound_invite,
    })
}

/// 分机组路由：聚合在线成员联系人，建立 candidates 并路由到第一个联系人。
async fn route_extension_group(
    request: &SipRequest,
    peer: SocketAddr,
    edge_state: &EdgeState,
    source: &CallSource,
    call_direction: CallDirection,
    did_dest: &cdr_core::DidDestination,
) -> Result<response::RequestHandling, Vec<PendingDatagram>> {
    let members = edge_state
        .extension_groups
        .read()
        .ok()
        .and_then(|lock| lock.get(&did_dest.target_id).cloned())
        .unwrap_or_default();

    let mut group_contacts = Vec::new();
    for member in members {
        let mut member_uri = request.uri.clone();
        member_uri.user = Some(member.into());
        if let Some(contact) = edge_state.lookup_contact(&member_uri).await {
            group_contacts.push(contact);
        }
    }

    if group_contacts.is_empty() {
        warn!(group_id = %did_dest.target_id, "分机组内没有在线成员");
        return Err(vec![PendingDatagram::new(
            peer.to_string(),
            response::build_response_with_owned_headers(
                request,
                480,
                "Temporarily Unavailable - Extension Group Offline",
                &[],
                "",
            ),
        )]);
    }

    let first_contact = &group_contacts[0];
    let outbound_uri = SipUri::from_str(&first_contact.uri).map_err(|_| {
        warn!(group_id = %did_dest.target_id, "invalid group contact uri");
        vec![PendingDatagram::new(
            peer.to_string(),
            response::build_response_with_owned_headers(
                request,
                500,
                "Internal Server Error - Invalid Group Contact",
                &[],
                "",
            ),
        )]
    })?;

    let outcome = response::response_for_invite_to_uri_with_direction(
        request,
        &edge_state.call_manager,
        outbound_uri,
        call_direction,
    );

    let internal_call_id = request
        .headers
        .get("call-id")
        .map(|v| v.as_str().to_string())
        .unwrap_or_default();
    let mut candidates = Vec::new();
    for contact in group_contacts {
        if let Ok(mut outbound_uri) = SipUri::from_str(&contact.uri) {
            if let Ok(received_addr) = contact.received_from.parse::<std::net::SocketAddr>() {
                outbound_uri.host = received_addr.ip().to_string().into();
                outbound_uri.port = Some(received_addr.port());
            }
            candidates.push(call_core::SelectedRoute {
                route_id: format!("group-{}", did_dest.target_id),
                target: call_core::RouteTarget::new(
                    "extension-group-gateway",
                    outbound_uri.host.to_string(),
                    outbound_uri.port,
                ),
                outbound_uri,
            });
        }
    }
    edge_state
        .call_manager
        .set_candidates(&call_core::CallId::new(internal_call_id), candidates);

    // 抑制未使用参数告警：source 仅在 fallback 路径使用，此处由 response_for_invite_to_uri_with_direction 处理。
    let _ = source;
    Ok(outcome)
}

/// 标准分机路由：优先使用已注册联系人，否则回退到网关健康路由。
fn route_standard_extension(
    request: &SipRequest,
    edge_state: &EdgeState,
    source: &CallSource,
    call_direction: CallDirection,
    registered_contact: &Option<RegistrationContact>,
) -> response::RequestHandling {
    if let Some(ref contact) = registered_contact {
        if let Ok(outbound_uri) = SipUri::from_str(&contact.uri) {
            return response::response_for_invite_to_uri_with_direction(
                request,
                &edge_state.call_manager,
                outbound_uri,
                call_direction,
            );
        }
    }
    response::response_for_request_with_health_and_direction(
        request,
        &edge_state.call_manager,
        Some(source),
        Some(&edge_state.gateway_health),
        call_direction,
    )
}
