use sip_core::parse_message;
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
    if edge_state
        .sipflow_enabled
        .load(std::sync::atomic::Ordering::Relaxed)
    {
        edge_state.capture_sip_packet(packet, "in", peer);
    }

    if let Err(datagrams) = sbc::check_sbc_filter(packet, peer, edge_state) {
        return datagrams;
    }

    match parse_message(packet) {
        Ok(sip_core::SipMessageBorrow::Request(request)) => {
            request::dispatch_request(request.into_owned(), peer, edge_state, edge_config).await
        }
        Ok(sip_core::SipMessageBorrow::Response(response)) => {
            response::dispatch_response(response.into_owned(), peer, edge_state, edge_config).await
        }
        Err(sip_core::SipParseError::EmptyMessage) => {
            // NAT 保活心跳（CRLF / 0 字节数据包），属于常见网络行为，静默忽略不报 warn
            Vec::new()
        }
        Err(error) => {
            warn!(%error, "failed to parse SIP datagram");
            Vec::new()
        }
    }
}
