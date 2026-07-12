use crate::AppState;
use axum::{
    extract::{Path, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::IntoResponse,
};
use std::{
    collections::HashMap,
    path::Path as FsPath,
    time::{Duration, SystemTime},
};
use storage_core::{StorageBackend, StorageError};

/// 将已经停止写入的本地 WAV 归档到对象存储。
///
/// SIP Edge 录音使用本地顺序写入以保证 RTP 热路径稳定，API Server 负责把
/// 闭合后的文件异步归档到 RustFS。文件最后修改时间小于 10 秒时跳过，避免
/// 上传仍在增长的 WAV 文件。
pub async fn sync_local_recordings(
    storage: &dyn StorageBackend,
    directory: &FsPath,
    uploaded_sizes: &mut HashMap<String, u64>,
) -> usize {
    sync_local_recordings_with_policy(
        storage,
        directory,
        uploaded_sizes,
        Duration::from_secs(10),
        storage.backend_name() == "oss",
    )
    .await
}

async fn sync_local_recordings_with_policy(
    storage: &dyn StorageBackend,
    directory: &FsPath,
    uploaded_sizes: &mut HashMap<String, u64>,
    minimum_age: Duration,
    delete_after_upload: bool,
) -> usize {
    let Ok(mut entries) = tokio::fs::read_dir(directory).await else {
        return 0;
    };
    let mut uploaded = 0;
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("wav") {
            continue;
        }
        let Ok(metadata) = entry.metadata().await else {
            tracing::warn!(path = %path.display(), "读取录音文件元数据失败");
            continue;
        };
        let recently_modified = metadata
            .modified()
            .ok()
            .and_then(|modified| SystemTime::now().duration_since(modified).ok())
            .is_some_and(|age| age < minimum_age);
        if recently_modified {
            continue;
        }
        let Some(file_name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        let size = metadata.len();
        if uploaded_sizes.get(file_name) == Some(&size) {
            continue;
        }
        let Ok(data) = tokio::fs::read(&path).await else {
            tracing::warn!(path = %path.display(), "读取录音文件失败，稍后重试归档");
            continue;
        };
        if let Err(error) = storage.put(file_name, data.into(), Some("audio/wav")).await {
            tracing::warn!(file_name, %error, "录音归档到对象存储失败，稍后重试");
        } else {
            if delete_after_upload {
                match storage.exists(file_name).await {
                    Ok(true) => {
                        if let Err(error) = tokio::fs::remove_file(&path).await {
                            tracing::warn!(file_name, %error, "录音已归档，但删除本地文件失败");
                            continue;
                        }
                        tracing::info!(file_name, "录音已归档并删除本地文件");
                    }
                    Ok(false) => {
                        tracing::warn!(file_name, "录音上传后未能在对象存储中确认，保留本地文件");
                        continue;
                    }
                    Err(error) => {
                        tracing::warn!(file_name, %error, "无法确认对象存储中的录音，保留本地文件");
                        continue;
                    }
                }
            } else {
                uploaded_sizes.insert(file_name.to_string(), size);
            }
            uploaded += 1;
        }
    }
    uploaded
}

/// 复刻 sip-edge `recording_file_stem` 的清洗规则：非 [A-Za-z0-9-_.] 替换为 `_`。
fn sanitize_call_id(call_id: &str) -> String {
    call_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

/// 按 call_id 获取录音音频流（WAV）。
pub async fn get_recording_audio(
    State(state): State<AppState>,
    Path(call_id): Path<String>,
) -> Result<axum::response::Response, (StatusCode, String)> {
    let storage = &state.recording_storage;
    let prefix = sanitize_call_id(&call_id);
    tracing::info!(
        call_id,
        prefix,
        backend = storage.backend_name(),
        "录音查询"
    );

    // 优先按确定的对象 key 读取，避免依赖 S3 ListObjects 权限；分段录音再回退到列表查询。
    let bytes = match storage.get(&(prefix.clone() + ".wav")).await {
        Ok(bytes) => {
            tracing::info!(prefix, "直接匹配录音成功");
            bytes
        }
        Err(error @ StorageError::NotFound(_)) => {
            tracing::info!(prefix, %error, "直接匹配失败，尝试列表查询");
            // sip-edge 文件名格式: {sanitized_call_id}-{timestamp_ms}.wav
            // 只查询当前 call_id 前缀，避免在大 bucket 中扫描全部录音。
            let files = storage.list(&prefix).await.map_err(|e| {
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
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        }
        Err(error) => {
            tracing::error!(%error, "读取录音存储失败");
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "录音存储暂时不可用".to_string(),
            ));
        }
    };

    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("audio/wav"));
    headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    headers.insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&bytes.len().to_string())
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "音频长度无效".into()))?,
    );
    Ok((StatusCode::OK, headers, bytes).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use storage_core::local::LocalStorage;

    #[tokio::test]
    async fn successful_archive_deletes_local_source_when_enabled() {
        let test_root = std::env::temp_dir().join(format!(
            "vos-rs-recording-archive-{}",
            std::process::id()
        ));
        let source_dir = test_root.join("source");
        let archive_dir = test_root.join("archive");
        tokio::fs::create_dir_all(&source_dir)
            .await
            .expect("source directory should be created");
        let source_path = source_dir.join("call-1.wav");
        tokio::fs::write(&source_path, b"RIFF-test")
            .await
            .expect("source recording should be written");
        let storage = LocalStorage::new(
            archive_dir
                .to_str()
                .expect("archive path should be valid UTF-8"),
        )
        .expect("archive storage should be created");
        let mut uploaded_sizes = HashMap::new();

        let uploaded = sync_local_recordings_with_policy(
            &storage,
            &source_dir,
            &mut uploaded_sizes,
            Duration::ZERO,
            true,
        )
        .await;

        assert_eq!(uploaded, 1);
        assert!(!source_path.exists());
        assert!(archive_dir.join("call-1.wav").exists());
        let _ = tokio::fs::remove_dir_all(test_root).await;
    }

    #[tokio::test]
    async fn successful_archive_keeps_local_source_when_cleanup_is_disabled() {
        let test_root = std::env::temp_dir().join(format!(
            "vos-rs-recording-dual-{}",
            std::process::id()
        ));
        let source_dir = test_root.join("source");
        let archive_dir = test_root.join("archive");
        tokio::fs::create_dir_all(&source_dir)
            .await
            .expect("source directory should be created");
        let source_path = source_dir.join("call-2.wav");
        tokio::fs::write(&source_path, b"RIFF-test")
            .await
            .expect("source recording should be written");
        let storage = LocalStorage::new(
            archive_dir
                .to_str()
                .expect("archive path should be valid UTF-8"),
        )
        .expect("archive storage should be created");
        let mut uploaded_sizes = HashMap::new();

        let uploaded = sync_local_recordings_with_policy(
            &storage,
            &source_dir,
            &mut uploaded_sizes,
            Duration::ZERO,
            false,
        )
        .await;

        assert_eq!(uploaded, 1);
        assert!(source_path.exists());
        assert!(archive_dir.join("call-2.wav").exists());
        let _ = tokio::fs::remove_dir_all(test_root).await;
    }
}
