//! # storage-core：统一存储抽象
//!
//! 本 crate 提供了统一的文件存储接口，支持多种后端：
//!
//! - **本地文件系统**：开发和小规模部署
//! - **阿里云 OSS**：生产环境云存储
//! - **双写模式**：本地 + OSS 同时写入，保证数据安全
//!
//! ## 配置
//!
//! 通过 `StorageConfig` 配置存储后端：
//! - `kind`：后端类型（local/oss/dual）
//! - `base_dir`：本地存储目录
//! - `oss_endpoint`：OSS 端点
//! - `oss_bucket`：OSS 桶名
//! - `oss_access_key`：OSS 访问密钥
//!
//! ## 使用场景
//!
//! - 录音文件存储
//! - CDR 导出文件
//! - 系统配置文件

use async_trait::async_trait;
use bytes::Bytes;
use thiserror::Error;

pub mod config;
pub mod hw_accel;
pub mod local;
pub mod oss;

pub use config::{StorageBackendKind, StorageConfig, StorageRule};
pub use hw_accel::{HardwareAudioEncoder, HwAccelError, SoftwareFallbackEncoder};

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("序列化错误: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("存储后端不可用: {0}")]
    BackendUnavailable(String),

    #[error("文件不存在: {0}")]
    NotFound(String),

    #[error("认证失败: {0}")]
    AuthError(String),

    #[error("配置错误: {0}")]
    ConfigError(String),
}

#[derive(Debug, Clone)]
pub struct FileInfo {
    pub key: String,
    pub size: u64,
    pub content_type: Option<String>,
    pub last_modified: Option<i64>,
}

/// 统一存储接口，支持本地文件系统和 OSS 兼容后端。
#[async_trait]
pub trait StorageBackend: Send + Sync {
    /// 上传字节数据到指定 key。
    async fn put(
        &self,
        key: &str,
        data: Bytes,
        content_type: Option<&str>,
    ) -> Result<(), StorageError>;

    /// 下载指定 key 的内容。
    async fn get(&self, key: &str) -> Result<Bytes, StorageError>;

    /// 列出指定前缀下的所有文件。
    async fn list(&self, prefix: &str) -> Result<Vec<FileInfo>, StorageError>;

    /// 检查文件是否存在。
    async fn exists(&self, key: &str) -> Result<bool, StorageError>;

    /// 删除指定 key 的文件。
    async fn delete(&self, key: &str) -> Result<(), StorageError>;

    /// 获取文件的预签名 URL（用于直接下载/播放）。
    async fn presign_get(&self, key: &str, expires_secs: u64) -> Result<String, StorageError>;

    /// 返回后端类型名称。
    fn backend_name(&self) -> &str;
}

/// 根据存储配置创建对应的存储后端。
pub async fn create_storage(
    config: &StorageConfig,
) -> Result<Box<dyn StorageBackend>, StorageError> {
    match &config.backend {
        StorageBackendKind::Local => {
            let base = config.local_dir.as_str();
            Ok(Box::new(local::LocalStorage::new(base)?))
        }
        StorageBackendKind::Oss => {
            let endpoint = config
                .oss_endpoint
                .as_ref()
                .ok_or_else(|| StorageError::ConfigError("oss_endpoint 未配置".into()))?;
            let bucket = config
                .oss_bucket
                .as_ref()
                .ok_or_else(|| StorageError::ConfigError("oss_bucket 未配置".into()))?;
            let access_key = config
                .oss_access_key
                .as_ref()
                .ok_or_else(|| StorageError::ConfigError("oss_access_key 未配置".into()))?;
            let secret_key = config
                .oss_secret_key
                .as_ref()
                .ok_or_else(|| StorageError::ConfigError("oss_secret_key 未配置".into()))?;
            let prefix = config.oss_key_prefix.clone().unwrap_or_default();
            let region = config
                .oss_region
                .clone()
                .unwrap_or_else(|| "us-east-1".to_string());

            Ok(Box::new(oss::OssStorage::new(
                endpoint, bucket, access_key, secret_key, prefix, region,
            )?))
        }
        StorageBackendKind::Dual => {
            let primary: Box<dyn StorageBackend> = {
                let endpoint = config
                    .oss_endpoint
                    .as_ref()
                    .ok_or_else(|| StorageError::ConfigError("oss_endpoint 未配置".into()))?;
                let bucket = config
                    .oss_bucket
                    .as_ref()
                    .ok_or_else(|| StorageError::ConfigError("oss_bucket 未配置".into()))?;
                let access_key = config
                    .oss_access_key
                    .as_ref()
                    .ok_or_else(|| StorageError::ConfigError("oss_access_key 未配置".into()))?;
                let secret_key = config
                    .oss_secret_key
                    .as_ref()
                    .ok_or_else(|| StorageError::ConfigError("oss_secret_key 未配置".into()))?;
                let prefix = config.oss_key_prefix.clone().unwrap_or_default();
                let region = config
                    .oss_region
                    .clone()
                    .unwrap_or_else(|| "us-east-1".to_string());
                Box::new(oss::OssStorage::new(
                    endpoint, bucket, access_key, secret_key, prefix, region,
                )?)
            };
            let fallback: Box<dyn StorageBackend> =
                Box::new(local::LocalStorage::new(&config.local_dir)?);
            Ok(Box::new(DualStorage {
                primary,
                fallback,
                rules: config.rules.clone(),
            }))
        }
    }
}

/// 双写存储：先写 OSS，失败时回退到本地存储。
/// 同时支持从两个后端读取。
pub struct DualStorage {
    primary: Box<dyn StorageBackend>,
    fallback: Box<dyn StorageBackend>,
    rules: Vec<StorageRule>,
}

impl DualStorage {
    fn should_use_primary(&self, key: &str) -> bool {
        for rule in &self.rules {
            if rule.matches(key) {
                return rule.primary_only;
            }
        }
        true
    }
}

#[async_trait]
impl StorageBackend for DualStorage {
    async fn put(
        &self,
        key: &str,
        data: Bytes,
        content_type: Option<&str>,
    ) -> Result<(), StorageError> {
        if self.should_use_primary(key) {
            match self.primary.put(key, data.clone(), content_type).await {
                Ok(()) => return Ok(()),
                Err(e) => {
                    tracing::warn!(key, error = %e, "主存储写入失败，回退到本地存储");
                }
            }
        }
        self.fallback.put(key, data, content_type).await
    }

    async fn get(&self, key: &str) -> Result<Bytes, StorageError> {
        match self.primary.get(key).await {
            Ok(data) => return Ok(data),
            Err(StorageError::NotFound(_)) => {}
            Err(e) => {
                tracing::warn!(key, error = %e, "从主存储读取失败，尝试本地存储");
            }
        }
        self.fallback.get(key).await
    }

    async fn list(&self, prefix: &str) -> Result<Vec<FileInfo>, StorageError> {
        let mut results = self.primary.list(prefix).await.unwrap_or_default();
        let fallback_results = self.fallback.list(prefix).await.unwrap_or_default();
        let existing_keys: std::collections::HashSet<_> =
            results.iter().map(|f| f.key.clone()).collect();
        for item in fallback_results {
            if !existing_keys.contains(&item.key) {
                results.push(item);
            }
        }
        Ok(results)
    }

    async fn exists(&self, key: &str) -> Result<bool, StorageError> {
        if self.primary.exists(key).await? {
            return Ok(true);
        }
        self.fallback.exists(key).await
    }

    async fn delete(&self, key: &str) -> Result<(), StorageError> {
        let _ = self.primary.delete(key).await;
        self.fallback.delete(key).await
    }

    async fn presign_get(&self, key: &str, expires_secs: u64) -> Result<String, StorageError> {
        match self.primary.presign_get(key, expires_secs).await {
            Ok(url) => Ok(url),
            Err(_) => self.fallback.presign_get(key, expires_secs).await,
        }
    }

    fn backend_name(&self) -> &str {
        "dual"
    }
}
