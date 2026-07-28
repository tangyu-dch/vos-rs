use call_core::{CallDirection, CallError, CallManager, CallSource, GatewayHealthTracker};
use sip_core::{Method, SipRequest, SipUri};

use super::builders::build_response;
use super::handling::{OutboundInvitePlan, RequestHandling};

pub fn response_for_request_with_health(
    request: &SipRequest,
    call_manager: &CallManager,
    source: Option<&CallSource>,
    health: Option<&GatewayHealthTracker>,
) -> RequestHandling {
    response_for_request_with_health_and_direction(
        request,
        call_manager,
        source,
        health,
        CallDirection::Outbound,
    )
}

/// 处理请求，并使用接入识别阶段确定的可信业务方向。
pub fn response_for_request_with_health_and_direction(
    request: &SipRequest,
    call_manager: &CallManager,
    source: Option<&CallSource>,
    health: Option<&GatewayHealthTracker>,
    direction: CallDirection,
) -> RequestHandling {
    match &request.method {
        Method::Options => build_response(
            request,
            200,
            "OK",
            &[("Allow", "REGISTER, INVITE, ACK, BYE, CANCEL, OPTIONS, INFO")],
            "",
        )
        .into(),
        Method::Invite => {
            response_for_invite_with_source(request, call_manager, source, health, direction)
        }
        _ => build_response(request, 501, "Not Implemented", &[], "").into(),
    }
}

fn response_for_invite_with_source(
    request: &SipRequest,
    call_manager: &CallManager,
    source: Option<&CallSource>,
    health: Option<&GatewayHealthTracker>,
    direction: CallDirection,
) -> RequestHandling {
    match call_manager.handle_inbound_invite_with_source_and_health_and_direction(
        request, source, health, direction,
    ) {
        Ok(outcome) => {
            let gateway_id = call_manager
                .current_gateway_id(outcome.call_id.as_str())
                .unwrap_or_default();
            RequestHandling {
                response: build_response(request, 100, "Trying", &[], ""),
                outbound_invite: Some(OutboundInvitePlan {
                    outbound_uri: outcome.outbound_uri,
                    target_override_addr: None,
                    gateway_id,
                    caller_identity: outcome.caller_identity,
                }),
            }
        }
        Err(error) => {
            let (status_code, reason_phrase) = invite_error_status(&error);
            let error_header = error.to_string();
            build_response(
                request,
                status_code,
                reason_phrase,
                &[("X-VOS-RS-Error", error_header.as_str())],
                "",
            )
            .into()
        }
    }
}

/// 将 INVITE 发送到预选 URI，并保留可信业务方向。
pub fn response_for_invite_to_uri_with_direction(
    request: &SipRequest,
    call_manager: &CallManager,
    outbound_uri: SipUri,
    direction: CallDirection,
) -> RequestHandling {
    match call_manager.handle_inbound_invite_to_uri_with_direction(request, outbound_uri, direction)
    {
        Ok(outcome) => RequestHandling {
            response: build_response(request, 100, "Trying", &[], ""),
            outbound_invite: Some(OutboundInvitePlan {
                outbound_uri: outcome.outbound_uri,
                target_override_addr: None,
                gateway_id: String::new(),
                caller_identity: None,
            }),
        },
        Err(error) => {
            let (status_code, reason_phrase) = invite_error_status(&error);
            let error_header = error.to_string();
            build_response(
                request,
                status_code,
                reason_phrase,
                &[("X-VOS-RS-Error", error_header.as_str())],
                "",
            )
            .into()
        }
    }
}

pub(super) fn invite_error_status(error: &CallError) -> (u16, &'static str) {
    match error {
        CallError::MissingRequiredHeader(_) | CallError::InvalidDestinationUri => {
            (400, "Bad Request")
        }
        CallError::NoRouteForDestination(_) => (404, "Not Found"),
        CallError::GatewayUnavailable(_) => (503, "Service Unavailable"),
        CallError::CallerIdentityUnavailable(_) => (403, "Forbidden"),
        CallError::UnknownCall(_) => (481, "Call/Transaction Does Not Exist"),
        CallError::WebhookRoutingError(_) => (502, "Bad Gateway"),
        CallError::InvalidTransition { .. }
        | CallError::OutboundLegAlreadyExists
        | CallError::MissingOutboundLeg => (500, "Internal Server Error"),
    }
}
