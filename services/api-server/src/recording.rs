use crate::AppState;
use axum::{
    extract::{Path, State},
    http::{header, HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
pub struct RecordingInfo {
    pub call_id: String,
    pub stem: String,
    pub size_bytes: u64,
    pub duration_secs: f64,
    pub created_at_ms: i64,
    pub has_audio: bool,
}

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
    let storage = &state.recording_storage;
    let prefix = "";

    let files = storage
        .list(prefix)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let mut out = Vec::new();
    for file in files {
        if !file.key.ends_with(".json") {
            continue;
        }
        let content = match storage.get(&file.key).await {
            Ok(c) => c,
            Err(_) => continue,
        };
        let meta: RecordingMetadata = match serde_json::from_slice(&content) {
            Ok(m) => m,
            Err(_) => continue,
        };
        let stem = file.key.trim_end_matches(".json").to_string();
        let wav_key = format!("{stem}.wav");
        let has_audio = storage.exists(&wav_key).await.unwrap_or(false);
        let size = if has_audio {
            storage
                .get(&wav_key)
                .await
                .map(|b| b.len() as u64)
                .unwrap_or(0)
        } else {
            0
        };
        // WAV: 8kHz stereo 16-bit = 32000 bytes/sec, minus 44 byte header
        let duration_secs = if size > 44 {
            ((size - 44) as f64) / 32000.0
        } else {
            0.0
        };
        out.push(RecordingInfo {
            call_id: meta.call_id,
            stem,
            size_bytes: size,
            duration_secs,
            created_at_ms: meta.created_at_unix_ms,
            has_audio,
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
    let storage = &state.recording_storage;
    let prefix = sanitize_call_id(&call_id);

    let files = storage
        .list("")
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;

    let wav_key = files
        .iter()
        .find(|f| f.key.starts_with(&prefix) && f.key.ends_with(".wav"))
        .map(|f| f.key.clone())
        .ok_or_else(|| (StatusCode::NOT_FOUND, "未找到该通话的录音".into()))?;

    let bytes = storage
        .get(&wav_key)
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
