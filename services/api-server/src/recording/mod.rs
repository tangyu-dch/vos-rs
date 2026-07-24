use crate::AppState;
use axum::{
    extract::{Path, State},
    http::{header, HeaderMap, StatusCode},
};
use std::{
    collections::HashMap,
    path::Path as FsPath,
    time::{Duration, SystemTime},
};
use storage_core::StorageBackend;

mod audio;
mod loader;

use audio::build_audio_response;
use loader::{load_legacy_recording, load_recording_path};

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
    request_headers: HeaderMap,
) -> Result<axum::response::Response, (StatusCode, String)> {
    let storage = &state.recording_storage;
    let prefix = sanitize_call_id(&call_id);
    tracing::info!(
        call_id,
        prefix,
        backend = storage.backend_name(),
        "录音查询"
    );

    let cdr_recording_path = match state.store.get_cdr(&call_id).await {
        Ok(cdr) => cdr.and_then(|event| event.recording_path),
        Err(error) => {
            tracing::warn!(%error, %call_id, "读取 CDR 录音路径失败，回退到旧式录音查找");
            None
        }
    };
    let bytes = if let Some(path) = cdr_recording_path.as_deref() {
        load_recording_path(&state, path).await?
    } else {
        load_legacy_recording(storage.as_ref(), &prefix).await?
    };

    let requested_range = request_headers
        .get(header::RANGE)
        .and_then(|value| value.to_str().ok());
    build_audio_response(bytes, &prefix, requested_range)
}

#[cfg(test)]
mod tests {
    use super::sync_local_recordings_with_policy;
    use std::{collections::HashMap, time::Duration};
    use storage_core::local::LocalStorage;

    #[tokio::test]
    async fn successful_archive_deletes_local_source_when_enabled() {
        let test_root =
            std::env::temp_dir().join(format!("vos-rs-recording-archive-{}", std::process::id()));
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
        let test_root =
            std::env::temp_dir().join(format!("vos-rs-recording-dual-{}", std::process::id()));
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
