use async_trait::async_trait;
use bytes::Bytes;
use std::path::PathBuf;
use tokio::fs;

use crate::{FileInfo, StorageBackend, StorageError};

/// 本地文件系统存储后端。
pub struct LocalStorage {
    base_dir: PathBuf,
}

impl LocalStorage {
    pub fn new(base_dir: &str) -> Result<Self, StorageError> {
        let base = PathBuf::from(base_dir);
        if !base.exists() {
            std::fs::create_dir_all(&base)?;
        }
        Ok(Self { base_dir: base })
    }

    fn resolve_path(&self, key: &str) -> PathBuf {
        self.base_dir.join(key)
    }
}

#[async_trait]
impl StorageBackend for LocalStorage {
    async fn put(
        &self,
        key: &str,
        data: Bytes,
        _content_type: Option<&str>,
    ) -> Result<(), StorageError> {
        let path = self.resolve_path(key);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await?;
        }
        fs::write(&path, data).await?;
        Ok(())
    }

    async fn get(&self, key: &str) -> Result<Bytes, StorageError> {
        let path = self.resolve_path(key);
        if !path.exists() {
            return Err(StorageError::NotFound(key.to_string()));
        }
        let data = fs::read(&path).await?;
        Ok(Bytes::from(data))
    }

    async fn list(&self, prefix: &str) -> Result<Vec<FileInfo>, StorageError> {
        let dir = self.resolve_path(prefix);
        if !dir.exists() {
            return Ok(Vec::new());
        }

        let mut results = Vec::new();
        let mut entries = fs::read_dir(&dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let metadata = entry.metadata().await?;
            let name = entry.file_name().to_string_lossy().to_string();
            let full_key = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{}/{}", prefix.trim_end_matches('/'), name)
            };
            results.push(FileInfo {
                key: full_key,
                size: metadata.len(),
                content_type: None,
                last_modified: metadata
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_millis() as i64),
            });
        }
        Ok(results)
    }

    async fn exists(&self, key: &str) -> Result<bool, StorageError> {
        Ok(self.resolve_path(key).exists())
    }

    async fn delete(&self, key: &str) -> Result<(), StorageError> {
        let path = self.resolve_path(key);
        if path.exists() {
            fs::remove_file(&path).await?;
        }
        Ok(())
    }

    async fn presign_get(&self, key: &str, _expires_secs: u64) -> Result<String, StorageError> {
        let path = self.resolve_path(key);
        if !path.exists() {
            return Err(StorageError::NotFound(key.to_string()));
        }
        Ok(path.to_string_lossy().into_owned())
    }

    fn backend_name(&self) -> &str {
        "local"
    }
}
