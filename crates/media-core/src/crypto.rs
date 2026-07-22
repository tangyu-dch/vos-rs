//! SRTP session wrapper used by media relays.

use rtp_core::{SrtpConfig, SrtpContext, SrtpError};

#[derive(Debug)]
pub struct MediaCryptoSession {
    context: SrtpContext,
}

impl MediaCryptoSession {
    /// Creates a session from an already validated SRTP configuration.
    pub fn new(config: SrtpConfig, ssrc: u32) -> Self {
        Self {
            context: SrtpContext::new(config, ssrc),
        }
    }

    /// Creates a session from an SDES crypto suite and inline key parameters.
    pub fn from_sdes(suite: &str, key_params: &str, ssrc: u32) -> Result<Self, SrtpError> {
        let config = SrtpConfig::from_sdes_key_params(suite, key_params)?;
        Ok(Self::new(config, ssrc))
    }

    /// Encrypts an RTP packet stored in a growable buffer.
    pub fn encrypt(&mut self, packet: &mut Vec<u8>) -> Result<usize, SrtpError> {
        self.context.encrypt_rtp(packet)
    }

    /// Encrypts RTP in a fixed-capacity buffer without allocating on the hot path.
    pub fn encrypt_in_place(
        &mut self,
        buffer: &mut [u8],
        packet_len: usize,
    ) -> Result<usize, SrtpError> {
        self.context.encrypt_rtp_in_place(buffer, packet_len)
    }

    /// Decrypts an SRTP packet in place.
    pub fn decrypt(&mut self, packet: &mut [u8]) -> Result<usize, SrtpError> {
        self.context.decrypt_srtp(packet)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rtp_core::SrtpProfile;

    #[test]
    fn session_round_trips_rtp_payload() {
        let config = SrtpConfig {
            master_key: [7_u8; 16],
            master_salt: [9_u8; 14],
            profile: SrtpProfile::Aes128CmHmacSha1_80,
        };
        let mut sender = MediaCryptoSession::new(config.clone(), 0x0102_0304);
        let mut receiver = MediaCryptoSession::new(config, 0x0102_0304);
        let mut packet = vec![
            0x80, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0x01, 0x02, 0x03, 0x04, 1, 2, 3, 4,
        ];
        let original = packet.clone();

        sender.encrypt(&mut packet).unwrap();
        assert_ne!(packet, original);
        let decrypted_len = receiver.decrypt(&mut packet).unwrap();
        packet.truncate(decrypted_len);

        assert_eq!(packet, original);
    }
}
