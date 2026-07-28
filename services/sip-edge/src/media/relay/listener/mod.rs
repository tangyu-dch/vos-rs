//! RTP/RTCP 媒体端口监听与转发主循环。
//!
//! 由 [`spawn_rtp_relay_listeners`] 启动每个端口的独立 task，
//! 调用 [`relay_media_port`] 执行实际的 RTP 包接收、转码与转发。

use super::*;

mod accept;
mod helpers;
mod relay_loop;
mod symmetric;
#[cfg(test)]
mod test_runner;

pub(crate) use relay_loop::relay_media_port;
#[cfg(test)]
pub use test_runner::spawn_rtp_relay_listeners;

// 共享内部类型
pub(super) struct CachedSourceBinding {
    pub(super) address: SocketAddr,
    pub(super) last_seen: std::time::Instant,
}
