//! # 媒体处理层
//!
//! 本模块实现了 VoIP 媒体处理的核心功能，包括：
//!
//! - **RTP 中继**：双向 RTP 包转发，支持 Symmetric RTP（NAT 穿透）
//! - **RTCP 质量监控**：收集 RTCP 报告，计算 MOS 分数
//! - **录音**：双声道 WAV 录音（Caller + Gateway）
//! - **DTMF**：RFC 2833 检测 + SIP INFO 支持
//! - **SRTP**：SRTP 加解密支持
//! - **SDP 重写**：SIP 代理的 SDP 地址改写
//! - **端口分配**：RTP/RTCP 端口池管理
//!
//! ## 架构
//!
//! ```text
//! SIP 信令层 → SDP 协商 → 分配 RTP 端口 → 建立双向中继
//!                              ↓
//!                         录音 worker pool（异步写入 WAV）
//! ```
//!
//! ## 关键设计
//!
//! - 端口分配使用 Mutex 保护（高并发下可优化为 lock-free 池）
//! - 录音使用独立线程池，避免阻塞 tokio runtime
//! - Symmetric RTP 自动学习对端地址，支持 NAT 穿透

#[allow(dead_code)]
pub(crate) mod conference;
#[allow(dead_code)]
pub(crate) mod config;
pub(crate) mod crypto;
#[allow(dead_code)]
pub(crate) mod dtmf;
pub(crate) mod live_transcode;
pub(crate) mod metrics;
pub(crate) mod recording;
pub(crate) mod relay;
pub(crate) mod rtcp_processor;
#[allow(dead_code)]
pub(crate) mod sdp;
pub(crate) mod transcode;
pub(crate) mod utils;
#[allow(dead_code)]
pub(crate) mod wav;

pub use self::config::MediaConfig;
pub use self::live_transcode::LiveTranscoder;
pub use self::relay::MediaRelayState;
