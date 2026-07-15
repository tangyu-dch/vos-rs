//! WebRTC ICE-Lite、DTLS 与 SRTP 媒体会话。

mod dtls;
mod ice;
mod session;
mod srtp;

pub use ice::IceCredentials;
pub use session::{WebRtcSession, WebRtcSessionDescription};

/// RFC 7983 定义的 DTLS 数据包识别。
pub fn is_dtls_packet(packet: &[u8]) -> bool {
    packet
        .first()
        .is_some_and(|value| (20..=63).contains(value))
}

/// RFC 7983 定义的 STUN 数据包识别。
pub fn is_stun_packet(packet: &[u8]) -> bool {
    stun::message::is_message(packet)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packet_demultiplexing_requires_stun_magic_cookie() {
        assert!(is_dtls_packet(&[22, 0xfe, 0xfd]));
        assert!(!is_dtls_packet(&[0x80, 0x60]));

        let mut stun = vec![0_u8; 20];
        stun[4..8].copy_from_slice(&stun::message::MAGIC_COOKIE.to_be_bytes());
        assert!(is_stun_packet(&stun));
        assert!(!is_stun_packet(&[0, 1, 2]));
    }
}
