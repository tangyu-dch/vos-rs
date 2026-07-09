use call_core::CallError;
use sdp_core::RtpEndpoint;
use sip_core::{HeaderMap, SipRequest};
use tracing::warn;

use crate::sip::dialog::DialogValidationError;
use super::relay::{self, MediaConfig, MediaRelayState};
use crate::sip::response;

pub fn call_error_for_unknown_request(request: &SipRequest) -> CallError {
    match request.headers.get("call-id") {
        Some(call_id) => CallError::UnknownCall(call_id.as_str().to_string()),
        None => CallError::MissingRequiredHeader("Call-ID"),
    }
}

pub fn response_for_dialog_validation_error(
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

pub fn response_for_media_error(request: &SipRequest, error: &relay::MediaError) -> Vec<u8> {
    match error {
        relay::MediaError::PortRangeExhausted { .. } => {
            response::service_unavailable_for_request(request, &error.to_string())
        }
        _ => response::not_acceptable_for_request(request, &error.to_string()),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewrittenSdp {
    pub original_endpoint: Option<RtpEndpoint>,
    pub relay_endpoint: RtpEndpoint,
    pub body: Vec<u8>,
}

pub fn prepare_rewritten_sdp(
    headers: &HeaderMap,
    body: &[u8],
    media_relay: &MediaRelayState,
    media_config: &MediaConfig,
    direction: &'static str,
) -> Result<Option<RewrittenSdp>, relay::MediaError> {
    if !relay::is_sdp_body(headers, body) {
        return Ok(None);
    }

    let relay_endpoint = media_relay.allocate_endpoint(media_config)?;
    match relay::rewrite_sdp_and_extract_endpoint(body, &relay_endpoint) {
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

pub fn register_relay_target(
    media_relay: &MediaRelayState,
    relay_endpoint: &RtpEndpoint,
    target_endpoint: &RtpEndpoint,
    direction: &'static str,
) {
    if let Err(error) = media_relay.set_target(relay_endpoint, target_endpoint) {
        warn!(%error, direction, "failed to register RTP relay target");
    }
}

pub fn replace_header_value(raw: &str, header_name: &str, new_value: &str) -> String {
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

pub fn parse_sip_info_dtmf(content_type: &str, body: &[u8]) -> Option<char> {
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
