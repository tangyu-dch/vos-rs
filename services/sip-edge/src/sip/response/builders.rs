use std::net::SocketAddr;

use call_core::CallError;
use sip_core::{Method, SipRequest, SipResponse};

use super::headers::{
    append_all_headers, append_single_header, append_to_header, append_to_with_local_tag,
    append_via_headers_with_peer,
};
use super::invite::invite_error_status;
use super::SERVER_HEADER;

pub(super) fn build_response(
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

pub fn response_100_trying(request: &sip_core::SipRequestBorrow<'_>) -> Vec<u8> {
    build_response(request, 100, "Trying", &[], "")
}

pub fn build_response_with_owned_headers(
    request: &sip_core::SipRequestBorrow<'_>,
    status_code: u16,
    reason_phrase: &str,
    extra_headers: &[(String, String)],
    body: &str,
) -> Vec<u8> {
    build_response_with_owned_headers_and_peer(
        request,
        status_code,
        reason_phrase,
        extra_headers,
        body,
        None,
    )
}

pub fn build_response_with_owned_headers_and_peer(
    request: &sip_core::SipRequestBorrow<'_>,
    status_code: u16,
    reason_phrase: &str,
    extra_headers: &[(String, String)],
    body: &str,
    peer: Option<SocketAddr>,
) -> Vec<u8> {
    let mut response = String::new();
    response.push_str(&format!("SIP/2.0 {status_code} {reason_phrase}\r\n"));

    append_via_headers_with_peer(&mut response, &request.headers, peer);
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

/// Builds the response owned by the inbound B2BUA dialog leg.
///
/// The downstream status, To-tag and negotiation headers are retained, while dialog routing
/// identity comes exclusively from the original inbound request and this edge. In particular,
/// the downstream Contact must never become the caller's remote target.
pub(crate) fn build_inbound_leg_response(
    downstream: &SipResponse,
    inbound_request: &SipRequest,
    advertised_addr: &str,
    caller_local_tag: &str,
    body: &[u8],
    peer: Option<SocketAddr>,
) -> Vec<u8> {
    let mut forwarded = String::new();
    forwarded.push_str(&format!(
        "SIP/2.0 {} {}\r\n",
        downstream.status_code, downstream.reason_phrase
    ));

    append_via_headers_with_peer(&mut forwarded, &inbound_request.headers, peer);
    append_all_headers(
        &mut forwarded,
        &inbound_request.headers,
        "record-route",
        "Record-Route",
    );
    append_single_header(&mut forwarded, &inbound_request.headers, "from", "From");
    append_to_with_local_tag(&mut forwarded, &inbound_request.headers, caller_local_tag);
    append_single_header(
        &mut forwarded,
        &inbound_request.headers,
        "call-id",
        "Call-ID",
    );
    append_single_header(&mut forwarded, &inbound_request.headers, "cseq", "CSeq");

    let creates_dialog = downstream.status_code > 100
        && downstream.status_code < 300
        && inbound_request.method == Method::Invite;
    if creates_dialog {
        forwarded.push_str("Contact: <sip:vosrs@");
        forwarded.push_str(advertised_addr);
        forwarded.push_str(">\r\n");
    }

    // RFC 3262: pass through 100rel negotiation headers in provisional responses
    append_single_header(&mut forwarded, &downstream.headers, "require", "Require");
    append_single_header(&mut forwarded, &downstream.headers, "rseq", "RSeq");

    // RFC 4028: pass through session timer negotiation headers
    append_single_header(
        &mut forwarded,
        &downstream.headers,
        "session-expires",
        "Session-Expires",
    );
    append_single_header(&mut forwarded, &downstream.headers, "min-se", "Min-SE");
    append_single_header(
        &mut forwarded,
        &downstream.headers,
        "supported",
        "Supported",
    );

    if !body.is_empty() {
        append_single_header(
            &mut forwarded,
            &downstream.headers,
            "content-type",
            "Content-Type",
        );
    }

    forwarded.push_str(&format!("Server: {SERVER_HEADER}\r\n"));
    forwarded.push_str(&format!("Content-Length: {}\r\n", body.len()));
    forwarded.push_str("\r\n");

    let mut bytes = forwarded.into_bytes();
    bytes.extend_from_slice(body);
    bytes
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
