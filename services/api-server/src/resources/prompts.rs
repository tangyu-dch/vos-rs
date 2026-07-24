//! IVR 音频提示音文件管理：上传 / 列表 / 下载 / 删除
//!
//! 文件落盘到 `VOS_RS_PROMPTS_DIR` (默认 `./prompts`) 目录,
//! 前端通过 multipart 上传音频文件 (wav/mp3), 通过 GET 接口试听或下载。

use std::path::PathBuf;

use axum::{
    body::Body,
    extract::{Multipart, Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;
use serde_json::{json, Value};
use tokio::fs;
use tokio::io::AsyncReadExt;

use crate::{ApiError, AppState};

/// 允许上传的音频扩展名
const ALLOWED_EXTS: &[&str] = &["wav", "mp3", "gsm", "ogg"];

/// 单文件最大 50 MB
const MAX_FILE_SIZE: usize = 50 * 1024 * 1024;

#[derive(Debug, Serialize)]
pub(crate) struct PromptFile {
    pub filename: String,
    pub size: u64,
    pub content_type: String,
    pub url: String,
}

/// 解析提示音存储目录 (环境变量优先, 默认 `./prompts`)
fn prompts_dir() -> PathBuf {
    let dir = std::env::var("VOS_RS_PROMPTS_DIR").unwrap_or_else(|_| "prompts".to_string());
    PathBuf::from(dir)
}

/// 计算安全的存储文件名 (避免路径穿越)
///
/// 保留原文件名的主干部分, 追加纳秒时间戳防止冲突, 仅保留 ASCII 字母数字/下划线/连字符。
fn sanitize_filename(original: &str) -> String {
    let stem = original
        .split('.')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("prompt");
    let cleaned: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let ext = original
        .rsplit('.')
        .next()
        .map(|e| e.to_lowercase())
        .filter(|e| ALLOWED_EXTS.contains(&e.as_str()))
        .unwrap_or_else(|| "wav".to_string());
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{cleaned}_{ts}.{ext}")
}

/// 从文件名中提取安全的纯文件名部分 (防止路径穿越)
fn safe_file_name(filename: &str) -> Option<String> {
    let safe = PathBuf::from(filename)
        .file_name()
        .and_then(|n| n.to_str())?
        .to_string();
    if safe.is_empty() || safe.contains('/') || safe.contains('\\') {
        None
    } else {
        Some(safe)
    }
}

/// `POST /api/v1/ivr/prompts/upload`
///
/// 接收 multipart 字段 `file` (音频文件), 保存到 prompts 目录,
/// 返回 `{ filename, size, content_type, url }` 供前端持久化到节点 config。
pub async fn upload_prompt(
    State(_state): State<AppState>,
    mut multipart: Multipart,
) -> Result<Json<Value>, ApiError> {
    let dir = prompts_dir();
    fs::create_dir_all(&dir)
        .await
        .map_err(|e| ApiError::internal(format!("创建 prompts 目录失败: {e}")))?;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::internal(format!("读取 multipart 字段失败: {e}")))?
    {
        if field.name() != Some("file") {
            continue;
        }
        let original = field
            .file_name()
            .map(|s| s.to_string())
            .unwrap_or_else(|| "prompt.wav".to_string());
        let content_type = field
            .content_type()
            .map(|s| s.to_string())
            .unwrap_or_else(|| "audio/wav".to_string());

        let bytes = field
            .bytes()
            .await
            .map_err(|e| ApiError::internal(format!("读取上传数据失败: {e}")))?;
        if bytes.len() > MAX_FILE_SIZE {
            return Err(ApiError::bad_request(format!(
                "文件大小超过限制 ({}MB)",
                MAX_FILE_SIZE / 1024 / 1024
            )));
        }
        if bytes.is_empty() {
            return Err(ApiError::bad_request("上传文件为空"));
        }

        let stored_name = sanitize_filename(&original);
        let path = dir.join(&stored_name);
        fs::write(&path, &bytes)
            .await
            .map_err(|e| ApiError::internal(format!("写入文件失败: {e}")))?;

        let size = bytes.len() as u64;
        let url = format!("/api/v1/ivr/prompts/{stored_name}");
        return Ok(Json(json!({
            "filename": stored_name,
            "original_name": original,
            "size": size,
            "content_type": content_type,
            "url": url,
        })));
    }
    Err(ApiError::bad_request("未找到名为 file 的上传字段"))
}

/// `GET /api/v1/ivr/prompts`
///
/// 列出已上传的所有音频提示音文件。
pub async fn list_prompts(
    State(_state): State<AppState>,
) -> Result<Json<Vec<PromptFile>>, ApiError> {
    let dir = prompts_dir();
    if !dir.exists() {
        return Ok(Json(Vec::new()));
    }
    let mut entries = fs::read_dir(&dir)
        .await
        .map_err(|e| ApiError::internal(format!("读取 prompts 目录失败: {e}")))?;
    let mut files = Vec::new();
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| ApiError::internal(format!("遍历 prompts 目录失败: {e}")))?
    {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let filename = match path.file_name().and_then(|n| n.to_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|s| s.to_lowercase())
            .unwrap_or_default();
        if !ALLOWED_EXTS.contains(&ext.as_str()) {
            continue;
        }
        let meta = entry
            .metadata()
            .await
            .map_err(|e| ApiError::internal(format!("读取文件元数据失败: {e}")))?;
        let content_type = match ext.as_str() {
            "wav" => "audio/wav",
            "mp3" => "audio/mpeg",
            "gsm" => "audio/gsm",
            "ogg" => "audio/ogg",
            _ => "application/octet-stream",
        }
        .to_string();
        files.push(PromptFile {
            url: format!("/api/v1/ivr/prompts/{filename}"),
            filename,
            size: meta.len(),
            content_type,
        });
    }
    files.sort_by(|a, b| b.filename.cmp(&a.filename));
    Ok(Json(files))
}

/// `GET /api/v1/ivr/prompts/:filename`
///
/// 下载或试听指定音频文件, 内联返回 (支持浏览器 `<audio>` 标签直接播放)。
pub async fn get_prompt(
    State(_state): State<AppState>,
    Path(filename): Path<String>,
) -> Result<Response, ApiError> {
    let safe_name =
        safe_file_name(&filename).ok_or_else(|| ApiError::bad_request("参数无效: 文件名非法"))?;
    let path = prompts_dir().join(&safe_name);
    if !path.exists() {
        return Err(ApiError::not_found(format!("文件 {filename} 不存在")));
    }
    let mut file = fs::File::open(&path)
        .await
        .map_err(|e| ApiError::internal(format!("打开文件失败: {e}")))?;
    let meta = file
        .metadata()
        .await
        .map_err(|e| ApiError::internal(format!("读取文件元数据失败: {e}")))?;
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_lowercase())
        .unwrap_or_default();
    let content_type = match ext.as_str() {
        "wav" => "audio/wav",
        "mp3" => "audio/mpeg",
        "gsm" => "audio/gsm",
        "ogg" => "audio/ogg",
        _ => "application/octet-stream",
    };
    let mut buffer = Vec::with_capacity(meta.len() as usize);
    file.read_to_end(&mut buffer)
        .await
        .map_err(|e| ApiError::internal(format!("读取文件内容失败: {e}")))?;
    let content_length = meta.len().to_string();
    let response = (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, content_type),
            (header::CONTENT_LENGTH, content_length.as_str()),
            (header::CACHE_CONTROL, "public, max-age=3600"),
        ],
        Body::from(buffer),
    )
        .into_response();
    Ok(response)
}

/// `DELETE /api/v1/ivr/prompts/:filename`
///
/// 删除指定的音频提示音文件。
pub async fn delete_prompt(
    State(_state): State<AppState>,
    Path(filename): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let safe_name =
        safe_file_name(&filename).ok_or_else(|| ApiError::bad_request("参数无效: 文件名非法"))?;
    let path = prompts_dir().join(&safe_name);
    if !path.exists() {
        return Err(ApiError::not_found(format!("文件 {filename} 不存在")));
    }
    fs::remove_file(&path)
        .await
        .map_err(|e| ApiError::internal(format!("删除文件失败: {e}")))?;
    Ok(Json(json!({ "success": true, "filename": safe_name })))
}
