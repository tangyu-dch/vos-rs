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
//! | `VOS_RS_RTP_ADVERTISED_ADDR` | RTP 对外通告地址 | 127.0.0.1 |
//! | `VOS_RS_RTP_PORT_MIN` | RTP 端口范围起始 | 40000 |
//! | `VOS_RS_RTP_PORT_MAX` | RTP 端口范围结束 | 40100 |
//! | `VOS_RS_RTP_SYMMETRIC_LEARNING` | 对称 RTP 学习 | true |
//! | `VOS_RS_RTP_ANTI_SPOOFING` | RTP 反欺骗 | true |
//! | `VOS_RS_RTP_SOURCE_RELEARN_SECS` | RTP 源重新学习间隔 | 30s |
//! | `VOS_RS_RECORDING_ENABLED` | 录音开关 | false |
//! | `VOS_RS_RECORDING_DIR` | 录音目录 | target/recordings |
//! | `VOS_RS_RECORDING_RETENTION_SECS` | 录音保留时间 | 7 天 |
//! | `VOS_RS_RECORDING_MIN_FREE_BYTES` | 最小磁盘空间 | 512MB |
//! | `VOS_RS_RECORDING_MAX_FILE_BYTES` | 最大录音文件 | 128MB |
//! | `VOS_RS_RECORDING_MAX_DURATION_SECS` | 最大录音时长 | 3600s |

use std::env;
use std::path::PathBuf;

pub const RTP_ADVERTISED_ADDR_ENV: &str = "VOS_RS_RTP_ADVERTISED_ADDR";
pub const RTP_PORT_MIN_ENV: &str = "VOS_RS_RTP_PORT_MIN";
pub const RTP_PORT_MAX_ENV: &str = "VOS_RS_RTP_PORT_MAX";
pub const RTP_SYMMETRIC_LEARNING_ENV: &str = "VOS_RS_RTP_SYMMETRIC_LEARNING";
pub const RTP_ANTI_SPOOFING_ENV: &str = "VOS_RS_RTP_ANTI_SPOOFING";
pub const RTP_SOURCE_RELEARN_SECS_ENV: &str = "VOS_RS_RTP_SOURCE_RELEARN_SECS";
pub const RECORDING_ENABLED_ENV: &str = "VOS_RS_RECORDING_ENABLED";
pub const RECORDING_DIR_ENV: &str = "VOS_RS_RECORDING_DIR";
pub const RECORDING_RETENTION_SECS_ENV: &str = "VOS_RS_RECORDING_RETENTION_SECS";
pub const RECORDING_MIN_FREE_BYTES_ENV: &str = "VOS_RS_RECORDING_MIN_FREE_BYTES";
pub const RECORDING_MAX_FILE_BYTES_ENV: &str = "VOS_RS_RECORDING_MAX_FILE_BYTES";
pub const RECORDING_MAX_DURATION_SECS_ENV: &str = "VOS_RS_RECORDING_MAX_DURATION_SECS";
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

#[derive(Debug, Clone, PartialEq, Eq)]
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
        }
    }

    pub fn from_env() -> Self {
        let advertised_addr = env::var(RTP_ADVERTISED_ADDR_ENV)
            .unwrap_or_else(|_| DEFAULT_RTP_ADVERTISED_ADDR.to_string());
        let port_min = env_port(RTP_PORT_MIN_ENV).unwrap_or(DEFAULT_RTP_PORT_MIN);
        let port_max = env_port(RTP_PORT_MAX_ENV).unwrap_or(DEFAULT_RTP_PORT_MAX);
        let symmetric_rtp_learning =
            env_bool(RTP_SYMMETRIC_LEARNING_ENV).unwrap_or(DEFAULT_RTP_SYMMETRIC_LEARNING);
        let anti_spoofing = env_bool(RTP_ANTI_SPOOFING_ENV).unwrap_or(DEFAULT_RTP_ANTI_SPOOFING);
        let source_relearn_after_secs = env::var(RTP_SOURCE_RELEARN_SECS_ENV)
            .ok()
            .and_then(|value| value.trim().parse::<u64>().ok())
            .unwrap_or(DEFAULT_RTP_SOURCE_RELEARN_SECS);
        let recording_enabled =
            env_bool(RECORDING_ENABLED_ENV).unwrap_or(DEFAULT_RECORDING_ENABLED);
        let recording_dir =
            env::var(RECORDING_DIR_ENV).unwrap_or_else(|_| DEFAULT_RECORDING_DIR.to_string());
        let recording_retention_secs =
            env_u64(RECORDING_RETENTION_SECS_ENV).unwrap_or(DEFAULT_RECORDING_RETENTION_SECS);
        let recording_min_free_bytes =
            env_u64(RECORDING_MIN_FREE_BYTES_ENV).unwrap_or(DEFAULT_RECORDING_MIN_FREE_BYTES);
        let recording_max_file_bytes =
            env_u64(RECORDING_MAX_FILE_BYTES_ENV).unwrap_or(DEFAULT_RECORDING_MAX_FILE_BYTES);
        let recording_max_duration_secs =
            env_u64(RECORDING_MAX_DURATION_SECS_ENV).unwrap_or(DEFAULT_RECORDING_MAX_DURATION_SECS);
        let mut config = Self::new_with_symmetric_learning(
            advertised_addr,
            port_min,
            port_max,
            symmetric_rtp_learning,
        );
        config.recording_enabled = recording_enabled;
        config.anti_spoofing = anti_spoofing;
        config.source_relearn_after_secs = source_relearn_after_secs;
        config.recording_dir = PathBuf::from(recording_dir);
        config.recording_retention_secs = recording_retention_secs;
        config.recording_min_free_bytes = recording_min_free_bytes;
        config.recording_max_file_bytes = recording_max_file_bytes;
        config.recording_max_duration_secs = recording_max_duration_secs;
        config
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

// Helper environment functions
fn env_port(name: &str) -> Option<u16> {
    env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<u16>().ok())
}

fn env_bool(name: &str) -> Option<bool> {
    env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<bool>().ok())
}

fn env_u64(name: &str) -> Option<u64> {
    env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
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

pub(crate) const RECORDING_WORKERS_ENV: &str = "VOS_RS_RECORDING_WORKERS";
pub(crate) const RECORDING_QUEUE_CAPACITY_ENV: &str = "VOS_RS_RECORDING_QUEUE_CAPACITY";
pub(crate) const DEFAULT_RECORDING_WORKERS: usize = 4;
pub(crate) const DEFAULT_RECORDING_QUEUE_CAPACITY: usize = 10000;

pub(crate) fn recording_worker_count() -> usize {
    env::var(RECORDING_WORKERS_ENV)
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|workers| *workers > 0)
        .unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(|parallelism| parallelism.get().clamp(1, DEFAULT_RECORDING_WORKERS))
                .unwrap_or(DEFAULT_RECORDING_WORKERS)
        })
}

pub(crate) fn recording_queue_capacity() -> usize {
    env::var(RECORDING_QUEUE_CAPACITY_ENV)
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|capacity| *capacity > 0)
        .unwrap_or(DEFAULT_RECORDING_QUEUE_CAPACITY)
}
