use crate::config::EdgeConfig;
use crate::edge_state::{DialogLeg, EdgeState, InboundTransaction};
use crate::media;
use sip_core::SipResponse;
use std::net::SocketAddr;

pub(crate) fn prepare_response_sdp(
    sip_response: &SipResponse,
    peer: SocketAddr,
    transaction: Option<&InboundTransaction>,
    call_id: Option<&str>,
    edge_state: &EdgeState,
    edge_config: &EdgeConfig,
) -> (Option<Vec<u8>>, bool) {
    let mut rewritten_sdp_body = None;
    let mut mid_dialog_rewritten = false;
    let media_session_id = transaction
        .map(|value| value.session_id.as_str())
        .or(call_id)
        .unwrap_or("");

    if let Some(t) = transaction {
        if t.gateway_relay_rtp.is_some() && t.caller_relay_rtp.is_some() {
            mid_dialog_rewritten = true;
            if media::is_sdp_body(&sip_response.headers, &sip_response.body) {
                let is_to_caller = matches!(t.dialog_leg_for_peer(peer), Some(DialogLeg::Gateway));
                let relay_ep = if is_to_caller {
                    t.caller_relay_rtp.as_ref()
                } else {
                    t.gateway_relay_rtp.as_ref()
                };

                if let Some(ep) = relay_ep {
                    if let Ok((rewritten, remote_ep)) =
                        media::rewrite_sdp_and_extract_endpoint(&sip_response.body, ep)
                    {
                        rewritten_sdp_body = Some(rewritten);
                        crate::sip::handlers::register_relay_target(
                            &edge_state.media_relay,
                            ep,
                            &remote_ep,
                            "mid-dialog response target update",
                        );

                        if !media_session_id.is_empty() {
                            if let Some(mut t_mut) =
                                edge_state.inbound_transactions.get_mut(media_session_id)
                            {
                                if is_to_caller {
                                    t_mut.gateway_rtp = Some(remote_ep);
                                } else {
                                    t_mut.caller_rtp = Some(remote_ep);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    let rewritten_sdp_bytes = if mid_dialog_rewritten {
        rewritten_sdp_body
    } else {
        let caller_is_webrtc = transaction
            .and_then(|value| value.original_request.as_ref())
            .is_some_and(|request| media::is_webrtc_sdp(&request.body));
        let prepared =
            if caller_is_webrtc && media::is_sdp_body(&sip_response.headers, &sip_response.body) {
                crate::sip::handlers::prepare_webrtc_answer(
                    &sip_response.body,
                    &edge_state.media_relay,
                    &edge_config.media,
                    media_session_id,
                )
                .map(Some)
            } else {
                crate::sip::handlers::prepare_rewritten_sdp(
                    &sip_response.headers,
                    &sip_response.body,
                    &edge_state.media_relay,
                    &edge_config.media,
                    "outbound response answer",
                    media_session_id,
                )
            };
        match prepared {
            Ok(Some(sdp)) => {
                if let Some(gateway_rtp) = &sdp.original_endpoint {
                    crate::sip::handlers::register_relay_target(
                        &edge_state.media_relay,
                        &sdp.relay_endpoint,
                        gateway_rtp,
                        "caller-to-gateway RTP",
                    );

                    if let Some(pt) = media::parse_sdp_dtmf_payload_type(&sip_response.body) {
                        edge_state.media_relay.register_port_dtmf_tracking(
                            media_session_id,
                            sdp.relay_endpoint.port,
                            pt,
                        );
                    }

                    if let Some(t) = transaction {
                        if let Some(original_req) = &t.original_request {
                            if let Some(pt) = media::parse_sdp_dtmf_payload_type(&original_req.body)
                            {
                                if let Some(gateway_relay) = &t.gateway_relay_rtp {
                                    edge_state.media_relay.register_port_dtmf_tracking(
                                        media_session_id,
                                        gateway_relay.port,
                                        pt,
                                    );
                                }
                            }
                        }
                    }

                    edge_state.remember_gateway_media(
                        media_session_id,
                        sdp.original_endpoint.clone(),
                        sdp.relay_endpoint.clone(),
                        &edge_config.media,
                    );
                }
                Some(sdp.body)
            }
            _ => None,
        }
    };

    (rewritten_sdp_bytes, mid_dialog_rewritten)
}
