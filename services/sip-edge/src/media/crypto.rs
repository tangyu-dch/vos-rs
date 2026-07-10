use rtp_core::{SrtpContext, SrtpConfig, SrtpError};

#[derive(Debug)]
pub struct MediaCryptoSession {
    pub(crate) context: SrtpContext,
}

impl MediaCryptoSession {
    pub fn from_sdes(suite: &str, key_params: &str, ssrc: u32) -> Result<Self, SrtpError> {
        let config = SrtpConfig::from_sdes_key_params(suite, key_params)?;
        Ok(Self {
            context: SrtpContext::new(config, ssrc),
        })
    }

    pub fn encrypt(&mut self, packet: &mut Vec<u8>) -> Result<usize, SrtpError> {
        self.context.encrypt_rtp(packet)
    }

    pub fn decrypt(&mut self, packet: &mut [u8]) -> Result<usize, SrtpError> {
        self.context.decrypt_srtp(packet)
    }
}
