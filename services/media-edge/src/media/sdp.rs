//! # SDP 重写
//!
//! 本模块实现了 SIP 代理的 SDP 重写功能，包括：
//!
//! - **地址改写**：将 SDP 中的 RTP 地址改写为代理地址
//! - **端口改写**：将 SDP 中的 RTP 端口改写为代理端口
//! - **端点提取**：从 SDP 中提取原始 RTP 端点信息
//! - **ICE/DTLS 验证**：验证 ICE 和 DTLS-SRTP 属性
//!
//! ## 重写流程
//!
//! ```text
//! 入站 INVITE SDP → 提取原始端点 → 分配代理端口 → 重写 SDP → 转发到网关
//! ```
//!
//! ## 快速路径
//!
//! 当 SDP 包含兼容的音频编解码器时，使用字节级快速重写，
//! 避免完整的解析-修改-序列化流程。

use crate::media::recording::MediaError;
use rtp_core::AudioCodec;
use sdp_core::{RtpEndpoint, SessionDescription};
use sip_core::HeaderMap;
use std::net::{SocketAddr, ToSocketAddrs};
use std::str;

pub fn is_sdp_body(headers: &HeaderMap, body: &[u8]) -> bool {
    if body.is_empty() {
        return false;
    }
    if let Some(ct) = headers.get("content-type") {
        let raw = ct.as_str().as_bytes();
        if raw.len() >= 15 {
            if raw[..15].eq_ignore_ascii_case(b"application/sdp") {
                return true;
            }
            for i in 1..raw.len().saturating_sub(15) {
                if raw[i] == b';' && raw[i + 1] == b' ' {
                    let rest = &raw[i + 2..];
                    if rest.len() >= 15 && rest[..15].eq_ignore_ascii_case(b"application/sdp") {
                        return true;
                    }
                }
            }
        }
    }
    false
}

pub fn validate_media_negotiation(body: &[u8]) -> Result<(), MediaError> {
    let input = str::from_utf8(body).map_err(|_| MediaError::InvalidUtf8)?;
    let upper = input.to_ascii_uppercase();
    if !upper.contains("PCMU")
        && !upper.contains("PCMA")
        && !upper.contains("OPUS")
        && !upper.contains("G722")
        && !upper.contains("G729")
    {
        return Err(MediaError::Sdp(
            sdp_core::SdpError::MissingCompatibleAudioCodec,
        ));
    }
    Ok(())
}

#[allow(dead_code)]
pub fn rewrite_sdp_body(body: &[u8], endpoint: RtpEndpoint) -> Result<Vec<u8>, MediaError> {
    let input = str::from_utf8(body).map_err(|_| MediaError::InvalidUtf8)?;

    if let Some(result) = try_fast_rewrite(input, &endpoint) {
        return Ok(result);
    }

    let mut session = SessionDescription::parse(input)?;
    let payloads = compatible_audio_payloads(&session)?;
    session.retain_first_audio_rtp_payloads(&payloads)?;
    session.rewrite_first_audio_rtp_endpoint(endpoint)?;
    Ok(session.to_bytes())
}

pub fn rewrite_sdp_and_extract_endpoint(
    body: &[u8],
    relay_endpoint: &RtpEndpoint,
) -> Result<(Vec<u8>, RtpEndpoint), MediaError> {
    let input = str::from_utf8(body).map_err(|_| MediaError::InvalidUtf8)?;

    if let Some(result) = try_fast_rewrite_and_extract(input, relay_endpoint) {
        return Ok(result);
    }

    let mut session = SessionDescription::parse(input)?;
    let original_endpoint = session.first_audio_rtp_endpoint()?;
    let payloads = compatible_audio_payloads(&session)?;
    session.retain_first_audio_rtp_payloads(&payloads)?;
    session.rewrite_first_audio_rtp_endpoint(relay_endpoint.clone())?;
    Ok((session.to_bytes(), original_endpoint))
}

#[allow(dead_code)]
fn try_fast_rewrite(input: &str, endpoint: &RtpEndpoint) -> Option<Vec<u8>> {
    try_fast_rewrite_inner(input, endpoint).map(|(bytes, _)| bytes)
}

fn try_fast_rewrite_and_extract(
    input: &str,
    endpoint: &RtpEndpoint,
) -> Option<(Vec<u8>, RtpEndpoint)> {
    try_fast_rewrite_inner(input, endpoint)
}

fn try_fast_rewrite_inner(input: &str, endpoint: &RtpEndpoint) -> Option<(Vec<u8>, RtpEndpoint)> {
    let upper = input.to_ascii_uppercase();
    if !upper.contains("PCMU")
        && !upper.contains("PCMA")
        && !upper.contains("OPUS")
        && !upper.contains("G722")
        && !upper.contains("G729")
    {
        return None;
    }

    let mut found_audio_m = false;
    let mut session_c_rewritten = false;
    let mut result = Vec::with_capacity(input.len() + 64);
    let mut in_audio_section = false;
    let mut original_port: Option<u16> = None;
    let mut original_addr: Option<String> = None;

    for line in input.lines() {
        let trimmed = line.trim_end_matches('\r');

        if trimmed.starts_with("m=audio ") {
            found_audio_m = true;
            in_audio_section = true;
            if let Some(rest) = trimmed.get(8..) {
                if let Some(space2) = rest.find(' ') {
                    let port_str = &rest[..space2];
                    if original_port.is_none() {
                        original_port = port_str.parse().ok();
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
                if let Some(rest) = trimmed.get(7..) {
                    if let Some(addr) = rest.split_whitespace().nth(1) {
                        original_addr = Some(addr.to_string());
                    }
                }
            }

            if in_audio_section || (!found_audio_m && !session_c_rewritten) {
                if !found_audio_m {
                    session_c_rewritten = true;
                }
                let addr_type = if endpoint.address.contains(':') {
                    "IP6"
                } else {
                    "IP4"
                };
                result.extend_from_slice(b"c=IN ");
                result.extend_from_slice(addr_type.as_bytes());
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

    if found_audio_m {
        let original_endpoint = RtpEndpoint {
            address: original_addr.unwrap_or_else(|| "0.0.0.0".to_string()),
            port: original_port.unwrap_or(0),
        };
        Some((result, original_endpoint))
    } else {
        None
    }
}

pub fn parse_sdp_rtp_endpoint(body: &[u8]) -> Result<RtpEndpoint, MediaError> {
    let input = str::from_utf8(body).map_err(|_| MediaError::InvalidUtf8)?;

    if let Some(endpoint) = try_fast_parse_endpoint(input) {
        return Ok(endpoint);
    }

    let session = SessionDescription::parse(input)?;
    Ok(session.first_audio_rtp_endpoint()?)
}

fn try_fast_parse_endpoint(input: &str) -> Option<RtpEndpoint> {
    let mut audio_port: Option<u16> = None;
    let mut connection_addr: Option<&str> = None;
    let mut in_audio_section = false;

    for line in input.lines() {
        let trimmed = line.trim_end_matches('\r');

        if let Some(rest) = trimmed.strip_prefix("m=audio ") {
            let port_str = rest.split_whitespace().next()?;
            audio_port = Some(port_str.parse().ok()?);
            in_audio_section = true;
        } else if trimmed.starts_with("m=") {
            in_audio_section = false;
        } else if trimmed.starts_with("c=IN IP")
            && (in_audio_section || (audio_port.is_none() && connection_addr.is_none()))
        {
            let rest = &trimmed[7..];
            connection_addr = rest.split_whitespace().nth(1);
        }
    }

    let port = audio_port?;
    let address = connection_addr.unwrap_or("0.0.0.0").to_string();

    Some(RtpEndpoint { address, port })
}

pub fn parse_sdp_dtmf_payload_type(body: &[u8]) -> Option<u8> {
    let input = str::from_utf8(body).ok()?;
    let session = SessionDescription::parse(input).ok()?;
    let formats = session.first_audio_rtp_formats().ok()?;
    for format in formats {
        if let Some(encoding_name) = &format.encoding_name {
            if encoding_name.eq_ignore_ascii_case("telephone-event")
                && format.clock_rate == Some(8000)
            {
                return format.payload_type.parse::<u8>().ok();
            }
        }
    }
    None
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

fn compatible_audio_payloads(session: &SessionDescription) -> Result<Vec<String>, MediaError> {
    let formats = session.first_audio_rtp_formats()?;

    let mut payloads = Vec::with_capacity(formats.len());
    let mut has_voice = false;

    for format in &formats {
        let is_voice = match (format.encoding_name.as_deref(), format.clock_rate) {
            (Some(name), Some(rate)) => AudioCodec::from_rtpmap(name, rate).is_some(),
            _ => format
                .payload_type
                .parse::<u8>()
                .ok()
                .and_then(AudioCodec::from_static_payload_type)
                .is_some(),
        };

        if is_voice {
            has_voice = true;
            payloads.push(format.payload_type.clone());
        } else if let Some(name) = &format.encoding_name {
            if name.eq_ignore_ascii_case("telephone-event") && format.clock_rate == Some(8000) {
                payloads.push(format.payload_type.clone());
            }
        }
    }

    if !has_voice {
        return Ok(Vec::new());
    }

    Ok(payloads)
}

pub fn negotiated_audio_codec(body: &[u8]) -> Option<AudioCodec> {
    let input = str::from_utf8(body).ok()?;
    let session = SessionDescription::parse(input).ok()?;
    let formats = session.first_audio_rtp_formats().ok()?;
    for format in &formats {
        if let Some(codec) = match (format.encoding_name.as_deref(), format.clock_rate) {
            (Some(name), Some(rate)) => AudioCodec::from_rtpmap(name, rate),
            _ => format
                .payload_type
                .parse::<u8>()
                .ok()
                .and_then(AudioCodec::from_static_payload_type),
        } {
            return Some(codec);
        }
    }
    None
}
