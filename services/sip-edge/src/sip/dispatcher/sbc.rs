use sip_core::{parse_message, SipMessage};
use std::net::SocketAddr;
use tracing::warn;

use crate::edge_state::{EdgeState, PendingDatagram};
use crate::sip::response;

pub(crate) fn check_sbc_filter(
    packet: &[u8],
    peer: SocketAddr,
    edge_state: &EdgeState,
) -> Result<(), Vec<PendingDatagram>> {
    if !edge_state.sbc_engine.is_allowed(peer.ip()) {
        warn!(%peer, "packet blocked by SBC IP ACL");
        return Err(Vec::new());
    }

    if !edge_state.sbc_rate_limit_enabled || peer.ip().is_loopback() {
        return Ok(());
    }

    if !edge_state.sbc_engine.check_rate(peer.ip()) {
        warn!(%peer, "packet blocked by SBC rate limit");
        if let Ok(SipMessage::Request(req)) = parse_message(packet) {
            return Err(vec![PendingDatagram::new(
                peer.to_string(),
                response::build_response_with_owned_headers(
                    &req,
                    503,
                    "Service Unavailable - Rate Limit Exceeded",
                    &[("Retry-After".to_string(), "10".to_string())],
                    "",
                ),
            )]);
        }
        return Err(Vec::new());
    }

    Ok(())
}
