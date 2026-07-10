//! # sdp-core：SDP 协议解析
//!
//! 本 crate 实现了 SDP（Session Description Protocol）协议的解析和生成，
//! 用于 VoIP 媒体协商。
//!
//! ## 核心功能
//!
//! - **SDP 解析**：解析 SDP 会话描述，提取 RTP 端点信息
//! - **SDP 重写**：修改 SDP 中的地址和端口（SIP 代理使用）
//! - **ICE/DTLS**：ICE 候选和 DTLS-SRTP 参数提取
//! - **音频格式**：音频编解码器格式解析
//!
//! ## SDP 结构
//!
//! ```text
//! v=0                          ← 版本
//! o=- ...                      ← 会话标识
//! s=...                        ← 会话名称
//! c=IN IP4 1.2.3.4            ← 连接信息
//! t=0 0                        ← 时间
//! m=audio 40000 RTP/AVP 0 8   ← 媒体描述
//! a=rtpmap:0 PCMU/8000        ← 编解码器映射
//! a=rtpmap:8 PCMA/8000
//! ```

mod error;
mod session;

pub use error::{SdpError, SdpResult};
pub use session::{
    AudioFormat, DtlsFingerprint, DtlsParameters, IceCandidate, IceParameters, MediaDescription,
    RtpEndpoint, SessionDescription, SrtpCryptoAttribute,
};
