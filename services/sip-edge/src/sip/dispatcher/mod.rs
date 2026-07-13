use sip_core::{parse_message, SipMessage};
use std::net::SocketAddr;
use tracing::warn;

use crate::config::EdgeConfig;
use crate::edge_state::{EdgeState, PendingDatagram};

pub(crate) mod request;
pub(crate) mod response;
pub(crate) mod sbc;

pub(crate) async fn handle_datagram(
    packet: &[u8],
    peer: SocketAddr,
    edge_state: &EdgeState,
    edge_config: &EdgeConfig,
) -> Vec<PendingDatagram> {
    if let Err(datagrams) = sbc::check_sbc_filter(packet, peer, edge_state) {
        return datagrams;
    }

    match parse_message(packet) {
        Ok(SipMessage::Request(request)) => {
            request::dispatch_request(request, peer, edge_state, edge_config).await
        }
        Ok(SipMessage::Response(response)) => {
            response::dispatch_response(response, peer, edge_state, edge_config).await
        }
        Err(error) => {
            warn!(%error, "failed to parse SIP datagram");
            Vec::new()
        }
    }
}
