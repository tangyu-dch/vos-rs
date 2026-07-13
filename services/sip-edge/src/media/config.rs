//! # 媒体配置
//!
//! 本模块定义了媒体处理的配置参数，包括：
//!
//! - **RTP 配置**：地址、端口范围、对称学习、反欺骗
//! - **录音配置**：启用状态、目录、保留时间、磁盘保护
//!
//! ## 配置项
//!
//! | 环境变量 | 说明 | 默认值 |
//! |---------|------|--------|
//! 媒体与录音参数统一由 `config.yaml` 的 `sip_edge.media` 和
//! `sip_edge.recording` 提供，并可在启动阶段由 Redis 动态配置覆盖。

use std::path::PathBuf;

pub const DEFAULT_RTP_ADVERTISED_ADDR: &str = "127.0.0.1";
pub const DEFAULT_RTP_PORT_MIN: u16 = 40_000;
pub const DEFAULT_RTP_PORT_MAX: u16 = 40_100;
pub const DEFAULT_RTP_SYMMETRIC_LEARNING: bool = true;
pub const DEFAULT_RTP_ANTI_SPOOFING: bool = true;
pub const DEFAULT_RTP_SOURCE_RELEARN_SECS: u64 = 30;
pub const DEFAULT_RECORDING_ENABLED: bool = false;
pub const DEFAULT_RECORDING_DIR: &str = "target/recordings";
pub const DEFAULT_RECORDING_RETENTION_SECS: u64 = 7 * 24 * 60 * 60;
pub const DEFAULT_RECORDING_MIN_FREE_BYTES: u64 = 512 * 1024 * 1024;
pub const DEFAULT_RECORDING_MAX_FILE_BYTES: u64 = 128 * 1024 * 1024;
pub const DEFAULT_RECORDING_MAX_DURATION_SECS: u64 = 60 * 60;
/// 录音后处理转码格式。支持 `wav`（无转码）、`opus`（需 ffmpeg）、`amr`（需 ffmpeg）。
pub const DEFAULT_RECORDING_FORMAT: &str = "wav";

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MediaConfig {
    pub advertised_addr: String,
    pub port_min: u16,
    pub port_max: u16,
    pub symmetric_rtp_learning: bool,
    pub anti_spoofing: bool,
    pub source_relearn_after_secs: u64,
    pub recording_enabled: bool,
    pub recording_dir: PathBuf,
    pub recording_retention_secs: u64,
    pub recording_min_free_bytes: u64,
    pub recording_max_file_bytes: u64,
    pub recording_max_duration_secs: u64,
    /// 录音完成后的转码格式：`wav`（原始，无转码）/ `opus` / `amr`
    /// 非 wav 格式需要系统安装 ffmpeg。
    pub recording_format: String,
}

impl MediaConfig {
    #[cfg(test)]
    pub fn new(advertised_addr: impl Into<String>, port_min: u16, port_max: u16) -> Self {
        Self::new_with_symmetric_learning(
            advertised_addr,
            port_min,
            port_max,
            DEFAULT_RTP_SYMMETRIC_LEARNING,
        )
    }

    pub fn new_with_symmetric_learning(
        advertised_addr: impl Into<String>,
        port_min: u16,
        port_max: u16,
        symmetric_rtp_learning: bool,
    ) -> Self {
        let mut port_min = even_port_at_or_above(port_min).unwrap_or(DEFAULT_RTP_PORT_MIN);
        let mut port_max = even_port_at_or_below(port_max).unwrap_or(DEFAULT_RTP_PORT_MAX);

        if port_min > port_max {
            port_min = DEFAULT_RTP_PORT_MIN;
            port_max = DEFAULT_RTP_PORT_MAX;
        }

        Self {
            advertised_addr: advertised_addr.into(),
            port_min,
            port_max,
            symmetric_rtp_learning,
            anti_spoofing: DEFAULT_RTP_ANTI_SPOOFING,
            source_relearn_after_secs: DEFAULT_RTP_SOURCE_RELEARN_SECS,
            recording_enabled: DEFAULT_RECORDING_ENABLED,
            recording_dir: PathBuf::from(DEFAULT_RECORDING_DIR),
            recording_retention_secs: DEFAULT_RECORDING_RETENTION_SECS,
            recording_min_free_bytes: DEFAULT_RECORDING_MIN_FREE_BYTES,
            recording_max_file_bytes: DEFAULT_RECORDING_MAX_FILE_BYTES,
            recording_max_duration_secs: DEFAULT_RECORDING_MAX_DURATION_SECS,
            recording_format: DEFAULT_RECORDING_FORMAT.to_string(),
        }
    }

    #[cfg(test)]
    pub fn with_recording(mut self, enabled: bool, dir: impl Into<PathBuf>) -> Self {
        self.recording_enabled = enabled;
        self.recording_dir = dir.into();
        self.recording_retention_secs = 0;
        self.recording_min_free_bytes = 0;
        self.recording_max_file_bytes = 0;
        self.recording_max_duration_secs = 0;
        self
    }

    pub fn set_advertised_addr(&mut self, addr: impl Into<String>) {
        self.advertised_addr = addr.into();
    }
}

fn even_port_at_or_above(port: u16) -> Option<u16> {
    if port % 2 == 0 {
        Some(port)
    } else {
        port.checked_add(1)
    }
}

fn even_port_at_or_below(port: u16) -> Option<u16> {
    if port % 2 == 0 {
        Some(port)
    } else {
        port.checked_sub(1)
    }
}

pub(crate) const DEFAULT_RECORDING_WORKERS: usize = 4;
pub(crate) const DEFAULT_RECORDING_QUEUE_CAPACITY: usize = 10000;
