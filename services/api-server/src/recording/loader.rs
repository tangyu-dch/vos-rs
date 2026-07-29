use crate::AppState;
use axum::{body::Bytes, http::StatusCode};
use std::path::Path as FsPath;
use storage_core::{StorageBackend, StorageError};

pub(crate) async fn load_legacy_recording(
    storage: &dyn StorageBackend,
    prefix: &str,
) -> Result<Bytes, (StatusCode, String)> {
    match storage.get(&(prefix.to_string() + ".wav")).await {
        Ok(bytes) => {
            tracing::info!(prefix, "直接匹配录音成功");
            Ok(bytes)
        }
        Err(error @ StorageError::NotFound(_)) => {
            tracing::info!(prefix, %error, "直接匹配失败，尝试列表查询");
            // sip-edge 文件名格式: {sanitized_call_id}-{timestamp_ms}.wav
            // 只查询当前 call_id 前缀，避免在大 bucket 中扫描全部录音。
            let files = storage.list(prefix).await.map_err(|e| {
                tracing::error!(%e, "列表查询失败");
                (
                    StatusCode::SERVICE_UNAVAILABLE,
                    "录音存储暂时不可用".to_string(),
                )
            })?;
            tracing::info!(count = files.len(), "列表查询返回文件数");
            let wav_key = files
                .iter()
                .filter(|f| f.key.ends_with(".wav"))
                .find(|f| {
                    let stem = f.key.trim_end_matches(".wav");
                    stem == prefix || stem.starts_with(&format!("{prefix}-"))
                })
                .map(|f| {
                    tracing::info!(key = %f.key, "找到匹配录音");
                    f.key.clone()
                })
                .ok_or_else(|| {
                    tracing::warn!(prefix, "未找到匹配的录音文件");
                    (StatusCode::NOT_FOUND, "未找到该通话的录音".into())
                })?;
            storage
                .get(&wav_key)
                .await
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
        }
        Err(error) => {
            tracing::error!(%error, "读取录音存储失败");
            Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "录音存储暂时不可用".to_string(),
            ))
        }
    }
}

pub(crate) async fn load_recording_path(
    state: &AppState,
    recording_path: &str,
) -> Result<Bytes, (StatusCode, String)> {
    if let Some(path) = recording_path.strip_prefix("local:") {
        let configured_root = state
            .store
            .get_system_config("recording_dir")
            .await
            .ok()
            .flatten()
            .filter(|value| !value.trim().is_empty())
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| state.recording_local_dir.clone());
        let trusted_roots = [
            configured_root,
            state.recording_local_dir.clone(),
            "recordings".into(),
            "target/recordings".into(),
            "target/test_recordings".into(),
        ];
        return load_local_recording_with_fallback(
            &trusted_roots,
            FsPath::new(path),
            state.recording_storage.as_ref(),
        )
        .await;
    }
    let key = recording_path
        .strip_prefix("oss:")
        .or_else(|| recording_path.strip_prefix("s3:"))
        .unwrap_or(recording_path);
    state
        .recording_storage
        .get(key)
        .await
        .map_err(|error| match error {
            StorageError::NotFound(_) => (StatusCode::NOT_FOUND, "未找到该通话的录音".into()),
            other => (StatusCode::SERVICE_UNAVAILABLE, other.to_string()),
        })
}

async fn load_local_recording_with_fallback(
    configured_roots: &[std::path::PathBuf],
    requested_path: &FsPath,
    storage: &dyn StorageBackend,
) -> Result<Bytes, (StatusCode, String)> {
    match load_local_recording(configured_roots, requested_path).await {
        Ok(bytes) => Ok(bytes),
        Err((StatusCode::NOT_FOUND, _)) => {
            let key = requested_path
                .file_name()
                .and_then(|value| value.to_str())
                .ok_or_else(|| (StatusCode::FORBIDDEN, "录音路径不合法".into()))?;
            tracing::info!(path = %requested_path.display(), key, "本地录音不存在，查询归档存储");
            storage.get(key).await.map_err(|error| match error {
                StorageError::NotFound(_) => (StatusCode::NOT_FOUND, "未找到该通话的录音".into()),
                other => {
                    tracing::error!(key, %other, "读取归档录音失败");
                    (StatusCode::SERVICE_UNAVAILABLE, "录音存储暂时不可用".into())
                }
            })
        }
        Err(error) => Err(error),
    }
}

pub(crate) async fn load_local_recording(
    configured_roots: &[std::path::PathBuf],
    requested_path: &FsPath,
) -> Result<Bytes, (StatusCode, String)> {
    if requested_path.extension().and_then(|value| value.to_str()) != Some("wav") {
        return Err((StatusCode::FORBIDDEN, "录音路径不合法".into()));
    }
    let path = tokio::fs::canonicalize(requested_path)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "未找到该通话的录音".into()))?;
    let mut trusted = false;
    for root in configured_roots {
        if let Ok(root) = tokio::fs::canonicalize(root).await {
            if path.starts_with(root) {
                trusted = true;
                break;
            }
        }
    }
    if !trusted {
        tracing::warn!(path = %path.display(), "拒绝读取受信任录音目录之外的路径");
        return Err((StatusCode::FORBIDDEN, "录音路径不合法".into()));
    }
    tokio::fs::read(path)
        .await
        .map(Bytes::from)
        .map_err(|error| (StatusCode::SERVICE_UNAVAILABLE, error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::{load_local_recording, load_local_recording_with_fallback};
    use axum::http::StatusCode;
    use storage_core::{local::LocalStorage, StorageBackend};

    #[tokio::test]
    async fn local_cdr_path_is_read_only_inside_recording_root() {
        let test_root =
            std::env::temp_dir().join(format!("vos-rs-recording-path-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&test_root)
            .await
            .expect("recording root");
        let recording = test_root.join("call-50.wav");
        tokio::fs::write(&recording, b"RIFF-call-50")
            .await
            .expect("recording file");

        let bytes = load_local_recording(std::slice::from_ref(&test_root), &recording)
            .await
            .expect("recording inside root should load");

        assert_eq!(bytes.as_ref(), b"RIFF-call-50");
        let _ = tokio::fs::remove_dir_all(test_root).await;
    }

    #[tokio::test]
    async fn local_cdr_path_outside_recording_root_is_rejected() {
        let base = std::env::temp_dir().join(format!(
            "vos-rs-recording-boundary-{}",
            uuid::Uuid::new_v4()
        ));
        let root = base.join("recordings");
        tokio::fs::create_dir_all(&root)
            .await
            .expect("recording root");
        let outside = base.join("secret.wav");
        tokio::fs::write(&outside, b"RIFF-secret")
            .await
            .expect("outside file");

        let error = load_local_recording(std::slice::from_ref(&root), &outside)
            .await
            .expect_err("outside path must be rejected");

        assert_eq!(error.0, StatusCode::FORBIDDEN);
        let _ = tokio::fs::remove_dir_all(base).await;
    }

    #[tokio::test]
    async fn historical_recording_root_remains_readable_after_root_change() {
        let base =
            std::env::temp_dir().join(format!("vos-rs-recording-history-{}", uuid::Uuid::new_v4()));
        let current = base.join("recordings");
        let historical = base.join("test_recordings");
        tokio::fs::create_dir_all(&current)
            .await
            .expect("current root");
        tokio::fs::create_dir_all(&historical)
            .await
            .expect("historical root");
        let recording = historical.join("call-50.wav");
        tokio::fs::write(&recording, b"RIFF-history")
            .await
            .expect("historical recording");

        let bytes = load_local_recording(&[current, historical], &recording)
            .await
            .expect("trusted historical root should remain readable");

        assert_eq!(bytes.as_ref(), b"RIFF-history");
        let _ = tokio::fs::remove_dir_all(base).await;
    }

    #[tokio::test]
    async fn missing_local_recording_falls_back_to_archive_storage() {
        let base = std::env::temp_dir().join(format!(
            "vos-rs-recording-fallback-{}",
            uuid::Uuid::new_v4()
        ));
        let local_root = base.join("local");
        let archive_root = base.join("archive");
        tokio::fs::create_dir_all(&local_root)
            .await
            .expect("local root");
        let storage = LocalStorage::new(archive_root.to_str().expect("archive path"))
            .expect("archive storage");
        storage
            .put(
                "call-archive.wav",
                b"RIFF-archive".as_slice().into(),
                Some("audio/wav"),
            )
            .await
            .expect("archive recording");

        let bytes = load_local_recording_with_fallback(
            std::slice::from_ref(&local_root),
            &local_root.join("call-archive.wav"),
            &storage,
        )
        .await
        .expect("archived recording should load");

        assert_eq!(bytes.as_ref(), b"RIFF-archive");
        let _ = tokio::fs::remove_dir_all(base).await;
    }

    #[tokio::test]
    async fn invalid_local_recording_path_does_not_fall_back() {
        let base =
            std::env::temp_dir().join(format!("vos-rs-recording-invalid-{}", uuid::Uuid::new_v4()));
        let local_root = base.join("local");
        let archive_root = base.join("archive");
        tokio::fs::create_dir_all(&local_root)
            .await
            .expect("local root");
        let storage = LocalStorage::new(archive_root.to_str().expect("archive path"))
            .expect("archive storage");
        storage
            .put("call.txt", b"not-a-recording".as_slice().into(), None)
            .await
            .expect("archive object");

        let error = load_local_recording_with_fallback(
            std::slice::from_ref(&local_root),
            &local_root.join("call.txt"),
            &storage,
        )
        .await
        .expect_err("invalid extension must not query archive storage");

        assert_eq!(error.0, StatusCode::FORBIDDEN);
        let _ = tokio::fs::remove_dir_all(base).await;
    }
}
