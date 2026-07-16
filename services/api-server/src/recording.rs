use crate::AppState;
use axum::{
    body::Bytes,
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

fn build_audio_response(
    bytes: Bytes,
    prefix: &str,
    requested_range: Option<&str>,
) -> Result<axum::response::Response, (StatusCode, String)> {
    let selected_range = match parse_byte_range(requested_range, bytes.len()) {
        Ok(range) => range,
        Err(()) => return Ok(range_not_satisfiable(bytes.len())),
    };
    let (status, body, content_range) = if let Some((start, end)) = selected_range {
        (
            StatusCode::PARTIAL_CONTENT,
            bytes.slice(start..=end),
            Some(format!("bytes {start}-{end}/{}", bytes.len())),
        )
    } else {
        (StatusCode::OK, bytes, None)
    };
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("audio/wav"));
    headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    headers.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!("inline; filename=\"{prefix}.wav\""))
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "录音文件名无效".into()))?,
    );
    headers.insert(
        axum::http::HeaderName::from_static("x-content-type-options"),
        HeaderValue::from_static("nosniff"),
    );
    if let Some(value) = content_range {
        headers.insert(
            header::CONTENT_RANGE,
            HeaderValue::from_str(&value)
                .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "音频范围无效".into()))?,
        );
    }
    headers.insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&body.len().to_string())
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "音频长度无效".into()))?,
    );
    Ok((status, headers, body).into_response())
}

async fn load_legacy_recording(
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

async fn load_recording_path(
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
        return load_local_recording(&trusted_roots, FsPath::new(path)).await;
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

async fn load_local_recording(
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

fn parse_byte_range(value: Option<&str>, length: usize) -> Result<Option<(usize, usize)>, ()> {
    let Some(value) = value else {
        return Ok(None);
    };
    let spec = value.strip_prefix("bytes=").ok_or(())?;
    if spec.contains(',') || length == 0 {
        return Err(());
    }
    let (start, end) = spec.split_once('-').ok_or(())?;
    if start.is_empty() {
        let suffix = end.parse::<usize>().map_err(|_| ())?;
        if suffix == 0 {
            return Err(());
        }
        let size = suffix.min(length);
        return Ok(Some((length - size, length - 1)));
    }
    let start = start.parse::<usize>().map_err(|_| ())?;
    if start >= length {
        return Err(());
    }
    let end = if end.is_empty() {
        length - 1
    } else {
        end.parse::<usize>().map_err(|_| ())?.min(length - 1)
    };
    if end < start {
        return Err(());
    }
    Ok(Some((start, end)))
}

fn range_not_satisfiable(length: usize) -> axum::response::Response {
    let mut headers = HeaderMap::new();
    headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    if let Ok(value) = HeaderValue::from_str(&format!("bytes */{length}")) {
        headers.insert(header::CONTENT_RANGE, value);
    }
    (StatusCode::RANGE_NOT_SATISFIABLE, headers).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use storage_core::local::LocalStorage;

    #[test]
    fn parses_browser_byte_ranges() {
        assert_eq!(parse_byte_range(None, 100), Ok(None));
        assert_eq!(
            parse_byte_range(Some("bytes=10-19"), 100),
            Ok(Some((10, 19)))
        );
        assert_eq!(parse_byte_range(Some("bytes=90-"), 100), Ok(Some((90, 99))));
        assert_eq!(parse_byte_range(Some("bytes=-10"), 100), Ok(Some((90, 99))));
        assert_eq!(
            parse_byte_range(Some("bytes=90-200"), 100),
            Ok(Some((90, 99)))
        );
    }

    #[test]
    fn rejects_invalid_or_multiple_ranges() {
        assert!(parse_byte_range(Some("bytes=100-"), 100).is_err());
        assert!(parse_byte_range(Some("bytes=20-10"), 100).is_err());
        assert!(parse_byte_range(Some("bytes=0-1,4-5"), 100).is_err());
        assert!(parse_byte_range(Some("items=0-1"), 100).is_err());
    }

    #[test]
    fn unsatisfied_range_advertises_complete_length() {
        let response = range_not_satisfiable(1234);
        assert_eq!(response.status(), StatusCode::RANGE_NOT_SATISFIABLE);
        assert_eq!(
            response.headers().get(header::CONTENT_RANGE),
            Some(&HeaderValue::from_static("bytes */1234"))
        );
        assert_eq!(
            response.headers().get(header::ACCEPT_RANGES),
            Some(&HeaderValue::from_static("bytes"))
        );
    }

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
