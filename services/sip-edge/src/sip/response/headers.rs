use std::net::SocketAddr;

use sip_core::HeaderMap;

use super::EDGE_TAG;

pub(super) fn append_to_with_local_tag(out: &mut String, headers: &HeaderMap, local_tag: &str) {
    let Some(value) = headers.get("to") else {
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
    out.push_str("To: ");
    out.push_str(&without_tag);
    out.push_str(";tag=");
    out.push_str(local_tag);
    out.push_str("\r\n");
}

pub(super) fn append_all_headers(
    out: &mut String,
    headers: &HeaderMap,
    name: &str,
    header_name: &str,
) {
    for value in headers.get_all(name) {
        out.push_str(header_name);
        out.push_str(": ");
        out.push_str(value.as_str());
        out.push_str("\r\n");
    }
}

pub(super) fn append_via_headers_with_peer(
    out: &mut String,
    headers: &HeaderMap,
    peer: Option<SocketAddr>,
) {
    for (i, via) in headers.get_all("via").enumerate() {
        out.push_str("Via: ");
        if i == 0 {
            out.push_str(&patch_via_rport_and_received(via.as_str(), peer));
        } else {
            out.push_str(via.as_str());
        }
        out.push_str("\r\n");
    }
}

pub(super) fn append_single_header(
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

pub(super) fn append_to_header(response: &mut String, headers: &HeaderMap, status_code: u16) {
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

pub fn patch_via_rport_and_received(via: &str, peer: Option<SocketAddr>) -> String {
    if let Ok(mut via_hdr) = sip_core::ViaHeader::parse(via) {
        if let Some(peer) = peer {
            via_hdr.apply_rfc3581(peer.ip(), peer.port());
        }
        via_hdr.to_string()
    } else {
        via.to_string()
    }
}
