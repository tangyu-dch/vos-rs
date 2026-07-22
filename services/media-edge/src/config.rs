use serde::Deserialize;
use std::{fs, net::SocketAddr, path::PathBuf};

const DEFAULT_CONTROL_BIND: &str = "0.0.0.0:3030";
const DEFAULT_UDS_PATH: &str = "/tmp/media-edge.sock";

/// media-edge 进程启动配置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaEdgeServiceConfig {
    pub control_bind: SocketAddr,
    pub control_token: String,
    pub uds_path: String,
    pub recording_workers: usize,
    pub recording_queue_capacity: usize,
}

#[derive(Debug, Default, Deserialize)]
struct RootConfig {
    media_edge: Option<FileConfig>,
}

#[derive(Debug, Default, Deserialize)]
struct FileConfig {
    control_bind: Option<SocketAddr>,
    control_token: Option<String>,
    uds_path: Option<String>,
    recording_workers: Option<usize>,
    recording_queue_capacity: Option<usize>,
}

impl Default for MediaEdgeServiceConfig {
    fn default() -> Self {
        Self {
            control_bind: DEFAULT_CONTROL_BIND
                .parse()
                .unwrap_or_else(|_| SocketAddr::from(([0, 0, 0, 0], 3030))),
            control_token: String::new(),
            uds_path: DEFAULT_UDS_PATH.to_string(),
            recording_workers: 4,
            recording_queue_capacity: 10_000,
        }
    }
}

impl MediaEdgeServiceConfig {
    /// 从 `VOS_RS_CONFIG_FILE` 指定的 YAML 加载；未指定时读取当前目录的 config.yaml。
    pub fn load() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let path = std::env::var("VOS_RS_CONFIG_FILE")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("config.yaml"));
        let content = fs::read_to_string(&path)
            .map_err(|error| format!("读取配置文件 {} 失败: {error}", path.display()))?;
        Self::from_yaml(&content)
    }

    fn from_yaml(content: &str) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let defaults = Self::default();
        let section = serde_yaml::from_str::<RootConfig>(content)?
            .media_edge
            .unwrap_or_default();
        let mut config = Self {
            control_bind: section.control_bind.unwrap_or(defaults.control_bind),
            control_token: std::env::var("VOS_RS_MEDIA_CONTROL_TOKEN")
                .ok()
                .or(section.control_token)
                .unwrap_or(defaults.control_token),
            uds_path: section.uds_path.unwrap_or(defaults.uds_path),
            recording_workers: section
                .recording_workers
                .unwrap_or(defaults.recording_workers)
                .max(1),
            recording_queue_capacity: section
                .recording_queue_capacity
                .unwrap_or(defaults.recording_queue_capacity)
                .max(1),
        };
        config.control_token = config.control_token.trim().to_string();
        if !config.control_bind.ip().is_loopback() && config.control_token.is_empty() {
            return Err("非回环 Media Edge 控制端口必须配置 control_token".into());
        }
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_media_edge_section() {
        let config = MediaEdgeServiceConfig::from_yaml(
            "media_edge:\n  control_bind: 127.0.0.1:3131\n  control_token: test-token\n  uds_path: /tmp/test.sock\n  recording_workers: 8\n  recording_queue_capacity: 2048\n",
        )
        .expect("valid config");
        assert_eq!(config.control_bind, "127.0.0.1:3131".parse().unwrap());
        assert_eq!(config.control_token, "test-token");
        assert_eq!(config.uds_path, "/tmp/test.sock");
        assert_eq!(config.recording_workers, 8);
        assert_eq!(config.recording_queue_capacity, 2048);
    }

    #[test]
    fn rejects_public_control_bind_without_token() {
        let error = MediaEdgeServiceConfig::from_yaml(
            "media_edge:\n  control_bind: 0.0.0.0:3030\n  control_token: ''\n",
        )
        .expect_err("public control endpoint must require authentication");

        assert!(error.to_string().contains("control_token"));
    }
}
