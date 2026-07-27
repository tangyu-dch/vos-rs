use std::net::SocketAddr;

use sip_core::SipRequest;
use tracing::{debug, warn};

use super::super::parse_sip_info_dtmf;
use crate::edge_state::{EdgeState, InboundTransaction, PendingDatagram};
use crate::sip::response;

/// Handle in-dialog INFO requests: parse DTMF digits from the body and register them
/// with the media relay, then append a 200 OK response to `datagrams`.
///
/// INFO is subsequently forwarded to the peer leg by the orchestrator (no early return).
pub(super) fn handle_info(
    request: &SipRequest,
    peer: SocketAddr,
    edge_state: &EdgeState,
    transaction: &InboundTransaction,
    datagrams: &mut Vec<PendingDatagram>,
) {
    let content_type = request
        .headers
        .get("content-type")
        .map(|v| v.as_str())
        .unwrap_or("");
    if let Some(digit) = parse_sip_info_dtmf(content_type, &request.body) {
        edge_state
            .media_relay
            .register_info_dtmf_digit(&transaction.session_id, digit);
    }

    datagrams.push(PendingDatagram::new(
        peer.to_string(),
        response::ok_for_request(request),
    ));
}

/// Handle in-dialog PRACK requests: validate the RAck header and respond 200 OK.
///
/// PRACK is fully terminated locally (the gateway leg has already been confirmed when
/// the reliable provisional response was sent). Always appends a response to `datagrams`
/// and returns `true` to signal early return to the orchestrator.
pub(super) fn handle_prack(
    request: &SipRequest,
    peer: SocketAddr,
    datagrams: &mut Vec<PendingDatagram>,
) -> bool {
    let rack_valid = if let Some(rack) = request.headers.get("rack") {
        let parts = rack.as_str().split_whitespace().collect::<Vec<_>>();
        if parts.len() == 3 {
            let rseq_ok = parts[0].parse::<u32>().is_ok();
            let cseq_ok = parts[1].parse::<u32>().is_ok();
            let method_ok = !parts[2].is_empty();
            rseq_ok && cseq_ok && method_ok
        } else {
            false
        }
    } else {
        false
    };

    if !rack_valid {
        warn!("received PRACK with missing or invalid RAck header");
        datagrams.push(PendingDatagram::new(
            peer.to_string(),
            response::build_response_with_owned_headers(
                request,
                400,
                "Bad Request - Invalid RAck",
                &[],
                "",
            ),
        ));
        return true;
    }

    debug!(
        call_id = request
            .headers
            .get("call-id")
            .map(|v| v.as_str())
            .unwrap_or("?"),
        "received PRACK from caller — responding 200 OK (already confirmed to gateway)"
    );
    datagrams.push(PendingDatagram::new(
        peer.to_string(),
        response::ok_for_request(request),
    ));
    true
}
