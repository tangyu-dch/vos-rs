use sip_core::{HeaderMap, Method, SipRequest, SipUri};
use std::str::FromStr;

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

#[allow(dead_code)]
pub fn build_outbound_invite_with_body(
    inbound: &SipRequest,
    outbound_uri: &SipUri,
    advertised_addr: &str,
    body: &[u8],
) -> Vec<u8> {
    build_outbound_request(inbound, outbound_uri, advertised_addr, &[], body, None)
}

/// Topology-hiding variant that sends a different `Call-ID` on the outbound leg.
pub fn build_outbound_invite_with_body_and_call_id(
    inbound: &SipRequest,
    outbound_uri: &SipUri,
    advertised_addr: &str,
    body: &[u8],
    external_call_id: &str,
) -> Vec<u8> {
    build_outbound_request(
        inbound,
        outbound_uri,
        advertised_addr,
        &[],
        body,
        Some(external_call_id),
    )
}

/// Topology-hiding variant of `build_outbound_invite_with_session_timer`.
/// Sends `external_call_id` on the outbound leg instead of copying the inbound Call-ID.
pub fn build_outbound_invite_with_session_timer_and_call_id(
    inbound: &SipRequest,
    outbound_uri: &SipUri,
    advertised_addr: &str,
    body: &[u8],
    session_expires: u32,
    route_set: &[String],
    external_call_id: &str,
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
        Some(external_call_id),
    )
}

pub fn build_outbound_in_dialog_request(
    inbound: &SipRequest,
    outbound_uri: &SipUri,
    advertised_addr: &str,
    route_set: &[String],
) -> Vec<u8> {
    build_outbound_request(
        inbound,
        outbound_uri,
        advertised_addr,
        route_set,
        &inbound.body,
        None,
    )
}

pub fn build_outbound_in_dialog_request_with_body(
    inbound: &SipRequest,
    outbound_uri: &SipUri,
    advertised_addr: &str,
    route_set: &[String],
    body: &[u8],
) -> Vec<u8> {
    build_outbound_request(
        inbound,
        outbound_uri,
        advertised_addr,
        route_set,
        body,
        None,
    )
}

/// Build an outbound MESSAGE request by copying parameters from inbound MESSAGE.
pub fn build_outbound_message(
    inbound: &SipRequest,
    outbound_uri: &SipUri,
    advertised_addr: &str,
) -> Vec<u8> {
    build_outbound_request(
        inbound,
        outbound_uri,
        advertised_addr,
        &[],
        &inbound.body,
        None,
    )
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
pub fn build_transfer_invite(
    call_id: &str,
    from: &str,
    to: &str,
    cseq: u32,
    advertised_addr: &str,
    target_uri: &SipUri,
    sdp_body: &[u8],
) -> Vec<u8> {
    let branch = format!("z9hG4bK-transfer-{}-{}", token_fragment(call_id), cseq);
    let request = format!(
        "INVITE {uri} SIP/2.0\r\n\
         Via: SIP/2.0/UDP {addr};branch={branch}\r\n\
         Max-Forwards: 70\r\n\
         From: {from}\r\n\
         To: {to}\r\n\
         Call-ID: {call_id}\r\n\
         CSeq: {cseq} INVITE\r\n\
         Contact: <sip:vosrs@{addr}>\r\n\
         Content-Type: application/sdp\r\n\
         Content-Length: {body_len}\r\n\r\n",
        uri = target_uri,
        addr = advertised_addr,
        branch = branch,
        from = from,
        to = to,
        call_id = call_id,
        cseq = cseq,
        body_len = sdp_body.len()
    );
    let mut bytes = request.into_bytes();
    bytes.extend_from_slice(sdp_body);
    bytes
}

fn build_outbound_request(
    inbound: &SipRequest,
    outbound_uri: &SipUri,
    advertised_addr: &str,
    route_set: &[String],
    body: &[u8],
    override_call_id: Option<&str>,
) -> Vec<u8> {
    build_outbound_request_with_extra(
        inbound,
        outbound_uri,
        advertised_addr,
        route_set,
        body,
        "",
        override_call_id,
    )
}

fn build_outbound_request_with_extra(
    inbound: &SipRequest,
    outbound_uri: &SipUri,
    advertised_addr: &str,
    route_set: &[String],
    body: &[u8],
    extra_headers: &str,
    override_call_id: Option<&str>,
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

    append_single_header(&mut request, &inbound.headers, "from", "From");
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
        Method::Ack | Method::Bye | Method::Cancel | Method::Info | Method::Refer | Method::Update
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

#[cfg(test)]
mod tests {
    use super::{
        build_notify_sipfrag, build_outbound_in_dialog_request, build_outbound_invite_with_body,
        target_addr_for,
    };
    use sip_core::{parse_message, SipMessage, SipUri};
    use std::str::FromStr;

    #[test]
    fn builds_outbound_invite_for_gateway() {
        let inbound = invite_request();
        let outbound_uri = SipUri::from_str("sip:13800138000@gw1.example.com:5070").unwrap();

        let outbound = build_outbound_invite_with_body(
            &inbound,
            &outbound_uri,
            "edge.example.com:5060",
            &inbound.body,
        );
        let outbound = String::from_utf8(outbound).expect("outbound INVITE should be UTF-8");

        assert!(outbound.starts_with("INVITE sip:13800138000@gw1.example.com:5070 SIP/2.0\r\n"));
        assert!(outbound.contains(
            "Via: SIP/2.0/UDP edge.example.com:5060;branch=z9hG4bK-vosrs-call-1-example-com-1-invite\r\n"
        ));
        assert!(outbound.contains("Max-Forwards: 69\r\n"));
        assert!(outbound.contains("Contact: <sip:vosrs@edge.example.com:5060>\r\n"));
        assert!(outbound.contains("Content-Type: application/sdp\r\n"));
        assert!(outbound.ends_with("v=0\r\n"));
    }

    #[test]
    fn target_addr_defaults_to_5060() {
        let uri = SipUri::from_str("sip:13800138000@gw1.example.com").unwrap();

        assert_eq!(target_addr_for(&uri), "gw1.example.com:5060");
    }

    #[test]
    fn builds_outbound_invite_with_rewritten_body() {
        let inbound = invite_request();
        let outbound_uri = SipUri::from_str("sip:13800138000@gw1.example.com:5070").unwrap();

        let outbound = build_outbound_invite_with_body(
            &inbound,
            &outbound_uri,
            "edge.example.com:5060",
            b"v=0\r\ns=rewritten\r\n",
        );
        let outbound = String::from_utf8(outbound).expect("outbound INVITE should be UTF-8");

        assert!(outbound.contains("Content-Type: application/sdp\r\n"));
        assert!(outbound.contains("Content-Length: 18\r\n\r\nv=0\r\ns=rewritten\r\n"));
    }

    #[test]
    fn builds_outbound_ack_without_body() {
        let inbound = request(concat!(
            "ACK sip:13800138000@example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-ack\r\n",
            "Max-Forwards: 70\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:13800138000@example.com>;tag=gw-tag\r\n",
            "Call-ID: call-1@example.com\r\n",
            "CSeq: 1 ACK\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        ));
        let outbound_uri = SipUri::from_str("sip:13800138000@gw1.example.com:5060").unwrap();

        let outbound =
            build_outbound_in_dialog_request(&inbound, &outbound_uri, "edge.example.com:5060", &[]);
        let outbound = String::from_utf8(outbound).expect("outbound ACK should be UTF-8");

        assert!(outbound.starts_with("ACK sip:13800138000@gw1.example.com:5060 SIP/2.0\r\n"));
        assert!(outbound.contains("CSeq: 1 ACK\r\n"));
        assert!(outbound.contains("Content-Length: 0\r\n\r\n"));
    }

    #[test]
    fn builds_outbound_info_with_body() {
        let inbound = request(concat!(
            "INFO sip:13800138000@example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-info\r\n",
            "Max-Forwards: 70\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:13800138000@example.com>;tag=gw-tag\r\n",
            "Call-ID: call-info@example.com\r\n",
            "CSeq: 2 INFO\r\n",
            "Content-Type: application/dtmf-relay\r\n",
            "Content-Length: 24\r\n",
            "\r\n",
            "Signal=1\r\nDuration=160\r\n"
        ));
        let outbound_uri = SipUri::from_str("sip:13800138000@gw1.example.com:5060").unwrap();

        let outbound =
            build_outbound_in_dialog_request(&inbound, &outbound_uri, "edge.example.com:5060", &[]);
        let outbound = String::from_utf8(outbound).expect("outbound INFO should be UTF-8");

        assert!(outbound.starts_with("INFO sip:13800138000@gw1.example.com:5060 SIP/2.0\r\n"));
        assert!(outbound.contains("CSeq: 2 INFO\r\n"));
        assert!(outbound.contains("Content-Type: application/dtmf-relay\r\n"));
        assert!(outbound.contains("Content-Length: 24\r\n\r\nSignal=1\r\nDuration=160\r\n"));
    }

    #[test]
    fn builds_outbound_refer_with_transfer_headers() {
        let inbound = request(concat!(
            "REFER sip:13800138000@example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-refer\r\n",
            "Max-Forwards: 70\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:13800138000@example.com>;tag=gw-tag\r\n",
            "Call-ID: call-refer@example.com\r\n",
            "CSeq: 3 REFER\r\n",
            "Refer-To: <sip:1002@example.com>\r\n",
            "Referred-By: <sip:1001@example.com>\r\n",
            "Refer-Sub: false\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        ));
        let outbound_uri = SipUri::from_str("sip:13800138000@gw1.example.com:5060").unwrap();

        let outbound =
            build_outbound_in_dialog_request(&inbound, &outbound_uri, "edge.example.com:5060", &[]);
        let outbound = String::from_utf8(outbound).expect("outbound REFER should be UTF-8");

        assert!(outbound.starts_with("REFER sip:13800138000@gw1.example.com:5060 SIP/2.0\r\n"));
        assert!(outbound.contains("CSeq: 3 REFER\r\n"));
        assert!(outbound.contains("Refer-To: <sip:1002@example.com>\r\n"));
        assert!(outbound.contains("Referred-By: <sip:1001@example.com>\r\n"));
        assert!(outbound.contains("Refer-Sub: false\r\n"));
        assert!(outbound.contains("Content-Length: 0\r\n\r\n"));
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
        let SipMessage::Request(request) = parse_message(raw.as_bytes()).unwrap() else {
            panic!("expected request");
        };
        request
    }
}
