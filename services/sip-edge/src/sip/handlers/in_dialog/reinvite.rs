use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use sip_core::{Method, SipRequest};
use tracing::debug;

use super::super::register_relay_target;
use crate::edge_state::{DialogLeg, EdgeState, InboundTransaction};
use crate::media;

/// Refresh the session timer and rewrite SDP for mid-dialog Re-INVITE/UPDATE requests.
///
/// - Updates `last_session_refresh` on the matched inbound transaction.
/// - When the request carries an SDP body, rewrites it for the peer leg's relay
///   endpoint (single-pass rewrite + endpoint extraction) and registers the new RTP
///   target with the media relay. The transaction's `caller_rtp` / `gateway_rtp` and
///   `original_request` fields are updated accordingly.
///
/// Returns the rewritten SDP body to be used as the outbound body, or `None` if no
/// rewriting was performed (bridged call, non-refresh method, or missing SDP).
pub(super) fn refresh_session_renegotiation(
    request: &SipRequest,
    peer: SocketAddr,
    edge_state: &EdgeState,
    transaction: &InboundTransaction,
    call_id: &str,
    is_bridged: bool,
) -> Option<Vec<u8>> {
    if is_bridged || !matches!(&request.method, Method::Invite | Method::Update) {
        return None;
    }

    // Refresh session timer timestamp.
    {
        if let Some(mut t_mut) = edge_state.inbound_transactions.get_mut(call_id) {
            t_mut.last_session_refresh = Some(Instant::now());
            debug!(call_id, "session timer refreshed by Re-INVITE/UPDATE");
        }
    }

    let mut rewritten_sdp: Option<Vec<u8>> = None;

    if media::is_sdp_body(&request.headers, &request.body) {
        let is_from_caller = transaction.dialog_leg_for_peer(peer) == Some(DialogLeg::Caller);
        if is_from_caller {
            if let Some(gw_relay) = &transaction.gateway_relay_rtp {
                // Single-pass: rewrite SDP + extract original endpoint
                if let Ok((rewritten, remote_ep)) =
                    media::rewrite_sdp_and_extract_endpoint(&request.body, gw_relay)
                {
                    rewritten_sdp = Some(rewritten);
                    register_relay_target(
                        &edge_state.media_relay,
                        gw_relay,
                        &remote_ep,
                        "mid-dialog caller target update",
                    );

                    if let Some(mut t_mut) = edge_state.inbound_transactions.get_mut(call_id) {
                        t_mut.caller_rtp = Some(remote_ep);
                        t_mut.original_request = Some(Arc::new(request.clone()));
                    }
                }
            }
        } else if let Some(caller_relay) = &transaction.caller_relay_rtp {
            // Single-pass: rewrite SDP + extract original endpoint
            if let Ok((rewritten, remote_ep)) =
                media::rewrite_sdp_and_extract_endpoint(&request.body, caller_relay)
            {
                rewritten_sdp = Some(rewritten);
                register_relay_target(
                    &edge_state.media_relay,
                    caller_relay,
                    &remote_ep,
                    "mid-dialog gateway target update",
                );

                if let Some(mut t_mut) = edge_state.inbound_transactions.get_mut(call_id) {
                    t_mut.gateway_rtp = Some(remote_ep);
                }
            }
        }
    }

    rewritten_sdp
}
