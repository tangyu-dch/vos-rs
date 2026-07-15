//! # rtp-core：RTP/RTCP 协议实现
//!
//! 本 crate 实现了 RTP（Real-time Transport Protocol）和 RTCP（RTP Control Protocol）协议，
//! 用于 VoIP 语音数据的传输和质量监控。
//!
//! ## 核心功能
//!
//! - **RTP 包解析**：零拷贝解析 RTP 头部和载荷
//! - **RTCP 报告**：Sender Report / Receiver Report 解析和生成
//! - **音频编解码**：PCMU（G.711 μ-law）和 PCMA（G.711 A-law）
//! - **SRTP**：SRTP 加解密（AES-128-CM）
//! - **Telephone Event**：RFC 2833 DTMF 事件
//!
//! ## RTP 包格式
//!
//! ```text
//!  0                   1                   2                   3
//!  0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
//! +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//! |V=2|P|X|  CC   |M|     PT      |       sequence number         |
//! +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//! |                           timestamp                           |
//! +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//! |           synchronization source (SSRC) identifier            |
//! +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//! |            contributing source (CSRC) identifiers             |
//! |                             ....                              |
//! +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//! ```

mod buffer_pool;
mod error;
mod packet;
mod payload;
mod rtcp;
mod srtp;
mod telephone_event;

pub use buffer_pool::{PacketBufferPool, RecycledBuffer, ReusablePacket, PACKET_BUFFER_SIZE};
pub use error::{RtpError, RtpResult};
pub use packet::{
    RtpHeaderExtension, RtpHeaderExtensionView, RtpPacket, RtpPacketView, RTP_VERSION,
};
pub use payload::{AudioCodec, StaticAudioPayload};
pub use rtcp::{RtcpPacket, RtcpPacketType, RtcpReceiverReport, RtcpReportBlock, RtcpSenderReport};
pub use srtp::{
    extract_srtp_config_from_dtls, SrtpConfig, SrtpContext, SrtpError, SrtpProfile,
    SrtpSessionManager,
};
pub use telephone_event::TelephoneEvent;
