use axum::{
    extract::{Path, State},
    http::{header, HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::AppState;

#[derive(Serialize)]
pub struct RecordingInfo {
    pub call_id: String,
    pub stem: String,
    pub size_bytes: u64,
    pub created_at_ms: i64,
    pub has_audio: bool,
}

/// 录音元数据 JSON（仅读取需要的字段）。
#[derive(Deserialize)]
struct RecordingMetadata {
    call_id: String,
    created_at_unix_ms: i64,
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

/// 列出录音目录下所有录音（按创建时间倒序）。
pub async fn list_recordings(
    State(state): State<AppState>,
) -> Result<Json<Vec<RecordingInfo>>, (StatusCode, String)> {
    let dir = state.recording_dir.clone();
    let mut out = Vec::new();

    let mut entries = tokio::fs::read_dir(&dir)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, format!("recording dir error: {e}")))?;

    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.ends_with(".json") {
            continue;
        }
        let path = entry.path();
        let content = match tokio::fs::read_to_string(&path).await {
            Ok(c) => c,
            Err(_) => continue,
        };
        let meta: RecordingMetadata = match serde_json::from_str(&content) {
            Ok(m) => m,
            Err(_) => continue,
        };
        let stem = name.trim_end_matches(".json").to_string();
        let wav_path = path.with_file_name(format!("{stem}.wav"));
        let size = tokio::fs::metadata(&wav_path)
            .await
            .map(|m| m.len())
            .unwrap_or(0);
        out.push(RecordingInfo {
            call_id: meta.call_id,
            stem,
            size_bytes: size,
            created_at_ms: meta.created_at_unix_ms,
            has_audio: size > 0,
        });
    }

    out.sort_by(|a, b| b.created_at_ms.cmp(&a.created_at_ms));
    Ok(Json(out))
}

/// 按 call_id 获取录音音频流（WAV）。
pub async fn get_recording_audio(
    State(state): State<AppState>,
    Path(call_id): Path<String>,
) -> Result<axum::response::Response, (StatusCode, String)> {
    let prefix = sanitize_call_id(&call_id);
    let dir = state.recording_dir.clone();

    let mut entries = tokio::fs::read_dir(&dir)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, format!("recording dir error: {e}")))?;

    let mut wav_path: Option<PathBuf> = None;
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with(&prefix) && name.ends_with(".wav") {
            wav_path = Some(entry.path());
            break;
        }
    }

    let path = wav_path.ok_or((StatusCode::NOT_FOUND, "no recording for call_id".into()))?;
    let bytes = tokio::fs::read(&path)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, "audio/wav".parse().unwrap());
    headers.insert(header::ACCEPT_RANGES, "bytes".parse().unwrap());
    headers.insert(
        header::CONTENT_LENGTH,
        bytes.len().to_string().parse().unwrap(),
    );
    Ok((StatusCode::OK, headers, bytes).into_response())
}
