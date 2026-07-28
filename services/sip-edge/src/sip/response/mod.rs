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

mod builders;
mod handling;
mod headers;
mod invite;

pub(crate) use builders::build_inbound_leg_response;
#[allow(unused_imports)]
pub use builders::build_response_with_owned_headers_and_peer;
pub use builders::{
    accepted_202_for_request, build_response_with_owned_headers, error_for_call_error,
    not_acceptable_for_request, ok_for_request, response_100_trying,
    response_503_service_unavailable, service_unavailable_for_request,
};
pub use handling::{OutboundInvitePlan, RequestHandling};
#[allow(unused_imports)]
pub use headers::patch_via_rport_and_received;
pub use invite::{
    response_for_invite_to_uri_with_direction, response_for_request_with_health,
    response_for_request_with_health_and_direction,
};

pub(super) const SERVER_HEADER: &str = "VOS-RS sip-edge/0.1";
/// Edge 在 caller leg 上使用的本地 tag，用于 To 头自动填充与 in-dialog 请求校验。
///
/// 该常量同时被 `append_to_header`（自动填充 To tag）与 `EdgeState::remember_inbound_invite`
/// （初始化 `DialogLegState::local_tag`）引用，确保两条路径产生一致的 To tag。
pub(crate) const EDGE_TAG: &str = "vosrs-edge";

#[cfg(test)]
mod tests {
    use super::{
        accepted_202_for_request, builders::build_response, response_for_request_with_health,
    };
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
        let handling = response_for_request_with_health(&request, &call_manager, None, None);
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
