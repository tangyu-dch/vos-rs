use tokio::sync::Mutex;
use webrtc_srtp::{
    config::Config,
    context::Context,
    option::{srtcp_replay_protection, srtp_replay_protection},
    protection_profile::ProtectionProfile,
};

/// 一条 WebRTC 媒体腿的双向 SRTP/SRTCP 上下文。
pub(super) struct SrtpContexts {
    inbound: Mutex<Context>,
    outbound: Mutex<Context>,
}

impl SrtpContexts {
    pub(super) fn from_config(config: Config) -> Result<Self, String> {
        let inbound = Context::new(
            &config.keys.remote_master_key,
            &config.keys.remote_master_salt,
            config.profile,
            config.remote_rtp_options,
            config.remote_rtcp_options,
        )
        .map_err(|error| error.to_string())?;
        let outbound = Context::new(
            &config.keys.local_master_key,
            &config.keys.local_master_salt,
            config.profile,
            config.local_rtp_options,
            config.local_rtcp_options,
        )
        .map_err(|error| error.to_string())?;
        Ok(Self {
            inbound: Mutex::new(inbound),
            outbound: Mutex::new(outbound),
        })
    }

    pub(super) async fn decrypt_rtp(&self, packet: &[u8]) -> Result<Vec<u8>, String> {
        self.inbound
            .lock()
            .await
            .decrypt_rtp(packet)
            .map(|bytes| bytes.to_vec())
            .map_err(|error| error.to_string())
    }

    pub(super) async fn encrypt_rtp(&self, packet: &[u8]) -> Result<Vec<u8>, String> {
        self.outbound
            .lock()
            .await
            .encrypt_rtp(packet)
            .map(|bytes| bytes.to_vec())
            .map_err(|error| error.to_string())
    }

    pub(super) async fn decrypt_rtcp(&self, packet: &[u8]) -> Result<Vec<u8>, String> {
        self.inbound
            .lock()
            .await
            .decrypt_rtcp(packet)
            .map(|bytes| bytes.to_vec())
            .map_err(|error| error.to_string())
    }

    pub(super) async fn encrypt_rtcp(&self, packet: &[u8]) -> Result<Vec<u8>, String> {
        self.outbound
            .lock()
            .await
            .encrypt_rtcp(packet)
            .map(|bytes| bytes.to_vec())
            .map_err(|error| error.to_string())
    }
}

pub(super) fn default_config() -> Config {
    Config {
        profile: ProtectionProfile::Aes128CmHmacSha1_80,
        remote_rtp_options: Some(srtp_replay_protection(64)),
        remote_rtcp_options: Some(srtcp_replay_protection(64)),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use webrtc_srtp::config::SessionKeys;

    fn config(local_key: u8, remote_key: u8) -> Config {
        Config {
            keys: SessionKeys {
                local_master_key: vec![local_key; 16],
                local_master_salt: vec![local_key; 14],
                remote_master_key: vec![remote_key; 16],
                remote_master_salt: vec![remote_key; 14],
            },
            ..default_config()
        }
    }

    #[tokio::test]
    async fn opposite_contexts_encrypt_and_decrypt_rtp() {
        let server = SrtpContexts::from_config(config(1, 2)).unwrap();
        let client = SrtpContexts::from_config(config(2, 1)).unwrap();
        let packet = [
            0x80, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0xa0, 0x12, 0x34, 0x56, 0x78, 1, 2, 3, 4,
        ];
        let encrypted = server.encrypt_rtp(&packet).await.unwrap();
        assert_ne!(encrypted, packet);
        assert_eq!(client.decrypt_rtp(&encrypted).await.unwrap(), packet);
    }
}
