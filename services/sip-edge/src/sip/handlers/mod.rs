use std::net::SocketAddr;

use call_core::CallError;
use sdp_core::{RtpEndpoint, SessionDescription};
use sip_core::{HeaderMap, Method, SipRequest};
use tracing::warn;

use crate::config::EdgeConfig;
use crate::edge_state::{EdgeState, PendingDatagram};
use crate::media::{self, MediaConfig, MediaRelayState};
use crate::sip::dialog::DialogValidationError;
use crate::sip::{outbound, response, AuthConfig};

pub mod in_dialog;
pub mod invite;
pub mod message;
pub mod register;

pub(crate) use in_dialog::handle_in_dialog_request;
pub(crate) use invite::handle_invite_request;
pub(crate) use message::handle_out_of_dialog_message;
pub(crate) use register::handle_register_request;

pub(crate) async fn handle_request(
    request: SipRequest,
    peer: SocketAddr,
    edge_state: &EdgeState,
    edge_config: &EdgeConfig,
) -> Vec<PendingDatagram> {
    if matches!(&request.method, Method::Register) {
        return handle_register_request(request, peer, edge_state, edge_config).await;
    }

    if matches!(&request.method, Method::Message) {
        let to_tag = request
            .headers
            .get("to")
            .and_then(|v| crate::sip::dialog::tag_param(v.as_str()));
        if to_tag.is_some() {
            return handle_in_dialog_request(request, peer, edge_state, edge_config).await;
        } else {
            return handle_out_of_dialog_message(request, peer, edge_state, edge_config).await;
        }
    }

    if outbound::is_forwardable_in_dialog_method(&request.method) {
        return handle_in_dialog_request(request, peer, edge_state, edge_config).await;
    }

    // Mid-dialog Re-INVITE: To header contains a tag, meaning this is within an established dialog
    if matches!(&request.method, Method::Invite) {
        let to_tag = request
            .headers
            .get("to")
            .and_then(|v| crate::sip::dialog::tag_param(v.as_str()));
        if to_tag.is_some() {
            return handle_in_dialog_request(request, peer, edge_state, edge_config).await;
        }
    }

    // RFC 3262: PRACK is always an in-dialog message (has To-tag) — route to in-dialog handler
    if matches!(&request.method, Method::Prack) {
        return handle_in_dialog_request(request, peer, edge_state, edge_config).await;
    }

    if matches!(&request.method, Method::Invite) {
        return handle_invite_request(request, peer, edge_state, edge_config).await;
    }

    let mut health = edge_state
        .gateway_health
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let handling = response::response_for_request_with_health(
        &request,
        &edge_state.call_manager,
        Some(&mut health),
    );
    vec![PendingDatagram::new(peer.to_string(), handling.response)]
}

// --- Helper functions for submodules ---

pub(super) fn call_error_for_unknown_request(request: &SipRequest) -> CallError {
    match request.headers.get("call-id") {
        Some(call_id) => CallError::UnknownCall(call_id.as_str().to_string()),
        None => CallError::MissingRequiredHeader("Call-ID"),
    }
}

pub(super) fn response_for_dialog_validation_error(
    request: &SipRequest,
    error: &DialogValidationError,
) -> Vec<u8> {
    let (status_code, reason_phrase) = error.status();
    response::build_response_with_owned_headers(
        request,
        status_code,
        reason_phrase,
        &[("X-VOS-RS-Error".to_string(), error.to_string())],
        "",
    )
}

pub(super) fn response_for_media_error(request: &SipRequest, error: &media::MediaError) -> Vec<u8> {
    match error {
        media::MediaError::PortRangeExhausted { .. } => {
            response::service_unavailable_for_request(request, &error.to_string())
        }
        _ => response::not_acceptable_for_request(request, &error.to_string()),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RewrittenSdp {
    pub(crate) original_endpoint: Option<RtpEndpoint>,
    pub(crate) relay_endpoint: RtpEndpoint,
    pub(crate) body: Vec<u8>,
}

pub(super) fn prepare_rewritten_sdp(
    headers: &HeaderMap,
    body: &[u8],
    media_relay: &MediaRelayState,
    media_config: &MediaConfig,
    direction: &'static str,
) -> Result<Option<RewrittenSdp>, media::MediaError> {
    if !media::is_sdp_body(headers, body) {
        return Ok(None);
    }

    media::validate_media_negotiation(body)?;

    let relay_endpoint = media_relay.allocate_endpoint(media_config)?;
    if let Some(codec) = media::negotiated_audio_codec(body) {
        media_relay.register_port_codec(relay_endpoint.port, codec);
    }
    if let Ok(sdp_text) = std::str::from_utf8(body) {
        if let Ok(session) = SessionDescription::parse(sdp_text) {
            if let Ok(crypto_attributes) = session.first_audio_srtp_crypto() {
                if let Some(crypto) = crypto_attributes.first() {
                    media_relay.register_srtp_offer(
                        relay_endpoint.port,
                        &crypto.suite,
                        &crypto.key_params,
                    );
                }
            }
        }
    }
    match media::rewrite_sdp_and_extract_endpoint(body, &relay_endpoint) {
        Ok((body, original_endpoint)) => Ok(Some(RewrittenSdp {
            original_endpoint: Some(original_endpoint),
            relay_endpoint,
            body,
        })),
        Err(error) => {
            media_relay.clear_target(relay_endpoint.port);
            warn!(%error, direction, "failed to rewrite SDP body for media relay");
            Err(error)
        }
    }
}

pub(super) fn register_relay_target(
    media_relay: &MediaRelayState,
    relay_endpoint: &RtpEndpoint,
    target_endpoint: &RtpEndpoint,
    direction: &'static str,
) {
    if let Err(error) = media_relay.set_target(relay_endpoint, target_endpoint) {
        warn!(%error, direction, "failed to register RTP relay target");
    }
}

pub(crate) fn replace_header_value(raw: &str, header_name: &str, new_value: &str) -> String {
    let needle_lower = header_name.to_ascii_lowercase();
    let mut result = String::with_capacity(raw.len() + 8);
    for line in raw.split_inclusive("\r\n") {
        let header_part = line.split(':').next().unwrap_or("");
        if header_part.trim().to_ascii_lowercase() == needle_lower {
            result.push_str(&format!("{header_name}: {new_value}\r\n"));
        } else {
            result.push_str(line);
        }
    }
    result
}

pub(super) fn parse_sip_info_dtmf(content_type: &str, body: &[u8]) -> Option<char> {
    let body_str = std::str::from_utf8(body).ok()?.trim();
    if content_type.contains("application/dtmf-relay") {
        for line in body_str.lines() {
            let line = line.trim();
            if line.to_ascii_lowercase().starts_with("signal=") {
                let parts: Vec<&str> = line.split('=').collect();
                if parts.len() == 2 {
                    let signal = parts[1].trim();
                    if signal.len() == 1 {
                        let c = signal.chars().next()?;
                        if c.is_ascii_digit() || c == '*' || c == '#' || ('A'..='D').contains(&c) {
                            return Some(c);
                        }
                    }
                }
            }
        }
    } else if content_type.contains("application/dtmf") && body_str.len() == 1 {
        let c = body_str.chars().next()?;
        if c.is_ascii_digit() || c == '*' || c == '#' || ('A'..='D').contains(&c) {
            return Some(c);
        }
    }
    None
}

pub(super) fn percent_decode(s: &str) -> String {
    let mut result = String::new();
    let mut chars = s.chars();
    while let Some(ch) = chars.next() {
        if ch == '%' {
            let mut hex = String::new();
            if let Some(h1) = chars.next() {
                hex.push(h1);
            }
            if let Some(h2) = chars.next() {
                hex.push(h2);
            }
            if let Ok(val) = u8::from_str_radix(&hex, 16) {
                result.push(val as char);
            } else {
                result.push('%');
                result.push_str(&hex);
            }
        } else {
            result.push(ch);
        }
    }
    result
}

pub(super) fn proxy_unauthorized_for_request(
    request: &SipRequest,
    auth_config: &AuthConfig,
) -> Vec<u8> {
    let nonce = auth_config.select_nonce();
    let challenge = auth_config.challenge_header_with_nonce(&nonce);
    response::build_response_with_owned_headers(
        request,
        407,
        "Proxy Authentication Required",
        &[("Proxy-Authenticate".to_string(), challenge)],
        "",
    )
}
