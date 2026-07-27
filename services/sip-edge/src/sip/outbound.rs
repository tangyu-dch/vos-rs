//! # SIP 出站消息构建
//!
//! 本模块负责构建出站 SIP 消息，包括：
//!
//! - **INVITE**：出站呼叫邀请（含 SDP、Session Timer、Topology Hiding）
//! - **ACK**：确认响应
//! - **BYE**：终止呼叫
//! - **OPTIONS**：网关健康探测
//! - **NOTIFY**：REFER 转接进度通知
//! - **INFO**：DTMF 传递
//! - **REFER**：呼叫转接
//!
//! ## Topology Hiding
//!
//! 出站 INVITE 使用独立的 Call-ID（external_call_id），
//! 隐藏内部拓扑信息，防止外部网关探测内部网络结构。

use call_core::CallerIdentity;
use sip_core::{HeaderMap, Method, SipRequest, SipUri};
use std::str::FromStr;

use crate::edge_state::DialogLegState;

const DEFAULT_SIP_PORT: u16 = 5060;

pub fn target_addr_for(uri: &SipUri) -> String {
    format!("{}:{}", uri.host, uri.port.unwrap_or(DEFAULT_SIP_PORT))
}

/// Derives a socket target address from a raw SIP URI string (e.g. "sip:gw@10.0.0.1:5060").
/// Falls back to port 5060 if missing.
pub fn target_addr_for_str(raw_uri: &str) -> String {
    if let Ok(uri) = SipUri::from_str(raw_uri) {
        target_addr_for(&uri)
    } else {
        // Best-effort: strip the "sip:" prefix and use as-is
        let host = raw_uri
            .trim_start_matches("sip:")
            .trim_start_matches("sips:");
        if host.contains(':') {
            host.to_string()
        } else {
            format!("{host}:{DEFAULT_SIP_PORT}")
        }
    }
}

/// Builds an out-of-dialog OPTIONS request used for gateway health probing.
pub fn build_gateway_options(
    target_uri: &SipUri,
    advertised_addr: &str,
    call_id: &str,
    cseq: u32,
) -> Vec<u8> {
    let branch = format!("z9hG4bK-health-{cseq}");
    format!(
        "OPTIONS {target_uri} SIP/2.0\r\n\
         Via: SIP/2.0/UDP {advertised_addr};branch={branch}\r\n\
         Max-Forwards: 70\r\n\
         From: <sip:health-check@{advertised_addr}>;tag=health-{cseq}\r\n\
         To: <{target_uri}>\r\n\
         Call-ID: {call_id}\r\n\
         CSeq: {cseq} OPTIONS\r\n\
         Contact: <sip:health-check@{advertised_addr}>\r\n\
         Accept: application/sdp\r\n\
         Content-Length: 0\r\n\r\n",
        target_uri = target_uri,
        advertised_addr = advertised_addr,
        branch = branch,
        call_id = call_id,
        cseq = cseq,
    )
    .into_bytes()
}

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

/// Builds a new request owned by one B2BUA dialog leg.
///
/// Unlike a proxy-style forward, every dialog identifier is supplied by the target leg. The
/// inbound request is used only for the method, hop limit, transferable extension headers, and
/// body metadata.
#[allow(clippy::too_many_arguments)]
pub fn build_b2bua_in_dialog_request(
    inbound: &SipRequest,
    request_uri: &SipUri,
    advertised_addr: &str,
    route_set: &[String],
    call_id: &str,
    local_uri: &SipUri,
    local_tag: &str,
    remote_uri: &SipUri,
    remote_tag: Option<&str>,
    cseq: u32,
    body: &[u8],
) -> Vec<u8> {
    let branch = format!(
        "z9hG4bK-vosrs-{}-{}-{}",
        token_fragment(call_id),
        cseq,
        inbound.method.as_str().to_ascii_lowercase()
    );
    let mut request = format!(
        "{} {} SIP/2.0\r\nVia: SIP/2.0/UDP {};branch={}\r\nMax-Forwards: {}\r\n",
        inbound.method.as_str(),
        request_uri,
        advertised_addr,
        branch,
        next_max_forwards(&inbound.headers),
    );

    for route in route_set {
        request.push_str("Route: ");
        request.push_str(route);
        request.push_str("\r\n");
    }

    request.push_str(&format!("From: <{local_uri}>;tag={local_tag}\r\n"));
    request.push_str(&format!("To: <{remote_uri}>"));
    if let Some(tag) = remote_tag {
        request.push_str(";tag=");
        request.push_str(tag);
    }
    request.push_str("\r\nCall-ID: ");
    request.push_str(call_id);
    request.push_str("\r\nCSeq: ");
    request.push_str(&cseq.to_string());
    request.push(' ');
    request.push_str(inbound.method.as_str());
    request.push_str("\r\nContact: <sip:vosrs@");
    request.push_str(advertised_addr);
    request.push_str(">\r\n");

    for (lookup_name, output_name) in [
        ("refer-to", "Refer-To"),
        ("referred-by", "Referred-By"),
        ("refer-sub", "Refer-Sub"),
        ("rack", "RAck"),
        ("reason", "Reason"),
        ("event", "Event"),
        ("subscription-state", "Subscription-State"),
        ("require", "Require"),
        ("supported", "Supported"),
        ("session-expires", "Session-Expires"),
        ("min-se", "Min-SE"),
    ] {
        append_single_header(&mut request, &inbound.headers, lookup_name, output_name);
    }
    if !body.is_empty() {
        append_single_header(
            &mut request,
            &inbound.headers,
            "content-type",
            "Content-Type",
        );
    }
    request.push_str(&format!("Content-Length: {}\r\n\r\n", body.len()));

    let mut bytes = request.into_bytes();
    bytes.extend_from_slice(body);
    bytes
}

/// Build an outbound MESSAGE request by copying parameters from inbound MESSAGE.
/// Build a PRACK request to send toward the gateway confirming receipt of a
/// `Require: 100rel` provisional response.
///
/// `rack_value` is the `RAck` header value, e.g. `"1 1 INVITE"`.
pub fn build_outbound_prack(
    call_id: &str,
    from: &str,
    to: &str,
    cseq: u32,
    rack_value: &str,
    advertised_addr: &str,
    target_uri: &SipUri,
) -> Vec<u8> {
    let branch = format!("z9hG4bK-prack-{}-{}", token_fragment(call_id), cseq);
    let request = format!(
        "PRACK {uri} SIP/2.0\r\n\
         Via: SIP/2.0/UDP {addr};branch={branch}\r\n\
         Max-Forwards: 70\r\n\
         From: {from}\r\n\
         To: {to}\r\n\
         Call-ID: {call_id}\r\n\
         CSeq: {cseq} PRACK\r\n\
         Contact: <sip:vosrs@{addr}>\r\n\
         RAck: {rack}\r\n\
         Content-Length: 0\r\n\r\n",
        uri = target_uri,
        addr = advertised_addr,
        branch = branch,
        call_id = call_id,
        cseq = cseq,
        from = from,
        to = to,
        rack = rack_value,
    );
    request.into_bytes()
}

pub fn build_notify_sipfrag_with_state(
    call_id: &str,
    refer_from: &str,
    refer_to: &str,
    cseq: u32,
    advertised_addr: &str,
    body: &str,
    sub_state: &str,
) -> Vec<u8> {
    let target_uri = header_uri(refer_from)
        .unwrap_or_else(|| format!("sip:refer-subscription@{advertised_addr}"));
    let branch = format!("z9hG4bK-notify-{}-{}", token_fragment(call_id), cseq);
    let request = format!(
        "NOTIFY {target_uri} SIP/2.0\r\n\
         Via: SIP/2.0/UDP {addr};branch={branch}\r\n\
         Max-Forwards: 70\r\n\
         From: {from}\r\n\
         To: {to}\r\n\
         Call-ID: {call_id}\r\n\
         CSeq: {cseq} NOTIFY\r\n\
         Contact: <sip:vosrs@{addr}>\r\n\
         Event: refer\r\n\
         Subscription-State: {sub_state}\r\n\
         Content-Type: message/sipfrag;version=2.0\r\n\
         Content-Length: {len}\r\n\r\n{body}",
        target_uri = target_uri,
        addr = advertised_addr,
        branch = branch,
        from = refer_to,
        to = refer_from,
        call_id = call_id,
        cseq = cseq,
        sub_state = sub_state,
        len = body.len(),
        body = body,
    );
    request.into_bytes()
}

pub fn build_notify_sipfrag(
    call_id: &str,
    refer_from: &str,
    refer_to: &str,
    cseq: u32,
    advertised_addr: &str,
    body: &str,
) -> Vec<u8> {
    build_notify_sipfrag_with_state(
        call_id,
        refer_from,
        refer_to,
        cseq,
        advertised_addr,
        body,
        "active;expires=60",
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
    let branch = format!("z9hG4bK-transfer-{}-{}", token_fragment(call_id), cseq);
    let replaces_header = replaces
        .map(|val| format!("Replaces: {}\r\n", val))
        .unwrap_or_default();
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
        from = format!("<{}>;tag={}", dialog.local_uri, dialog.local_tag),
        to = dialog
            .remote_tag
            .as_ref()
            .map(|tag| format!("<{}>;tag={tag}", dialog.remote_uri))
            .unwrap_or_else(|| format!("<{}>", dialog.remote_uri)),
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
fn build_outbound_request_with_extra(
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

fn branch_for(request: &SipRequest) -> String {
    let call_id = request
        .headers
        .get("call-id")
        .map(|value| value.as_str())
        .unwrap_or("missing-call-id");
    let cseq = request
        .headers
        .get("cseq")
        .map(|value| value.as_str())
        .unwrap_or("missing-cseq");

    format!(
        "z9hG4bK-vosrs-{}-{}",
        token_fragment(call_id),
        token_fragment(cseq)
    )
}

fn token_fragment(value: &str) -> String {
    let token = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string();

    if token.is_empty() {
        "empty".to_string()
    } else {
        token
    }
}

fn header_uri(header: &str) -> Option<String> {
    let trimmed = header.trim();
    if let Some(start) = trimmed.find('<') {
        let rest = &trimmed[start + 1..];
        let end = rest.find('>')?;
        let uri = rest[..end].trim();
        if uri.starts_with("sip:") || uri.starts_with("sips:") {
            return Some(uri.to_string());
        }
    }

    let first = trimmed.split(';').next()?.trim();
    if first.starts_with("sip:") || first.starts_with("sips:") {
        Some(first.to_string())
    } else {
        None
    }
}

fn next_max_forwards(headers: &HeaderMap) -> u32 {
    headers
        .get("max-forwards")
        .and_then(|value| value.as_str().parse::<u32>().ok())
        .map(|value| value.saturating_sub(1))
        .unwrap_or(69)
}

pub fn is_forwardable_in_dialog_method(method: &Method) -> bool {
    matches!(
        method,
        Method::Bye | Method::Cancel | Method::Info | Method::Refer | Method::Update
    )
}

fn append_single_header(
    request: &mut String,
    headers: &HeaderMap,
    lookup_name: &str,
    output_name: &str,
) {
    if let Some(value) = headers.get(lookup_name) {
        request.push_str(output_name);
        request.push_str(": ");
        request.push_str(value.as_str());
        request.push_str("\r\n");
    }
}

/// Rebuilds identity headers from trusted structured data instead of forwarding raw input.
pub fn caller_identity_headers(
    inbound: &SipRequest,
    advertised_addr: &str,
    identity: &CallerIdentity,
) -> Option<String> {
    if advertised_addr.contains(['\r', '\n']) || !valid_caller_number(&identity.presented_number) {
        return None;
    }
    let tag = inbound
        .headers
        .get("from")
        .and_then(|value| header_parameter(value.as_str(), "tag"))
        .filter(|value| value.bytes().all(valid_token_byte));
    let tag = tag.map(|tag| format!(";tag={tag}")).unwrap_or_default();
    Some(format!(
        "From: <sip:{number}@{domain}>{tag}\r\nP-Asserted-Identity: <sip:{number}@{domain}>\r\n",
        number = identity.presented_number,
        domain = advertised_addr,
    ))
}

fn append_caller_identity_headers(
    request: &mut String,
    inbound: &SipRequest,
    advertised_addr: &str,
    identity: &CallerIdentity,
    local_from_tag: Option<&str>,
) {
    let headers = if let Some(local_tag) = local_from_tag {
        valid_caller_number(&identity.presented_number).then(|| {
            format!(
                "From: <sip:{number}@{domain}>;tag={local_tag}\r\nP-Asserted-Identity: <sip:{number}@{domain}>\r\n",
                number = identity.presented_number,
                domain = advertised_addr,
            )
        })
    } else {
        caller_identity_headers(inbound, advertised_addr, identity)
    };
    if let Some(headers) = headers {
        request.push_str(&headers);
    }
}

fn append_header_with_tag(
    request: &mut String,
    headers: &HeaderMap,
    lookup_name: &str,
    output_name: &str,
    tag: &str,
) {
    let Some(value) = headers.get(lookup_name) else {
        return;
    };
    let value = value.as_str();
    let lower = value.to_ascii_lowercase();
    let without_tag = if let Some(start) = lower.find(";tag=") {
        let end = value[start + 1..]
            .find(';')
            .map(|offset| start + 1 + offset)
            .unwrap_or(value.len());
        format!("{}{}", &value[..start], &value[end..])
    } else {
        value.to_string()
    };
    request.push_str(output_name);
    request.push_str(": ");
    request.push_str(&without_tag);
    request.push_str(";tag=");
    request.push_str(tag);
    request.push_str("\r\n");
}

fn valid_caller_number(number: &str) -> bool {
    let digits = number.strip_prefix('+').unwrap_or(number);
    !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
}

fn header_parameter<'a>(header: &'a str, name: &str) -> Option<&'a str> {
    header.split(';').skip(1).find_map(|parameter| {
        let (key, value) = parameter.trim().split_once('=')?;
        key.eq_ignore_ascii_case(name)
            .then_some(value.trim().trim_end_matches('>'))
    })
}

fn valid_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'+')
}

#[cfg(test)]
mod tests {
    use super::{
        build_b2bua_in_dialog_request, build_gateway_options, build_notify_sipfrag,
        caller_identity_headers, target_addr_for,
    };
    use call_core::{CallerIdentity, CallerIdentityMode, GatewayId};
    use sip_core::{parse_message, SipUri};
    use std::str::FromStr;

    #[test]
    fn caller_identity_headers_rebuild_from_and_pai_without_forwarding_display_name() {
        let inbound = invite_request();
        let identity = CallerIdentity {
            original_number: "1001".to_string(),
            presented_number: "13800138000".to_string(),
            owner_gateway_id: GatewayId::new("gw1"),
            mode: CallerIdentityMode::Fixed,
            max_concurrent: 1,
        };
        let headers = caller_identity_headers(&inbound, "edge.example.com:5060", &identity)
            .expect("valid identity headers");
        assert_eq!(
            headers,
            "From: <sip:13800138000@edge.example.com:5060>;tag=from-tag\r\n\
             P-Asserted-Identity: <sip:13800138000@edge.example.com:5060>\r\n"
        );
        assert!(!headers.contains("1001"));
    }

    #[test]
    fn caller_identity_headers_reject_header_injection() {
        let inbound = invite_request();
        let identity = CallerIdentity {
            original_number: "1001".to_string(),
            presented_number: "13800138000\r\nX-Evil: yes".to_string(),
            owner_gateway_id: GatewayId::new("gw1"),
            mode: CallerIdentityMode::Fixed,
            max_concurrent: 1,
        };
        assert!(caller_identity_headers(&inbound, "edge.example.com", &identity).is_none());
    }

    #[test]
    fn builds_gateway_options_probe() {
        let target = SipUri::from_str("sip:health-check@gw1.example.com:5060").unwrap();
        let options = String::from_utf8(build_gateway_options(
            &target,
            "edge.example.com:5060",
            "health-probe-gw1-1",
            1,
        ))
        .unwrap();

        assert!(options.starts_with("OPTIONS sip:health-check@gw1.example.com:5060 SIP/2.0\r\n"));
        assert!(options.contains("CSeq: 1 OPTIONS\r\n"));
        assert!(options.contains("Call-ID: health-probe-gw1-1\r\n"));
        assert!(options.contains("Content-Length: 0\r\n\r\n"));
    }

    #[test]
    fn b2bua_in_dialog_request_uses_only_target_leg_dialog_identifiers() {
        let inbound = request(concat!(
            "BYE sip:edge@example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP caller.example.com;branch=caller-branch\r\n",
            "From: <sip:caller@example.com>;tag=caller-tag\r\n",
            "To: <sip:callee@example.com>;tag=edge-a-tag\r\n",
            "Call-ID: caller-leg-id\r\n",
            "CSeq: 44 BYE\r\n",
            "Reason: SIP;cause=200;text=normal\r\n",
            "Content-Length: 0\r\n\r\n"
        ));
        let target = SipUri::from_str("sip:callee@callee.example.com:5070").unwrap();
        let local = SipUri::from_str("sip:caller@edge.example.com").unwrap();
        let remote = SipUri::from_str("sip:callee@callee.example.com").unwrap();

        let outbound = String::from_utf8(build_b2bua_in_dialog_request(
            &inbound,
            &target,
            "edge.example.com:5060",
            &["<sip:proxy.example.com;lr>".to_string()],
            "gateway-leg-id",
            &local,
            "edge-b-tag",
            &remote,
            Some("callee-tag"),
            9,
            &[],
        ))
        .unwrap();

        assert!(outbound.starts_with("BYE sip:callee@callee.example.com:5070 SIP/2.0\r\n"));
        assert!(outbound.contains("Route: <sip:proxy.example.com;lr>\r\n"));
        assert!(outbound.contains("From: <sip:caller@edge.example.com>;tag=edge-b-tag\r\n"));
        assert!(outbound.contains("To: <sip:callee@callee.example.com>;tag=callee-tag\r\n"));
        assert!(outbound.contains("Call-ID: gateway-leg-id\r\n"));
        assert!(outbound.contains("CSeq: 9 BYE\r\n"));
        assert!(!outbound.contains("caller-leg-id"));
        assert!(!outbound.contains("caller-tag"));
        assert!(!outbound.contains("caller-branch"));
    }

    #[test]
    fn target_addr_defaults_to_5060() {
        let uri = SipUri::from_str("sip:13800138000@gw1.example.com").unwrap();

        assert_eq!(target_addr_for(&uri), "gw1.example.com:5060");
    }

    #[test]
    fn builds_notify_sipfrag_for_refer_progress() {
        let notify = build_notify_sipfrag(
            "refer-call@example.com",
            "<sip:1001@example.com>;tag=from-tag",
            "<sip:13800138000@example.com>;tag=to-tag",
            52,
            "edge.example.com:5060",
            "SIP/2.0 100 Trying\r\n",
        );
        let notify = String::from_utf8(notify).expect("NOTIFY should be UTF-8");

        assert!(notify.starts_with("NOTIFY sip:1001@example.com SIP/2.0\r\n"));
        assert!(notify.contains(
            "Via: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-notify-refer-call-example-com-52\r\n"
        ));
        assert!(notify.contains("From: <sip:13800138000@example.com>;tag=to-tag\r\n"));
        assert!(notify.contains("To: <sip:1001@example.com>;tag=from-tag\r\n"));
        assert!(notify.contains("Call-ID: refer-call@example.com\r\n"));
        assert!(notify.contains("CSeq: 52 NOTIFY\r\n"));
        assert!(notify.contains("Event: refer\r\n"));
        assert!(notify.contains("Subscription-State: active;expires=60\r\n"));
        assert!(notify.contains("Content-Type: message/sipfrag;version=2.0\r\n"));
        assert!(notify.ends_with("Content-Length: 20\r\n\r\nSIP/2.0 100 Trying\r\n"));
    }

    fn invite_request() -> sip_core::SipRequest {
        let raw = concat!(
            "INVITE sip:13800138000@example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-1\r\n",
            "Max-Forwards: 70\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:13800138000@example.com>\r\n",
            "Call-ID: call-1@example.com\r\n",
            "CSeq: 1 INVITE\r\n",
            "Content-Type: application/sdp\r\n",
            "Content-Length: 5\r\n",
            "\r\n",
            "v=0\r\n"
        );

        request(raw)
    }

    fn request(raw: &str) -> sip_core::SipRequest {
        let sip_core::SipMessageBorrow::Request(request) = parse_message(raw.as_bytes()).unwrap()
        else {
            panic!("expected request");
        };
        request.into_owned()
    }
}
