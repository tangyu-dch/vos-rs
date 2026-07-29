//! 可安全热更新的运行时配置快照。

use std::sync::Arc;

use serde::Serialize;

use crate::media::MediaConfig;

use super::EdgeState;

/// 只对新录音会话生效的配置；已有录音继续使用创建时的不可变快照。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct RecordingRuntimeConfig {
    pub(crate) enabled: bool,
    pub(crate) min_free_bytes: u64,
    pub(crate) max_file_bytes: u64,
    pub(crate) max_duration_secs: u64,
}

impl From<&MediaConfig> for RecordingRuntimeConfig {
    fn from(config: &MediaConfig) -> Self {
        Self {
            enabled: config.recording_enabled,
            min_free_bytes: config.recording_min_free_bytes,
            max_file_bytes: config.recording_max_file_bytes,
            max_duration_secs: config.recording_max_duration_secs,
        }
    }
}

impl RecordingRuntimeConfig {
    fn apply_to(&self, base: &MediaConfig) -> MediaConfig {
        let mut config = base.clone();
        config.recording_enabled = self.enabled;
        config.recording_min_free_bytes = self.min_free_bytes;
        config.recording_max_file_bytes = self.max_file_bytes;
        config.recording_max_duration_secs = self.max_duration_secs;
        config
    }
}

impl EdgeState {
    /// 获取单次录音启动使用的一致配置快照。
    pub(crate) fn recording_media_config(&self, base: &MediaConfig) -> MediaConfig {
        self.recording_runtime_config.load().apply_to(base)
    }

    /// 获取当前录音运行时配置。
    pub(crate) fn recording_runtime_config(&self) -> Arc<RecordingRuntimeConfig> {
        self.recording_runtime_config.load_full()
    }

    /// 原子替换录音运行时配置，新通话会立即读取新快照。
    pub(crate) fn replace_recording_runtime_config(&self, config: RecordingRuntimeConfig) {
        self.recording_runtime_config.store(Arc::new(config));
    }
}

#[cfg(test)]
mod tests {
    use super::RecordingRuntimeConfig;
    use crate::media::MediaConfig;

    #[test]
    fn applies_only_hot_recording_fields() {
        let base = MediaConfig::new("127.0.0.1", 40_000, 40_100);
        let runtime = RecordingRuntimeConfig {
            enabled: true,
            min_free_bytes: 10,
            max_file_bytes: 20,
            max_duration_secs: 30,
        };

        let applied = runtime.apply_to(&base);

        assert!(applied.recording_enabled);
        assert_eq!(applied.recording_min_free_bytes, 10);
        assert_eq!(applied.recording_max_file_bytes, 20);
        assert_eq!(applied.recording_max_duration_secs, 30);
        assert_eq!(applied.recording_dir, base.recording_dir);
    }
}
