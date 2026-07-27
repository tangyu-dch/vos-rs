//! 出站 dialog 内请求构建：BYE/INFO/REFER 等方法、PRACK、NOTIFY、OPTIONS 探测。

use sip_core::{SipRequest, SipUri};

use super::helpers::{append_single_header, header_uri, next_max_forwards, token_fragment};

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
