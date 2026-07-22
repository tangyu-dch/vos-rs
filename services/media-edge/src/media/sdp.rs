//! SDP negotiation adapters for the standalone media service.

use crate::media::recording::MediaError;
use media_core::sdp::{self as shared_sdp, MediaNegotiationPolicy, MediaSdpError};
use rtp_core::AudioCodec;
use sdp_core::RtpEndpoint;
use sip_core::HeaderMap;
use std::net::{SocketAddr, ToSocketAddrs};

pub fn is_sdp_body(headers: &HeaderMap, body: &[u8]) -> bool {
    if body.is_empty() {
        return false;
    }
    if let Some(content_type) = headers.get("content-type") {
        let raw = content_type.as_str().as_bytes();
        if raw.len() >= 15 {
            if raw[..15].eq_ignore_ascii_case(b"application/sdp") {
                return true;
            }
            for index in 1..raw.len().saturating_sub(15) {
                if raw[index] == b';' && raw[index + 1] == b' ' {
                    let rest = &raw[index + 2..];
                    if rest.len() >= 15 && rest[..15].eq_ignore_ascii_case(b"application/sdp") {
                        return true;
                    }
                }
            }
        }
    }
    false
}

fn map_sdp_error(error: MediaSdpError) -> MediaError {
    match error {
        MediaSdpError::InvalidUtf8 => MediaError::InvalidUtf8,
        MediaSdpError::Sdp(error) => MediaError::Sdp(error),
    }
}

pub fn validate_media_negotiation(body: &[u8]) -> Result<(), MediaError> {
    shared_sdp::validate_media_negotiation(body, MediaNegotiationPolicy::AUDIO_ONLY)
        .map_err(map_sdp_error)
}

pub fn rewrite_sdp_body(body: &[u8], endpoint: RtpEndpoint) -> Result<Vec<u8>, MediaError> {
    shared_sdp::rewrite_sdp_body(body, endpoint).map_err(map_sdp_error)
}

pub fn rewrite_sdp_and_extract_endpoint(
    body: &[u8],
    relay_endpoint: &RtpEndpoint,
) -> Result<(Vec<u8>, RtpEndpoint), MediaError> {
    shared_sdp::rewrite_sdp_and_extract_endpoint(body, relay_endpoint).map_err(map_sdp_error)
}

pub fn parse_sdp_rtp_endpoint(body: &[u8]) -> Result<RtpEndpoint, MediaError> {
    shared_sdp::parse_sdp_rtp_endpoint(body).map_err(map_sdp_error)
}

pub fn parse_sdp_dtmf_payload_type(body: &[u8]) -> Option<u8> {
    shared_sdp::parse_sdp_dtmf_payload_type(body)
}

pub fn negotiated_audio_codec(body: &[u8]) -> Option<AudioCodec> {
    shared_sdp::negotiated_audio_codec(body)
}

pub fn socket_addr_for_endpoint(endpoint: &RtpEndpoint) -> Result<SocketAddr, MediaError> {
    let target = if endpoint.address.contains(':') {
        format!("[{}]:{}", endpoint.address, endpoint.port)
    } else {
        format!("{}:{}", endpoint.address, endpoint.port)
    };

    target
        .to_socket_addrs()
        .map_err(|_| MediaError::InvalidEndpoint(target.clone()))?
        .next()
        .ok_or(MediaError::InvalidEndpoint(target))
}
