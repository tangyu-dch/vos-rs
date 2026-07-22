//! Shared SDP media negotiation helpers.

use rtp_core::AudioCodec;
use sdp_core::{RtpEndpoint, SdpError, SessionDescription};
use std::{error::Error, fmt, str};

/// Controls which media families are accepted during SDP validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MediaNegotiationPolicy {
    allow_t38: bool,
}

impl MediaNegotiationPolicy {
    /// Accepts the audio codecs supported by the media relay.
    pub const AUDIO_ONLY: Self = Self { allow_t38: false };

    /// Accepts supported audio codecs as well as T.38/UDPTL negotiation.
    pub const AUDIO_OR_T38: Self = Self { allow_t38: true };
}

/// Errors raised while parsing or rewriting SDP media bodies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaSdpError {
    /// The SDP body is not valid UTF-8.
    InvalidUtf8,
    /// The SDP protocol parser rejected the body.
    Sdp(SdpError),
}

impl fmt::Display for MediaSdpError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidUtf8 => write!(formatter, "SDP body is not valid UTF-8"),
            Self::Sdp(error) => write!(formatter, "{error}"),
        }
    }
}

impl Error for MediaSdpError {}

impl From<SdpError> for MediaSdpError {
    fn from(error: SdpError) -> Self {
        Self::Sdp(error)
    }
}

/// Validates that an SDP body contains a supported media family.
pub fn validate_media_negotiation(
    body: &[u8],
    policy: MediaNegotiationPolicy,
) -> Result<(), MediaSdpError> {
    let input = str::from_utf8(body).map_err(|_| MediaSdpError::InvalidUtf8)?;
    let upper = input.to_ascii_uppercase();
    let has_audio = ["PCMU", "PCMA", "OPUS", "G722", "G729"]
        .iter()
        .any(|codec| upper.contains(codec));
    let has_t38 = policy.allow_t38 && (upper.contains("T38") || upper.contains("UDPTL"));
    if !has_audio && !has_t38 {
        return Err(SdpError::MissingCompatibleAudioCodec.into());
    }
    Ok(())
}

/// Rewrites the first audio endpoint while retaining supported payloads.
pub fn rewrite_sdp_body(body: &[u8], endpoint: RtpEndpoint) -> Result<Vec<u8>, MediaSdpError> {
    let input = str::from_utf8(body).map_err(|_| MediaSdpError::InvalidUtf8)?;
    if let Some(result) = try_fast_rewrite(input, &endpoint) {
        return Ok(result);
    }

    let mut session = SessionDescription::parse(input)?;
    let payloads = compatible_audio_payloads(&session)?;
    session.retain_first_audio_rtp_payloads(&payloads)?;
    session.rewrite_first_audio_rtp_endpoint(endpoint)?;
    Ok(session.to_bytes())
}

/// Extracts the original endpoint and rewrites it to the relay endpoint.
pub fn rewrite_sdp_and_extract_endpoint(
    body: &[u8],
    relay_endpoint: &RtpEndpoint,
) -> Result<(Vec<u8>, RtpEndpoint), MediaSdpError> {
    let input = str::from_utf8(body).map_err(|_| MediaSdpError::InvalidUtf8)?;
    if let Some(result) = try_fast_rewrite_inner(input, relay_endpoint) {
        return Ok(result);
    }

    let mut session = SessionDescription::parse(input)?;
    let original_endpoint = session.first_audio_rtp_endpoint()?;
    let payloads = compatible_audio_payloads(&session)?;
    session.retain_first_audio_rtp_payloads(&payloads)?;
    session.rewrite_first_audio_rtp_endpoint(relay_endpoint.clone())?;
    Ok((session.to_bytes(), original_endpoint))
}

fn try_fast_rewrite(input: &str, endpoint: &RtpEndpoint) -> Option<Vec<u8>> {
    try_fast_rewrite_inner(input, endpoint).map(|(bytes, _)| bytes)
}

fn try_fast_rewrite_inner(input: &str, endpoint: &RtpEndpoint) -> Option<(Vec<u8>, RtpEndpoint)> {
    let upper = input.to_ascii_uppercase();
    if ["PCMU", "PCMA", "OPUS", "G722", "G729"]
        .iter()
        .all(|codec| !upper.contains(codec))
    {
        return None;
    }

    let mut found_audio_m = false;
    let mut session_c_rewritten = false;
    let mut result = Vec::with_capacity(input.len() + 64);
    let mut in_audio_section = false;
    let mut original_port = None;
    let mut original_addr = None;

    for line in input.lines() {
        let trimmed = line.trim_end_matches('\r');
        if trimmed.starts_with("m=audio ") {
            found_audio_m = true;
            in_audio_section = true;
            if let Some(rest) = trimmed.get(8..) {
                if let Some(space2) = rest.find(' ') {
                    if original_port.is_none() {
                        original_port = rest[..space2].parse().ok();
                    }
                    result.extend_from_slice(b"m=audio ");
                    result.extend_from_slice(endpoint.port.to_string().as_bytes());
                    result.extend_from_slice(&rest.as_bytes()[space2..]);
                    result.extend_from_slice(b"\r\n");
                    continue;
                }
            }
        } else if trimmed.starts_with("m=") {
            in_audio_section = false;
        }

        if trimmed.starts_with("c=IN IP") {
            if original_addr.is_none() {
                original_addr = trimmed
                    .get(7..)
                    .and_then(|rest| rest.split_whitespace().nth(1))
                    .map(str::to_string);
            }
            if in_audio_section || (!found_audio_m && !session_c_rewritten) {
                if !found_audio_m {
                    session_c_rewritten = true;
                }
                let address_type = if endpoint.address.contains(':') {
                    "IP6"
                } else {
                    "IP4"
                };
                result.extend_from_slice(b"c=IN ");
                result.extend_from_slice(address_type.as_bytes());
                result.extend_from_slice(b" ");
                result.extend_from_slice(endpoint.address.as_bytes());
                result.extend_from_slice(b"\r\n");
                continue;
            }
        }

        result.extend_from_slice(line.as_bytes());
        if !line.ends_with('\n') {
            result.extend_from_slice(b"\r\n");
        }
    }

    found_audio_m.then(|| {
        let original_endpoint = RtpEndpoint {
            address: original_addr.unwrap_or_else(|| "0.0.0.0".to_string()),
            port: original_port.unwrap_or(0),
        };
        (result, original_endpoint)
    })
}

/// Parses the first audio RTP endpoint from an SDP body.
pub fn parse_sdp_rtp_endpoint(body: &[u8]) -> Result<RtpEndpoint, MediaSdpError> {
    let input = str::from_utf8(body).map_err(|_| MediaSdpError::InvalidUtf8)?;
    if let Some(endpoint) = try_fast_parse_endpoint(input) {
        return Ok(endpoint);
    }
    Ok(SessionDescription::parse(input)?.first_audio_rtp_endpoint()?)
}

fn try_fast_parse_endpoint(input: &str) -> Option<RtpEndpoint> {
    let mut audio_port = None;
    let mut connection_addr = None;
    let mut in_audio_section = false;
    for line in input.lines() {
        let trimmed = line.trim_end_matches('\r');
        if let Some(rest) = trimmed.strip_prefix("m=audio ") {
            audio_port = Some(rest.split_whitespace().next()?.parse().ok()?);
            in_audio_section = true;
        } else if trimmed.starts_with("m=") {
            in_audio_section = false;
        } else if trimmed.starts_with("c=IN IP")
            && (in_audio_section || (audio_port.is_none() && connection_addr.is_none()))
        {
            connection_addr = trimmed[7..].split_whitespace().nth(1);
        }
    }
    Some(RtpEndpoint {
        address: connection_addr.unwrap_or("0.0.0.0").to_string(),
        port: audio_port?,
    })
}

/// Returns the RFC 4733 payload type negotiated at an 8 kHz clock rate.
pub fn parse_sdp_dtmf_payload_type(body: &[u8]) -> Option<u8> {
    let input = str::from_utf8(body).ok()?;
    let formats = SessionDescription::parse(input)
        .ok()?
        .first_audio_rtp_formats()
        .ok()?;
    for format in formats {
        if format.encoding_name.as_deref().is_some_and(|name| {
            name.eq_ignore_ascii_case("telephone-event") && format.clock_rate == Some(8000)
        }) {
            return format.payload_type.parse::<u8>().ok();
        }
    }
    None
}

fn compatible_audio_payloads(session: &SessionDescription) -> Result<Vec<String>, MediaSdpError> {
    let formats = session.first_audio_rtp_formats()?;
    let mut payloads = Vec::with_capacity(formats.len());
    let mut has_voice = false;
    for format in &formats {
        let is_voice = audio_codec_for_format(format).is_some();
        if is_voice {
            has_voice = true;
            payloads.push(format.payload_type.clone());
        } else if format.encoding_name.as_deref().is_some_and(|name| {
            name.eq_ignore_ascii_case("telephone-event") && format.clock_rate == Some(8000)
        }) {
            payloads.push(format.payload_type.clone());
        }
    }
    Ok(if has_voice { payloads } else { Vec::new() })
}

fn audio_codec_for_format(format: &sdp_core::AudioFormat) -> Option<AudioCodec> {
    match (format.encoding_name.as_deref(), format.clock_rate) {
        (Some(name), Some(rate)) => AudioCodec::from_rtpmap(name, rate),
        _ => format
            .payload_type
            .parse::<u8>()
            .ok()
            .and_then(AudioCodec::from_static_payload_type),
    }
}

/// Returns the first supported audio codec offered by an SDP body.
pub fn negotiated_audio_codec(body: &[u8]) -> Option<AudioCodec> {
    let input = str::from_utf8(body).ok()?;
    SessionDescription::parse(input)
        .ok()?
        .first_audio_rtp_formats()
        .ok()?
        .iter()
        .find_map(audio_codec_for_format)
}
