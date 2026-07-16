//! # SRTP 加解密
//!
//! 本模块实现了 SRTP（Secure Real-time Transport Protocol）的加解密功能。
//!
//! ## 支持的加密套件
//!
//! - AES-128-CM（默认）
//! - AES-128-CM-HMAC-SHA1-80
//!
//! ## 密钥交换
//!
//! SRTP 密钥通过 DTLS-SRTP 交换，从 DTLS 会话中提取。
//!
//! ## 使用场景
//!
//! - 加密出站 RTP 包
//! - 解密入站 SRTP 包

use rtp_core::{SrtpConfig, SrtpContext, SrtpError};

/// SRTP 会话：封装 SRTP 上下文，提供加解密接口。
#[derive(Debug)]
pub struct MediaCryptoSession {
    pub(crate) context: SrtpContext,
}

impl MediaCryptoSession {
    /// 从 SDES 属性创建 SRTP 会话。
    ///
    /// # 参数
    /// - `suite`：加密套件（如 "AES_CM_128_HMAC_SHA1_80"）
    /// - `key_params`：密钥参数
    /// - `ssrc`：RTP SSRC
    pub fn from_sdes(suite: &str, key_params: &str, ssrc: u32) -> Result<Self, SrtpError> {
        let config = SrtpConfig::from_sdes_key_params(suite, key_params)?;
        Ok(Self {
            context: SrtpContext::new(config, ssrc),
        })
    }

    /// 加密 RTP 包。
    pub fn encrypt(&mut self, packet: &mut Vec<u8>) -> Result<usize, SrtpError> {
        self.context.encrypt_rtp(packet)
    }

    /// 在固定容量缓冲区中加密 RTP 包，避免热路径分配 `Vec`。
    pub fn encrypt_in_place(
        &mut self,
        buffer: &mut [u8],
        packet_len: usize,
    ) -> Result<usize, SrtpError> {
        self.context.encrypt_rtp_in_place(buffer, packet_len)
    }

    /// 解密 SRTP 包。
    pub fn decrypt(&mut self, packet: &mut [u8]) -> Result<usize, SrtpError> {
        self.context.decrypt_srtp(packet)
    }
}
