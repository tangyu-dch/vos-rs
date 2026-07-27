//! 出站 INVITE 构建：初始 INVITE、转移 INVITE 及通用请求模板。
//!
//! ## Topology Hiding
//!
//! 出站 INVITE 使用独立的 Call-ID（external_call_id），
//! 隐藏内部拓扑信息，防止外部网关探测内部网络结构。

use call_core::CallerIdentity;
use sip_core::{SipRequest, SipUri};

use super::helpers::{
    append_caller_identity_headers, append_header_with_tag, append_single_header, branch_for,
    next_max_forwards,
};
use crate::edge_state::DialogLegState;

/// Builds the initial INVITE owned by the gateway dialog leg.
#[allow(clippy::too_many_arguments)]
pub fn build_b2bua_outbound_invite(
    inbound: &SipRequest,
    outbound_uri: &SipUri,
    advertised_addr: &str,
    body: &[u8],
    session_expires: u32,
    route_set: &[String],
    gateway_call_id: &str,
    gateway_local_tag: &str,
    caller_identity: Option<&CallerIdentity>,
) -> Vec<u8> {
    let extra_headers = format!(
        "Supported: timer,100rel\r\nSession-Expires: {session_expires};refresher=uac\r\nMin-SE: 90\r\n"
    );
    build_outbound_request_with_extra(
        inbound,
        outbound_uri,
        advertised_addr,
        route_set,
        body,
        &extra_headers,
        Some(gateway_call_id),
        caller_identity,
        Some(gateway_local_tag),
    )
}

pub(crate) fn build_transfer_invite(
    dialog: &DialogLegState,
    advertised_addr: &str,
    sdp_body: &[u8],
    replaces: Option<&str>,
) -> Vec<u8> {
    let call_id = &dialog.call_id;
    let cseq = dialog.local_cseq;
    let branch = format!(
        "z9hG4bK-transfer-{}-{}",
        super::helpers::token_fragment(call_id),
        cseq
    );
    let replaces_header = replaces
        .map(|val| format!("Replaces: {}\r\n", val))
        .unwrap_or_default();
    let from = format!("<{}>;tag={}", dialog.local_uri, dialog.local_tag);
    let to = dialog
        .remote_tag
        .as_ref()
        .map(|tag| format!("<{}>;tag={tag}", dialog.remote_uri))
        .unwrap_or_else(|| format!("<{}>", dialog.remote_uri));
    let request = format!(
        "INVITE {uri} SIP/2.0\r\n\
         Via: SIP/2.0/UDP {addr};branch={branch}\r\n\
         Max-Forwards: 70\r\n\
         From: {from}\r\n\
         To: {to}\r\n\
         Call-ID: {call_id}\r\n\
         CSeq: {cseq} INVITE\r\n\
         Contact: <sip:vosrs@{addr}>\r\n\
         {replaces_hdr}\
         Content-Type: application/sdp\r\n\
         Content-Length: {body_len}\r\n\r\n",
        uri = dialog.remote_target,
        addr = advertised_addr,
        branch = branch,
        from = from,
        to = to,
        call_id = call_id,
        cseq = cseq,
        replaces_hdr = replaces_header,
        body_len = sdp_body.len()
    );
    let mut bytes = request.into_bytes();
    bytes.extend_from_slice(sdp_body);
    bytes
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_outbound_request_with_extra(
    inbound: &SipRequest,
    outbound_uri: &SipUri,
    advertised_addr: &str,
    route_set: &[String],
    body: &[u8],
    extra_headers: &str,
    override_call_id: Option<&str>,
    caller_identity: Option<&CallerIdentity>,
    local_from_tag: Option<&str>,
) -> Vec<u8> {
    let mut request = String::new();
    request.push_str(inbound.method.as_str());
    request.push(' ');
    request.push_str(&outbound_uri.to_string());
    request.push_str(" SIP/2.0\r\n");

    // Topology Hiding: emit only a single clean Via pointing at our public address.
    // All inbound Via headers from the original sender are deliberately stripped.
    request.push_str("Via: SIP/2.0/UDP ");
    request.push_str(advertised_addr);
    request.push_str(";branch=");
    request.push_str(&branch_for(inbound));
    request.push_str("\r\n");

    request.push_str("Max-Forwards: ");
    request.push_str(&next_max_forwards(&inbound.headers).to_string());
    request.push_str("\r\n");

    // Topology Hiding: Route headers are used for proxy path, but we intentionally
    // do NOT forward internal Record-Route headers from the inbound message.
    for route in route_set {
        request.push_str("Route: ");
        request.push_str(route);
        request.push_str("\r\n");
    }

    if let Some(identity) = caller_identity {
        append_caller_identity_headers(
            &mut request,
            inbound,
            advertised_addr,
            identity,
            local_from_tag,
        );
    } else if let Some(local_tag) = local_from_tag {
        append_header_with_tag(&mut request, &inbound.headers, "from", "From", local_tag);
    } else {
        append_single_header(&mut request, &inbound.headers, "from", "From");
    }
    append_single_header(&mut request, &inbound.headers, "to", "To");
    // Topology Hiding: use the override Call-ID for the outbound leg if provided.
    if let Some(cid) = override_call_id {
        request.push_str("Call-ID: ");
        request.push_str(cid);
        request.push_str("\r\n");
    } else {
        append_single_header(&mut request, &inbound.headers, "call-id", "Call-ID");
    }
    append_single_header(&mut request, &inbound.headers, "cseq", "CSeq");
    append_single_header(&mut request, &inbound.headers, "refer-to", "Refer-To");
    append_single_header(&mut request, &inbound.headers, "referred-by", "Referred-By");
    append_single_header(&mut request, &inbound.headers, "refer-sub", "Refer-Sub");

    request.push_str("Contact: <sip:vosrs@");
    request.push_str(advertised_addr);
    request.push_str(">\r\n");

    // Inject any extra headers (e.g. Session-Expires, Supported: timer)
    if !extra_headers.is_empty() {
        request.push_str(extra_headers);
    }

    if !body.is_empty() {
        append_single_header(
            &mut request,
            &inbound.headers,
            "content-type",
            "Content-Type",
        );
    }

    request.push_str(&format!("Content-Length: {}\r\n", body.len()));
    request.push_str("\r\n");

    let mut bytes = request.into_bytes();
    bytes.extend_from_slice(body);
    bytes
}
