use crate::{normalize_page, AppState, PageQuery, PaginatedResponse};
use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::IntoResponse,
    Json,
};
use serde::Serialize;

#[derive(Clone, Serialize)]
pub struct RecordingInfo {
    pub call_id: String,
    pub stem: String,
    pub size_bytes: u64,
    pub duration_secs: f64,
    pub created_at_ms: i64,
    pub has_audio: bool,
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
    Query(query): Query<PageQuery>,
) -> Result<Json<PaginatedResponse<RecordingInfo>>, (StatusCode, String)> {
    let storage = &state.recording_storage;
    let prefix = "";

    let files = storage
        .list(prefix)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let mut out = Vec::new();
    for file in files {
        if !file.key.ends_with(".wav") {
            continue;
        }
        let wav_key = file.key.clone();
        let stem = wav_key.trim_end_matches(".wav").to_string();
        let has_audio = true;
        let size = file.size;
        // WAV: 8kHz stereo 16-bit = 32000 bytes/sec, minus 44 byte header
        let duration_secs = if size > 44 {
            ((size - 44) as f64) / 32000.0
        } else {
            0.0
        };
        out.push(RecordingInfo {
            call_id: stem.clone(),
            stem,
            size_bytes: size,
            duration_secs,
            created_at_ms: file.last_modified.unwrap_or_default(),
            has_audio,
        });
    }

    out.sort_by(|a, b| b.created_at_ms.cmp(&a.created_at_ms));
    let total = i64::try_from(out.len()).unwrap_or(i64::MAX);
    let (page, page_size, offset) = normalize_page(&query);
    let start = usize::try_from(offset).unwrap_or(usize::MAX).min(out.len());
    let end = start
        .saturating_add(usize::try_from(page_size).unwrap_or(0))
        .min(out.len());
    Ok(Json(PaginatedResponse {
        items: out[start..end].to_vec(),
        total,
        page,
        page_size,
    }))
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
        .find(|f| f.key.ends_with(".wav") && f.key.trim_end_matches(".wav") == prefix)
        .map(|f| f.key.clone())
        .ok_or_else(|| (StatusCode::NOT_FOUND, "未找到该通话的录音".into()))?;

    let bytes = storage
        .get(&wav_key)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

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
