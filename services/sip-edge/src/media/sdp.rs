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
use media_core::sdp::{self as shared_sdp, MediaNegotiationPolicy, MediaSdpError};
use rtp_core::AudioCodec;
use sdp_core::RtpEndpoint;
use sip_core::HeaderMap;
use std::net::{SocketAddr, ToSocketAddrs};
use std::str;

use super::relay::WebRtcSessionDescription;

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

/// 判断 SDP 是否为浏览器 WebRTC 的 DTLS-SRTP 媒体协商。
pub fn is_webrtc_sdp(body: &[u8]) -> bool {
    str::from_utf8(body).is_ok_and(|sdp| {
        let lower = sdp.to_ascii_lowercase();
        lower.contains("udp/tls/rtp/savpf")
            && lower.contains("a=ice-ufrag:")
            && lower.contains("a=fingerprint:")
    })
}

/// 将 WebRTC Offer 转为传统 SIP 网关可处理的 RTP/AVP Offer。
pub fn rewrite_webrtc_offer_for_legacy(
    body: &[u8],
    relay_endpoint: &RtpEndpoint,
) -> Result<Vec<u8>, MediaError> {
    let (rewritten, _) = rewrite_sdp_and_extract_endpoint(body, relay_endpoint)?;
    let input = str::from_utf8(&rewritten).map_err(|_| MediaError::InvalidUtf8)?;
    let mut output = Vec::with_capacity(rewritten.len());
    for line in input.lines() {
        let trimmed = line.trim_end_matches('\r');
        let lower = trimmed.to_ascii_lowercase();
        if is_webrtc_only_attribute(&lower) {
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("m=audio ") {
            let mut fields = rest.split_whitespace();
            let port = fields.next().unwrap_or("0");
            let _protocol = fields.next();
            let payloads = fields.collect::<Vec<_>>().join(" ");
            output.extend_from_slice(format!("m=audio {port} RTP/AVP {payloads}\r\n").as_bytes());
        } else {
            output.extend_from_slice(trimmed.as_bytes());
            output.extend_from_slice(b"\r\n");
        }
    }
    Ok(output)
}

fn is_webrtc_only_attribute(lower_line: &str) -> bool {
    [
        "a=ice-",
        "a=candidate:",
        "a=end-of-candidates",
        "a=fingerprint:",
        "a=setup:",
        "a=rtcp:",
        "a=rtcp-mux",
        "a=rtcp-rsize",
        "a=rtcp-fb:",
        "a=extmap:",
        "a=extmap-allow-mixed",
        "a=msid:",
        "a=ssrc:",
        "a=ssrc-group:",
        "a=mid:",
        "a=group:",
        "a=msid-semantic:",
        "a=rid:",
        "a=simulcast:",
    ]
    .iter()
    .any(|prefix| lower_line.starts_with(prefix))
}

/// 基于网关应答中的编解码器生成浏览器可接受的 ICE-Lite/DTLS-SRTP Answer。
pub fn build_webrtc_answer(
    gateway_answer: &[u8],
    relay_endpoint: &RtpEndpoint,
    session: &WebRtcSessionDescription,
) -> Result<Vec<u8>, MediaError> {
    let input = str::from_utf8(gateway_answer).map_err(|_| MediaError::InvalidUtf8)?;
    let media_lines = answer_media_lines(input);
    if media_lines.payloads.is_empty() {
        return Err(MediaError::Sdp(
            sdp_core::SdpError::MissingCompatibleAudioCodec,
        ));
    }
    let address_type = if relay_endpoint.address.contains(':') {
        "IP6"
    } else {
        "IP4"
    };
    let mut answer = format!(
        "v=0\r\no=vos-rs 1 1 IN {address_type} {address}\r\ns=vos-rs-webrtc\r\n\
         c=IN {address_type} {address}\r\nt=0 0\r\na=ice-lite\r\n\
         m=audio {port} UDP/TLS/RTP/SAVPF {payloads}\r\n",
        address = relay_endpoint.address,
        port = relay_endpoint.port,
        payloads = media_lines.payloads,
    );
    for line in media_lines.attributes {
        answer.push_str(&line);
        answer.push_str("\r\n");
    }
    answer.push_str(&format!(
        "a=ice-ufrag:{}\r\na=ice-pwd:{}\r\na=fingerprint:sha-256 {}\r\n\
         a=setup:{}\r\na=rtcp-mux\r\na=sendrecv\r\n\
         a=candidate:1 1 UDP 2130706431 {} {} typ host\r\na=end-of-candidates\r\n",
        session.ice.username_fragment,
        session.ice.password,
        session.fingerprint_sha256,
        session.dtls_setup,
        relay_endpoint.address,
        relay_endpoint.port,
    ));
    Ok(answer.into_bytes())
}

struct AnswerMediaLines {
    payloads: String,
    attributes: Vec<String>,
}

fn answer_media_lines(input: &str) -> AnswerMediaLines {
    let mut payloads = String::new();
    let mut attributes = Vec::new();
    let mut in_audio = false;
    for line in input.lines() {
        let trimmed = line.trim_end_matches('\r');
        if let Some(rest) = trimmed.strip_prefix("m=audio ") {
            let fields = rest.split_whitespace().collect::<Vec<_>>();
            if fields.len() > 2 {
                payloads = fields[2..].join(" ");
            }
            in_audio = true;
        } else if trimmed.starts_with("m=") {
            in_audio = false;
        } else if in_audio && (trimmed.starts_with("a=rtpmap:") || trimmed.starts_with("a=fmtp:")) {
            attributes.push(trimmed.to_string());
        }
    }
    AnswerMediaLines {
        payloads,
        attributes,
    }
}

fn map_sdp_error(error: MediaSdpError) -> MediaError {
    match error {
        MediaSdpError::InvalidUtf8 => MediaError::InvalidUtf8,
        MediaSdpError::Sdp(error) => MediaError::Sdp(error),
    }
}

pub fn validate_media_negotiation(body: &[u8]) -> Result<(), MediaError> {
    shared_sdp::validate_media_negotiation(body, MediaNegotiationPolicy::AUDIO_OR_T38)
        .map_err(map_sdp_error)
}

#[allow(dead_code)]
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

#[cfg(test)]
mod webrtc_tests {
    use super::*;
    use crate::media::relay::{WebRtcIceCredentials, WebRtcSessionDescription};

    const WEBRTC_OFFER: &str = "v=0\r\n\
o=- 1 1 IN IP4 0.0.0.0\r\ns=-\r\nc=IN IP4 0.0.0.0\r\nt=0 0\r\n\
m=audio 9 UDP/TLS/RTP/SAVPF 111 0 8 101\r\n\
a=rtpmap:111 opus/48000/2\r\na=rtpmap:0 PCMU/8000\r\na=rtpmap:8 PCMA/8000\r\n\
a=rtpmap:101 telephone-event/8000\r\na=ice-ufrag:browser\r\na=ice-pwd:secret\r\n\
a=fingerprint:sha-256 AA:BB\r\na=setup:actpass\r\na=rtcp-mux\r\n\
a=candidate:1 1 UDP 1 192.0.2.1 50000 typ host\r\n";

    #[test]
    fn web_rtc_offer_is_converted_to_legacy_rtp() {
        assert!(is_webrtc_sdp(WEBRTC_OFFER.as_bytes()));
        let endpoint = RtpEndpoint::new("203.0.113.10".to_string(), 40_000);
        let result = rewrite_webrtc_offer_for_legacy(WEBRTC_OFFER.as_bytes(), &endpoint).unwrap();
        let result = String::from_utf8(result).unwrap();
        assert!(result.contains("m=audio 40000 RTP/AVP 111 0 8 101"));
        assert!(result.contains("c=IN IP4 203.0.113.10"));
        assert!(!result.contains("a=ice-ufrag"));
        assert!(!result.contains("a=fingerprint"));
        assert!(!result.contains("a=candidate"));
    }

    #[test]
    fn gateway_answer_is_converted_to_webrtc_answer() {
        let gateway = b"v=0\r\no=- 1 1 IN IP4 192.0.2.2\r\ns=-\r\nc=IN IP4 192.0.2.2\r\nt=0 0\r\nm=audio 9000 RTP/AVP 0 101\r\na=rtpmap:0 PCMU/8000\r\na=rtpmap:101 telephone-event/8000\r\n";
        let endpoint = RtpEndpoint::new("203.0.113.10".to_string(), 40_002);
        let session = WebRtcSessionDescription {
            ice: WebRtcIceCredentials {
                username_fragment: "server".to_string(),
                password: "server-password".to_string(),
            },
            fingerprint_sha256: "AA:BB:CC".to_string(),
            dtls_setup: "passive".to_string(),
        };
        let answer = build_webrtc_answer(gateway, &endpoint, &session).unwrap();
        let answer = String::from_utf8(answer).unwrap();
        assert!(answer.contains("m=audio 40002 UDP/TLS/RTP/SAVPF 0 101"));
        assert!(answer.contains("a=ice-lite"));
        assert!(answer.contains("a=fingerprint:sha-256 AA:BB:CC"));
        assert!(answer.contains("a=setup:passive"));
        assert!(answer.contains("a=rtcp-mux"));
    }
}
