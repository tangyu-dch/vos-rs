//! # SIP 响应构建
//!
//! 本模块负责构建 SIP 响应消息，包括：
//!
//! - **100 Trying**：临时响应，防止重传
//! - **180 Ringing**：振铃通知
//! - **200 OK**：成功响应
//! - **4xx/5xx**：错误响应
//!
//! ## 请求处理流程
//!
//! ```text
//! 入站 INVITE → 路由选择 → 构建 100 Trying → 构建出站 INVITE → 返回给主叫
//! ```
//!
//! ## 路由选择
//!
//! 使用 `CallManager::handle_inbound_invite_with_health` 选择路由：
//! - 检查网关健康状态（Circuit Breaker）
//! - 检查网关容量
//! - 应用前缀规则和 Caller ID 重写

use call_core::{CallError, CallManager, CallerIdentity, GatewayHealthTracker};
use sip_core::{HeaderMap, Method, SipRequest, SipResponse, SipUri};

const SERVER_HEADER: &str = "VOS-RS sip-edge/0.1";
const EDGE_TAG: &str = "vosrs-edge";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestHandling {
    pub response: Vec<u8>,
    pub outbound_invite: Option<OutboundInvitePlan>,
}

impl RequestHandling {
    fn response(response: Vec<u8>) -> Self {
        Self {
            response,
            outbound_invite: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboundInvitePlan {
    pub outbound_uri: SipUri,
    pub target_override_addr: Option<String>,
    pub gateway_id: String,
    pub caller_identity: Option<CallerIdentity>,
}

pub fn response_for_request_with_health(
    request: &SipRequest,
    call_manager: &CallManager,
    health: Option<&GatewayHealthTracker>,
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
        Method::Invite => response_for_invite(request, call_manager, health),
        _ => build_response(request, 501, "Not Implemented", &[], "").into(),
    }
}

fn response_for_invite(
    request: &SipRequest,
    call_manager: &CallManager,
    health: Option<&GatewayHealthTracker>,
) -> RequestHandling {
    match call_manager.handle_inbound_invite_with_health(request, health) {
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

pub fn response_for_invite_to_uri(
    request: &SipRequest,
    call_manager: &CallManager,
    outbound_uri: SipUri,
) -> RequestHandling {
    match call_manager.handle_inbound_invite_to_uri(request, outbound_uri) {
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

pub fn ok_for_request(request: &SipRequest) -> Vec<u8> {
    build_response(request, 200, "OK", &[], "")
}

pub fn accepted_202_for_request(request: &SipRequest) -> Vec<u8> {
    build_response(request, 202, "Accepted", &[], "")
}

pub fn not_acceptable_for_request(request: &SipRequest, reason: &str) -> Vec<u8> {
    build_response(
        request,
        488,
        "Not Acceptable Here",
        &[("X-VOS-RS-Error", reason)],
        "",
    )
}

pub fn service_unavailable_for_request(request: &SipRequest, reason: &str) -> Vec<u8> {
    build_response(
        request,
        503,
        "Service Unavailable",
        &[("X-VOS-RS-Error", reason)],
        "",
    )
}

pub fn error_for_call_error(request: &SipRequest, error: &CallError) -> Vec<u8> {
    let (status_code, reason_phrase) = invite_error_status(error);
    let error_header = error.to_string();
    build_response(
        request,
        status_code,
        reason_phrase,
        &[("X-VOS-RS-Error", error_header.as_str())],
        "",
    )
}

impl From<Vec<u8>> for RequestHandling {
    fn from(response: Vec<u8>) -> Self {
        Self::response(response)
    }
}

fn invite_error_status(error: &CallError) -> (u16, &'static str) {
    match error {
        CallError::MissingRequiredHeader(_) | CallError::InvalidDestinationUri => {
            (400, "Bad Request")
        }
        CallError::NoRouteForDestination(_) => (404, "Not Found"),
        CallError::GatewayUnavailable(_) => (503, "Service Unavailable"),
        CallError::CallerIdentityUnavailable(_) => (403, "Forbidden"),
        CallError::UnknownCall(_) => (481, "Call/Transaction Does Not Exist"),
        CallError::InvalidTransition { .. }
        | CallError::OutboundLegAlreadyExists
        | CallError::MissingOutboundLeg => (500, "Internal Server Error"),
    }
}

fn build_response(
    request: &sip_core::SipRequestBorrow<'_>,
    status_code: u16,
    reason_phrase: &str,
    extra_headers: &[(&str, &str)],
    body: &str,
) -> Vec<u8> {
    let extra_headers = extra_headers
        .iter()
        .map(|(name, value)| ((*name).to_string(), (*value).to_string()))
        .collect::<Vec<_>>();
    build_response_with_owned_headers(request, status_code, reason_phrase, &extra_headers, body)
}

pub fn response_503_service_unavailable(request: &sip_core::SipRequestBorrow<'_>) -> Vec<u8> {
    build_response(
        request,
        503,
        "Service Unavailable",
        &[("Retry-After", "30")],
        "",
    )
}

pub fn build_response_with_owned_headers(
    request: &sip_core::SipRequestBorrow<'_>,
    status_code: u16,
    reason_phrase: &str,
    extra_headers: &[(String, String)],
    body: &str,
) -> Vec<u8> {
    let mut response = String::new();
    response.push_str(&format!("SIP/2.0 {status_code} {reason_phrase}\r\n"));

    append_all_headers(&mut response, &request.headers, "via", "Via");
    append_single_header(&mut response, &request.headers, "from", "From");
    append_to_header(&mut response, &request.headers, status_code);
    append_single_header(&mut response, &request.headers, "call-id", "Call-ID");
    append_single_header(&mut response, &request.headers, "cseq", "CSeq");
    append_all_headers(
        &mut response,
        &request.headers,
        "record-route",
        "Record-Route",
    );

    response.push_str(&format!("Server: {SERVER_HEADER}\r\n"));
    for (name, value) in extra_headers {
        response.push_str(name.as_str());
        response.push_str(": ");
        response.push_str(value.as_str());
        response.push_str("\r\n");
    }

    response.push_str(&format!("Content-Length: {}\r\n", body.len()));
    response.push_str("\r\n");
    response.push_str(body);
    response.into_bytes()
}

#[allow(dead_code)]
pub fn forward_response_to_inbound_with_body(
    response: &SipResponse,
    inbound_vias: &[String],
    inbound_route_set: &[String],
    body: &[u8],
) -> Vec<u8> {
    forward_response_to_inbound_with_body_and_call_id(
        response,
        inbound_vias,
        inbound_route_set,
        body,
        None,
    )
}

/// Topology-hiding variant: the caller-facing response uses `override_call_id` instead
/// of the gateway's external Call-ID.
pub fn forward_response_to_inbound_with_body_and_call_id(
    response: &SipResponse,
    inbound_vias: &[String],
    inbound_route_set: &[String],
    body: &[u8],
    override_call_id: Option<&str>,
) -> Vec<u8> {
    let mut forwarded = String::new();
    forwarded.push_str(&format!(
        "SIP/2.0 {} {}\r\n",
        response.status_code, response.reason_phrase
    ));

    for via in inbound_vias {
        forwarded.push_str("Via: ");
        forwarded.push_str(via);
        forwarded.push_str("\r\n");
    }

    for route in inbound_route_set {
        forwarded.push_str("Record-Route: ");
        forwarded.push_str(route);
        forwarded.push_str("\r\n");
    }

    append_single_header(&mut forwarded, &response.headers, "from", "From");
    append_single_header(&mut forwarded, &response.headers, "to", "To");
    // Topology Hiding: use the override (internal) Call-ID when present so the caller
    // never sees the external Call-ID that was forwarded to the gateway.
    if let Some(cid) = override_call_id {
        forwarded.push_str("Call-ID: ");
        forwarded.push_str(cid);
        forwarded.push_str("\r\n");
    } else {
        append_single_header(&mut forwarded, &response.headers, "call-id", "Call-ID");
    }
    append_single_header(&mut forwarded, &response.headers, "cseq", "CSeq");

    // RFC 3262: pass through 100rel negotiation headers in provisional responses
    append_single_header(&mut forwarded, &response.headers, "require", "Require");
    append_single_header(&mut forwarded, &response.headers, "rseq", "RSeq");

    // RFC 4028: pass through session timer negotiation headers
    append_single_header(
        &mut forwarded,
        &response.headers,
        "session-expires",
        "Session-Expires",
    );
    append_single_header(&mut forwarded, &response.headers, "min-se", "Min-SE");
    append_single_header(&mut forwarded, &response.headers, "supported", "Supported");

    if !body.is_empty() {
        append_single_header(
            &mut forwarded,
            &response.headers,
            "content-type",
            "Content-Type",
        );
    }

    forwarded.push_str(&format!("Content-Length: {}\r\n", body.len()));
    forwarded.push_str("\r\n");

    let mut bytes = forwarded.into_bytes();
    bytes.extend_from_slice(body);
    bytes
}

fn append_all_headers(
    response: &mut String,
    headers: &HeaderMap,
    lookup_name: &str,
    output_name: &str,
) {
    for value in headers.get_all(lookup_name) {
        response.push_str(output_name);
        response.push_str(": ");
        response.push_str(value.as_str());
        response.push_str("\r\n");
    }
}

fn append_single_header(
    response: &mut String,
    headers: &HeaderMap,
    lookup_name: &str,
    output_name: &str,
) {
    if let Some(value) = headers.get(lookup_name) {
        response.push_str(output_name);
        response.push_str(": ");
        response.push_str(value.as_str());
        response.push_str("\r\n");
    }
}

fn append_to_header(response: &mut String, headers: &HeaderMap, status_code: u16) {
    if let Some(value) = headers.get("to") {
        response.push_str("To: ");
        response.push_str(value.as_str());
        // RFC 3261 8.2.6.2: a 100 (Trying) response must copy the request To
        // header without adding a tag. A premature tag also makes SIP clients
        // treat the following gateway response as belonging to another dialog.
        if status_code != 100 && !value.as_str().to_ascii_lowercase().contains(";tag=") {
            response.push_str(";tag=");
            response.push_str(EDGE_TAG);
        }
        response.push_str("\r\n");
    }
}

#[cfg(test)]
mod tests {
    use super::{accepted_202_for_request, build_response, response_for_request_with_health};
    use call_core::{CallManager, RouteTable};
    use sip_core::{parse_message, SipMessage};

    #[test]
    fn unsupported_methods_receive_501() {
        let request = concat!(
            "MESSAGE sip:edge.example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-1\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:1001@example.com>\r\n",
            "Call-ID: message-1@example.com\r\n",
            "CSeq: 1 MESSAGE\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        );

        let SipMessage::Request(request) = parse_message(request.as_bytes()).unwrap() else {
            panic!("expected request");
        };

        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let call_manager = CallManager::new(RouteTable::default(), tx);
        let handling = response_for_request_with_health(&request, &call_manager, None);
        let response = String::from_utf8(handling.response.clone()).unwrap();

        assert!(response.starts_with("SIP/2.0 501 Not Implemented\r\n"));
        assert!(response.contains("CSeq: 1 MESSAGE\r\n"));
        assert!(handling.outbound_invite.is_none());
    }

    #[test]
    fn trying_response_does_not_add_to_tag() {
        let request = concat!(
            "INVITE sip:13800138000@example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-trying\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:13800138000@example.com>\r\n",
            "Call-ID: trying-1@example.com\r\n",
            "CSeq: 1 INVITE\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        );

        let SipMessage::Request(request) = parse_message(request.as_bytes()).unwrap() else {
            panic!("expected request");
        };
        let response = String::from_utf8(build_response(&request, 100, "Trying", &[], "")).unwrap();

        assert!(response.starts_with("SIP/2.0 100 Trying\r\n"));
        assert!(response.contains("To: <sip:13800138000@example.com>\r\n"));
        assert!(!response.contains("To: <sip:13800138000@example.com>;tag="));
    }

    #[test]
    fn builds_202_accepted_for_refer() {
        let request = concat!(
            "REFER sip:edge.example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-refer\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:13800138000@example.com>;tag=to-tag\r\n",
            "Call-ID: refer-1@example.com\r\n",
            "CSeq: 3 REFER\r\n",
            "Refer-To: <sip:1002@example.com>\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        );

        let SipMessage::Request(request) = parse_message(request.as_bytes()).unwrap() else {
            panic!("expected request");
        };

        let response = String::from_utf8(accepted_202_for_request(&request)).unwrap();

        assert!(response.starts_with("SIP/2.0 202 Accepted\r\n"));
        assert!(response.contains("CSeq: 3 REFER\r\n"));
        assert!(response.contains("Content-Length: 0\r\n\r\n"));
    }
}
