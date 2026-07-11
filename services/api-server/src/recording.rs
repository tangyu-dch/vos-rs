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
use storage_core::StorageBackend;

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
            .is_some_and(|age| age < Duration::from_secs(10));
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
            uploaded_sizes.insert(file_name.to_string(), size);
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

    // 优先按确定的对象 key 读取，避免依赖 S3 ListObjects 权限；分段录音再回退到列表查询。
    let bytes = match storage.get(&(prefix.clone() + ".wav")).await {
        Ok(bytes) => bytes,
        Err(_) => {
            let files = storage
                .list("")
                .await
                .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
            let wav_key = files
                .iter()
                .find(|f| f.key.ends_with(".wav") && f.key.trim_end_matches(".wav") == prefix)
                .map(|f| f.key.clone())
                .ok_or_else(|| (StatusCode::NOT_FOUND, "未找到该通话的录音".into()))?;
            storage
                .get(&wav_key)
                .await
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
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
