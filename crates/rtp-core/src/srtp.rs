use aes::cipher::{KeyIvInit, StreamCipher};
use aes::Aes128;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use ctr::Ctr128BE;
use hmac::{Hmac, Mac};
use sha1::Sha1;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

type Aes128Ctr = Ctr128BE<Aes128>;
type HmacSha1 = Hmac<Sha1>;

/// SRTP 保护配置
#[derive(Clone)]
pub struct SrtpConfig {
    /// SRTP 密钥 (16 bytes for AES-128)
    pub master_key: [u8; 16],
    /// SRTP 盐值 (14 bytes)
    pub master_salt: [u8; 14],
    /// 保护配置文件
    pub profile: SrtpProfile,
}

impl std::fmt::Debug for SrtpConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SrtpConfig")
            .field("profile", &self.profile)
            .finish()
    }
}

impl SrtpConfig {
    /// Builds an SRTP configuration from an SDES `inline:` key parameter.
    ///
    /// The lifetime and rollover fields after the first `|` are accepted and
    /// retained by the SDP layer, but this context starts at packet index 0.
    pub fn from_sdes_key_params(suite: &str, key_params: &str) -> Result<Self, SrtpError> {
        let profile = match suite.trim().to_ascii_uppercase().as_str() {
            "AES_CM_128_HMAC_SHA1_80" => SrtpProfile::Aes128CmHmacSha1_80,
            "AES_CM_128_HMAC_SHA1_32" => SrtpProfile::Aes128CmHmacSha1_32,
            "NULL_HMAC_SHA1_80" => SrtpProfile::NullHmacSha1_80,
            _ => return Err(SrtpError::UnsupportedProfile),
        };

        let encoded = key_params
            .trim()
            .strip_prefix("inline:")
            .ok_or(SrtpError::InvalidKey)?
            .split('|')
            .next()
            .filter(|value| !value.is_empty())
            .ok_or(SrtpError::InvalidKey)?;
        let decoded = STANDARD
            .decode(encoded)
            .map_err(|_| SrtpError::InvalidKey)?;
        let required_len = profile.key_length() + profile.salt_length();
        if decoded.len() < required_len {
            return Err(SrtpError::InvalidKey);
        }

        let mut master_key = [0u8; 16];
        let mut master_salt = [0u8; 14];
        if profile.key_length() > 0 {
            master_key.copy_from_slice(&decoded[..16]);
            master_salt.copy_from_slice(&decoded[16..30]);
        }

        Ok(Self {
            master_key,
            master_salt,
            profile,
        })
    }
}

/// SRTP 保护配置文件
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SrtpProfile {
    /// AES_128_CM_HMAC_SHA1_80 (最常用)
    Aes128CmHmacSha1_80,
    /// AES_128_CM_HMAC_SHA1_32
    Aes128CmHmacSha1_32,
    /// NULL_HMAC_SHA1_80 (不加密，仅认证)
    NullHmacSha1_80,
}

impl SrtpProfile {
    pub fn key_length(&self) -> usize {
        match self {
            Self::Aes128CmHmacSha1_80 | Self::Aes128CmHmacSha1_32 => 16,
            Self::NullHmacSha1_80 => 0,
        }
    }

    pub fn salt_length(&self) -> usize {
        match self {
            Self::Aes128CmHmacSha1_80 | Self::Aes128CmHmacSha1_32 => 14,
            Self::NullHmacSha1_80 => 0,
        }
    }

    pub fn auth_tag_length(&self) -> usize {
        match self {
            Self::Aes128CmHmacSha1_80 | Self::NullHmacSha1_80 => 10,
            Self::Aes128CmHmacSha1_32 => 4,
        }
    }
}

/// SRTP 上下文（单个 SSRC 的加密状态）
pub struct SrtpContext {
    config: SrtpConfig,
    ssrc: u32,
    packet_index: u64,
}

impl SrtpContext {
    pub fn new(config: SrtpConfig, ssrc: u32) -> Self {
        Self {
            config,
            ssrc,
            packet_index: 0,
        }
    }

    fn create_cipher(&self, index: u64) -> Aes128Ctr {
        // 从主盐值派生会话盐值
        let mut session_salt = [0u8; 14];
        session_salt.copy_from_slice(&self.config.master_salt);

        // XOR 会话盐值与 SSRC 和索引
        let ssrc_bytes = self.ssrc.to_be_bytes();
        for i in 0..4 {
            session_salt[4 + i] ^= ssrc_bytes[i];
        }
        let index_bytes = index.to_be_bytes();
        for i in 0..6 {
            session_salt[8 + i] ^= index_bytes[i];
        }

        // 创建 AES-128-CTR 密码
        let mut iv = [0u8; 16];
        iv[..14].copy_from_slice(&session_salt);
        Aes128Ctr::new(self.config.master_key[..16].into(), &iv.into())
    }

    /// 加密 RTP 数据包
    pub fn encrypt_rtp(&mut self, packet: &mut Vec<u8>) -> Result<usize, SrtpError> {
        if packet.len() < 12 {
            return Err(SrtpError::PacketTooShort);
        }

        // 解析 RTP 头长度
        let header_len = Self::rtp_header_length(packet);
        if packet.len() < header_len {
            return Err(SrtpError::PacketTooShort);
        }

        match self.config.profile {
            SrtpProfile::Aes128CmHmacSha1_80 | SrtpProfile::Aes128CmHmacSha1_32 => {
                // 就地加密负载（零分配，直接在原 Vec 上做 AES-CTR keystream XOR）
                let mut cipher = self.create_cipher(self.packet_index);
                cipher.apply_keystream(&mut packet[header_len..]);

                // 计算 HMAC-SHA1 认证标签
                let auth_tag = self.compute_auth_tag(packet)?;
                let tag_len = self.config.profile.auth_tag_length();
                packet.extend_from_slice(&auth_tag[..tag_len]);

                self.packet_index += 1;
                Ok(packet.len())
            }
            SrtpProfile::NullHmacSha1_80 => {
                // 不加密，仅添加认证标签
                let auth_tag = self.compute_auth_tag(packet)?;
                let tag_len = self.config.profile.auth_tag_length();
                packet.extend_from_slice(&auth_tag[..tag_len]);
                self.packet_index += 1;
                Ok(packet.len())
            }
        }
    }

    /// 解密 SRTP 数据包
    pub fn decrypt_srtp(&mut self, packet: &mut [u8]) -> Result<usize, SrtpError> {
        let tag_len = self.config.profile.auth_tag_length();
        if packet.len() < 12 + tag_len {
            return Err(SrtpError::PacketTooShort);
        }

        // 验证认证标签
        let received_tag = &packet[packet.len() - tag_len..];
        let payload = &packet[..packet.len() - tag_len];
        let computed_tag = self.compute_auth_tag(payload)?;
        if received_tag != &computed_tag[..tag_len] {
            return Err(SrtpError::AuthenticationFailed);
        }

        // 移除认证标签
        let packet_len = packet.len() - tag_len;

        match self.config.profile {
            SrtpProfile::Aes128CmHmacSha1_80 | SrtpProfile::Aes128CmHmacSha1_32 => {
                // 就地解密负载（零分配，CTR 模式解密 = 直接 XOR keystream）
                let header_len = Self::rtp_header_length(&packet[..packet_len]);
                let mut cipher = self.create_cipher(self.packet_index);
                cipher.apply_keystream(&mut packet[header_len..packet_len]);
            }
            SrtpProfile::NullHmacSha1_80 => {
                // 不解密
            }
        }

        self.packet_index += 1;
        Ok(packet_len)
    }

    fn rtp_header_length(packet: &[u8]) -> usize {
        let cc = (packet[0] & 0x0F) as usize;
        let extension = (packet[0] >> 4) & 1;
        let mut len = 12 + cc * 4;
        if extension == 1 && packet.len() >= len + 4 {
            let ext_len = u16::from_be_bytes([packet[len + 2], packet[len + 3]]) as usize;
            len += 4 + ext_len * 4;
        }
        len
    }

    fn compute_auth_tag(&self, packet: &[u8]) -> Result<[u8; 20], SrtpError> {
        let mut mac =
            HmacSha1::new_from_slice(&self.config.master_key).map_err(|_| SrtpError::InvalidKey)?;
        mac.update(packet);
        let result = mac.finalize().into_bytes();
        let mut tag = [0u8; 20];
        tag.copy_from_slice(&result);
        Ok(tag)
    }
}

impl std::fmt::Debug for SrtpContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SrtpContext")
            .field("ssrc", &self.ssrc)
            .field("packet_index", &self.packet_index)
            .finish()
    }
}

/// SRTP 错误类型
#[derive(Debug, Clone)]
pub enum SrtpError {
    PacketTooShort,
    AuthenticationFailed,
    InvalidKey,
    UnsupportedProfile,
}

impl std::fmt::Display for SrtpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PacketTooShort => write!(f, "SRTP packet too short"),
            Self::AuthenticationFailed => write!(f, "SRTP authentication failed"),
            Self::InvalidKey => write!(f, "SRTP invalid key"),
            Self::UnsupportedProfile => write!(f, "SRTP unsupported profile"),
        }
    }
}

impl std::error::Error for SrtpError {}

/// SRTP 会话管理器
#[derive(Debug, Clone)]
pub struct SrtpSessionManager {
    contexts: Arc<RwLock<HashMap<u32, Arc<tokio::sync::Mutex<SrtpContext>>>>>,
}

impl SrtpSessionManager {
    pub fn new() -> Self {
        Self {
            contexts: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// 添加新的 SRTP 上下文
    pub async fn add_context(&self, ssrc: u32, config: SrtpConfig) {
        let ctx = SrtpContext::new(config, ssrc);
        self.contexts
            .write()
            .await
            .insert(ssrc, Arc::new(tokio::sync::Mutex::new(ctx)));
    }

    /// 获取 SRTP 上下文
    pub async fn get_context(&self, ssrc: u32) -> Option<Arc<tokio::sync::Mutex<SrtpContext>>> {
        self.contexts.read().await.get(&ssrc).cloned()
    }

    /// 移除 SRTP 上下文
    pub async fn remove_context(&self, ssrc: u32) -> bool {
        self.contexts.write().await.remove(&ssrc).is_some()
    }
}

impl Default for SrtpSessionManager {
    fn default() -> Self {
        Self::new()
    }
}

/// 从 DTLS-SRTP 提取 SRTP 配置
pub fn extract_srtp_config_from_dtls(
    dtls_key: &[u8],
    profile: SrtpProfile,
) -> Result<SrtpConfig, SrtpError> {
    if dtls_key.len() < 16 {
        return Err(SrtpError::InvalidKey);
    }

    let mut master_key = [0u8; 16];
    master_key.copy_from_slice(&dtls_key[..16]);

    // 从 DTLS 密钥派生盐值（简化实现）
    let mut master_salt = [0u8; 14];
    if dtls_key.len() >= 30 {
        master_salt.copy_from_slice(&dtls_key[16..30]);
    }

    Ok(SrtpConfig {
        master_key,
        master_salt,
        profile,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_srtp_profile_properties() {
        assert_eq!(SrtpProfile::Aes128CmHmacSha1_80.key_length(), 16);
        assert_eq!(SrtpProfile::Aes128CmHmacSha1_80.salt_length(), 14);
        assert_eq!(SrtpProfile::Aes128CmHmacSha1_80.auth_tag_length(), 10);
        assert_eq!(SrtpProfile::NullHmacSha1_80.key_length(), 0);
    }

    #[test]
    fn test_srtp_context_creation() {
        let config = SrtpConfig {
            master_key: [0u8; 16],
            master_salt: [0u8; 14],
            profile: SrtpProfile::Aes128CmHmacSha1_80,
        };
        let ctx = SrtpContext::new(config, 12345);
        assert_eq!(ctx.ssrc, 12345);
        assert_eq!(ctx.packet_index, 0);
    }

    #[test]
    fn test_srtp_encrypt_decrypt_roundtrip() {
        let config = SrtpConfig {
            master_key: [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16],
            master_salt: [0u8; 14],
            profile: SrtpProfile::Aes128CmHmacSha1_80,
        };

        let mut ctx = SrtpContext::new(config.clone(), 12345);

        // 创建一个简单的 RTP 包
        let mut packet = vec![
            0x80, 0x00, 0x30, 0x39, // V=2, PT=0, sequence=12345
            0x00, 0x00, 0x00, 0x00, // timestamp
            0x00, 0x00, 0x30, 0x39, // SSRC=12345
            0x01, 0x02, 0x03, 0x04, // payload
        ];
        let original_payload = packet[12..].to_vec();

        // 加密
        let encrypted_len = ctx.encrypt_rtp(&mut packet).unwrap();
        assert!(encrypted_len > 12);

        // 验证负载已加密（不是原始值）
        assert_ne!(packet[12], original_payload[0]);

        // 解密
        let mut decrypt_ctx = SrtpContext::new(config, 12345);
        let decrypted_len = decrypt_ctx.decrypt_srtp(&mut packet).unwrap();
        assert_eq!(decrypted_len, 12 + original_payload.len());
    }

    #[test]
    fn test_srtp_authentication_failure() {
        let config = SrtpConfig {
            master_key: [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16],
            master_salt: [0u8; 14],
            profile: SrtpProfile::Aes128CmHmacSha1_80,
        };

        let mut ctx = SrtpContext::new(config, 12345);

        let mut packet = vec![
            0x80, 0x00, 0x30, 0x39, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x30, 0x39, 0x01, 0x02,
            0x03, 0x04,
        ];

        ctx.encrypt_rtp(&mut packet).unwrap();

        // 篡改认证标签
        let last = packet.len() - 1;
        packet[last] ^= 0xFF;

        // 解密应该失败
        let result = ctx.decrypt_srtp(&mut packet);
        assert!(matches!(result, Err(SrtpError::AuthenticationFailed)));
    }

    #[tokio::test]
    async fn test_srtp_session_manager() {
        let mgr = SrtpSessionManager::new();
        let config = SrtpConfig {
            master_key: [0u8; 16],
            master_salt: [0u8; 14],
            profile: SrtpProfile::Aes128CmHmacSha1_80,
        };

        mgr.add_context(12345, config).await;
        assert!(mgr.get_context(12345).await.is_some());
        assert!(mgr.get_context(99999).await.is_none());

        mgr.remove_context(12345).await;
        assert!(mgr.get_context(12345).await.is_none());
    }

    #[test]
    fn test_extract_srtp_config_from_dtls() {
        let dtls_key = vec![0u8; 30];
        let config = extract_srtp_config_from_dtls(&dtls_key, SrtpProfile::Aes128CmHmacSha1_80);
        assert!(config.is_ok());
    }

    #[test]
    fn test_sdes_key_params_create_config() {
        let encoded = base64::engine::general_purpose::STANDARD.encode([7u8; 30]);
        let config = SrtpConfig::from_sdes_key_params(
            "AES_CM_128_HMAC_SHA1_80",
            &format!("inline:{encoded}|2^31|1:32"),
        )
        .unwrap();

        assert_eq!(config.master_key, [7u8; 16]);
        assert_eq!(config.master_salt, [7u8; 14]);
        assert_eq!(config.profile, SrtpProfile::Aes128CmHmacSha1_80);
    }

    #[test]
    fn test_sdes_key_params_reject_invalid_input() {
        assert!(matches!(
            SrtpConfig::from_sdes_key_params("AES_CM_128_HMAC_SHA1_80", "inline:not-base64"),
            Err(SrtpError::InvalidKey)
        ));
        assert!(matches!(
            SrtpConfig::from_sdes_key_params("UNKNOWN", "inline:dGVzdA=="),
            Err(SrtpError::UnsupportedProfile)
        ));
    }
}
