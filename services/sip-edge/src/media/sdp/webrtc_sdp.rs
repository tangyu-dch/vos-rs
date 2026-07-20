//! WebRTC SDP 与 传统 SIP SDP 互相转换工具模块
//! 
//! 本模块实现将 WebRTC 浏览器终端发送的 Offer/Answer SDP（带有 ICE ufrag/pwd, DTLS fingerprint, a=setup 等）
//! 转换为传统 SIP 软交换能够理解与改写的标准 200 OK / INVITE SDP，
//! 同时能够将标准 SIP 软交换返回的 SDP 封装为包含 ICE/DTLS 参数的 WebRTC 响应 SDP。

use sdp_core::{SdpSession, SdpError};

/// 判断一个 SDP 字符串是否来源于 WebRTC 浏览器终端
pub fn is_webrtc_sdp(sdp: &str) -> bool {
    sdp.contains("a=ice-ufrag:") || sdp.contains("a=fingerprint:") || sdp.contains("a=setup:")
}

/// 将 WebRTC 浏览器发起的 SDP Offer 转化为适合 SIP 软交换处理的标准 SDP Offer
pub fn convert_webrtc_offer_to_sip(webrtc_sdp: &str) -> Result<String, SdpError> {
    let session = SdpSession::parse(webrtc_sdp)?;
    
    // 如果不含音频媒体，直接原样返回
    let Some(audio_media) = session.media.iter().find(|m| m.is_audio()) else {
        return Ok(webrtc_sdp.to_string());
    };

    let connection_ip = session.connection_ip().unwrap_or("0.0.0.0");
    let port = audio_media.port;

    // 构建过滤/净化后的 SIP SDP
    let mut sip_sdp = String::with_capacity(webrtc_sdp.len());
    sip_sdp.push_str("v=0\r\n");
    sip_sdp.push_str(&format!("o=- {} {} IN IP4 {}\r\n", session.origin_session_id, session.origin_session_version, connection_ip));
    sip_sdp.push_str("s=Vos-rs WebRTC Bridge\r\n");
    sip_sdp.push_str(&format!("c=IN IP4 {}\r\n", connection_ip));
    sip_sdp.push_str("t=0 0\r\n");

    // 重新拼接音频媒体行，优先保留 Opus (111) 与 PCMA (8) / PCMU (0)
    let formats = audio_media.formats.join(" ");
    sip_sdp.push_str(&format!("m=audio {} RTP/AVP {}\r\n", port, formats));
    sip_sdp.push_str("a=sendrecv\r\n");

    if formats.contains("111") {
        sip_sdp.push_str("a=rtpmap:111 opus/48000/2\r\n");
    }
    if formats.contains("8") {
        sip_sdp.push_str("a=rtpmap:8 PCMA/8000\r\n");
    }
    if formats.contains("0") {
        sip_sdp.push_str("a=rtpmap:0 PCMU/8000\r\n");
    }

    Ok(sip_sdp)
}

/// 将 SIP 落地中继/被叫返回的标准 SDP 转换为带有 DTLS/ICE 参数的 WebRTC SDP Answer
pub fn convert_sip_answer_to_webrtc(
    sip_sdp: &str,
    ice_ufrag: &str,
    ice_pwd: &str,
    dtls_fingerprint: &str,
) -> Result<String, SdpError> {
    let session = SdpSession::parse(sip_sdp)?;
    let Some(audio_media) = session.media.iter().find(|m| m.is_audio()) else {
        return Ok(sip_sdp.to_string());
    };

    let connection_ip = session.connection_ip().unwrap_or("0.0.0.0");
    let port = audio_media.port;

    let mut webrtc_sdp = String::with_capacity(sip_sdp.len() + 256);
    webrtc_sdp.push_str("v=0\r\n");
    webrtc_sdp.push_str(&format!("o=- {} {} IN IP4 {}\r\n", session.origin_session_id, session.origin_session_version, connection_ip));
    webrtc_sdp.push_str("s=Vos-rs WebRTC Bridge\r\n");
    webrtc_sdp.push_str(&format!("c=IN IP4 {}\r\n", connection_ip));
    webrtc_sdp.push_str("t=0 0\r\n");

    let formats = audio_media.formats.join(" ");
    webrtc_sdp.push_str(&format!("m=audio {} UDP/TLS/RTP/SAVPF {}\r\n", port, formats));
    webrtc_sdp.push_str("a=sendrecv\r\n");
    webrtc_sdp.push_str(&format!("a=ice-ufrag:{}\r\n", ice_ufrag));
    webrtc_sdp.push_str(&format!("a=ice-pwd:{}\r\n", ice_pwd));
    webrtc_sdp.push_str(&format!("a=fingerprint:{}\r\n", dtls_fingerprint));
    webrtc_sdp.push_str("a=setup:active\r\n");
    webrtc_sdp.push_str(&format!("a=candidate:1 1 UDP 2130706431 {} {} typ host\r\n", connection_ip, port));

    if formats.contains("111") {
        webrtc_sdp.push_str("a=rtpmap:111 opus/48000/2\r\n");
    }
    if formats.contains("8") {
        webrtc_sdp.push_str("a=rtpmap:8 PCMA/8000\r\n");
    }
    if formats.contains("0") {
        webrtc_sdp.push_str("a=rtpmap:0 PCMU/8000\r\n");
    }

    Ok(webrtc_sdp)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_webrtc_sdp_detection() {
        let webrtc = "v=0\r\no=- 123 456 IN IP4 127.0.0.1\r\na=ice-ufrag:abcd\r\na=fingerprint:sha-256 00:11\r\n";
        let sip = "v=0\r\no=- 123 456 IN IP4 127.0.0.1\r\nm=audio 5002 RTP/AVP 0 8\r\n";
        assert!(is_webrtc_sdp(webrtc));
        assert!(!is_webrtc_sdp(sip));
    }

    #[test]
    fn test_convert_webrtc_offer_to_sip() {
        let webrtc = "v=0\r\no=- 100 200 IN IP4 192.168.1.5\r\nc=IN IP4 192.168.1.5\r\na=ice-ufrag:test\r\nm=audio 40000 UDP/TLS/RTP/SAVPF 111 8\r\n";
        let sip_sdp = convert_webrtc_offer_to_sip(webrtc).unwrap();
        assert!(sip_sdp.contains("RTP/AVP"));
        assert!(sip_sdp.contains("opus/48000/2"));
    }
}
