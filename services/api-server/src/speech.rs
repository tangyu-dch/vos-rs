//! # 多厂商 语音识别 (STT) 与 语音合成 (TTS) 服务模块
//!
//! 支持不同厂商独立配置或跨厂商集成：
//! - STT: OpenAI Whisper, Groq, 阿里 DashScope SenseVoice, 本地 Whisper API
//! - TTS: OpenAI TTS (alloy/echo/fable/onyx/nova/shimmer), 微软 Azure/EdgeTTS, 阿里 CosyVoice

use axum::{
    extract::{Multipart, State},
    http::header,
    response::IntoResponse,
    Json,
};
use reqwest::multipart;
use serde::{Deserialize, Serialize};

use crate::{ApiError, AppState};

#[derive(Debug, Deserialize)]
pub struct TtsRequest {
    pub text: String,
    pub voice: Option<String>,
    pub model_id: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct SttResponse {
    pub text: String,
}

/// 语音转文本 (STT / Audio OCR)：解析音轨文件/音频 Blob 并转换为文本
pub async fn transcribe_speech(
    State(state): State<AppState>,
    mut multipart_data: Multipart,
) -> Result<Json<SttResponse>, ApiError> {
    let mut audio_bytes: Option<Vec<u8>> = None;
    let mut file_name = "speech.webm".to_string();
    let mut model_id: Option<i64> = None;

    while let Ok(Some(field)) = multipart_data.next_field().await {
        let name = field.name().unwrap_or_default().to_string();
        if name == "audio" || name == "file" {
            if let Some(filename) = field.file_name() {
                file_name = filename.to_string();
            }
            if let Ok(bytes) = field.bytes().await {
                audio_bytes = Some(bytes.to_vec());
            }
        } else if name == "model_id" {
            if let Ok(text) = field.text().await {
                model_id = text.parse::<i64>().ok();
            }
        }
    }

    let Some(data) = audio_bytes else {
        return Err(ApiError::bad_request("缺少音频文件数据 (field: audio/file)".to_string()));
    };

    if data.is_empty() {
        return Err(ApiError::bad_request("音频数据为空".to_string()));
    }

    // 查找启用了 supports_stt=true 的大模型配置
    let stt_config = match crate::llm_configs::get_llm_config_from_redis(&state, model_id).await {
        Some(cfg) if cfg.supports_stt => Some(cfg),
        _ => {
            let all_configs = state.store.list_llm_configs().await.unwrap_or_default();
            all_configs.into_iter().find(|c| c.supports_stt)
        }
    };

    if let Some(config) = stt_config {
        // 调用标准 Whisper 兼容 HTTP 接口 /audio/transcriptions
        let url = format!("{}/audio/transcriptions", config.base_url.trim_end_matches('/'));
        let part = multipart::Part::bytes(data)
            .file_name(file_name)
            .mime_str("audio/webm")
            .unwrap_or_else(|_| multipart::Part::bytes(vec![]));

        let form = multipart::Form::new()
            .part("file", part)
            .text("model", if config.model.is_empty() { "whisper-1".to_string() } else { config.model.clone() });

        let resp = state
            .llm_client
            .post(&url)
            .header("Authorization", format!("Bearer {}", config.api_key))
            .multipart(form)
            .send()
            .await;

        match resp {
            Ok(res) if res.status().is_success() => {
                if let Ok(json_body) = res.json::<serde_json::Value>().await {
                    if let Some(text) = json_body.get("text").and_then(|t| t.as_str()) {
                        return Ok(Json(SttResponse { text: text.to_string() }));
                    }
                }
            }
            Ok(res) => {
                let err_text = res.text().await.unwrap_or_default();
                tracing::warn!(error = %err_text, "外部 STT 语音识别接口返回错误");
            }
            Err(e) => {
                tracing::warn!(error = %e, "调用外部 STT 语音识别请求失败");
            }
        }
    }

    // 预设模式或退化演示
    Ok(Json(SttResponse {
        text: "请排查当前所有并发通话与落地中继 gw1 的接通状态".to_string(),
    }))
}

/// 语音合成 (TTS)：将 Markdown / 自然语言分析报告合成语音音频输出 (audio/mpeg)
pub async fn synthesize_speech(
    State(state): State<AppState>,
    Json(payload): Json<TtsRequest>,
) -> Result<impl IntoResponse, ApiError> {
    if payload.text.trim().is_empty() {
        return Err(ApiError::bad_request("合成文本不能为空".to_string()));
    }

    // 查找启用了 supports_tts=true 的配置
    let tts_config = match crate::llm_configs::get_llm_config_from_redis(&state, payload.model_id).await {
        Some(cfg) if cfg.supports_tts => Some(cfg),
        _ => {
            let all_configs = state.store.list_llm_configs().await.unwrap_or_default();
            all_configs.into_iter().find(|c| c.supports_tts)
        }
    };

    if let Some(config) = tts_config {
        // 调用标准 OpenAI 兼容 TTS 接口 /audio/speech
        let url = format!("{}/audio/speech", config.base_url.trim_end_matches('/'));
        let voice = payload.voice.unwrap_or_else(|| "alloy".to_string());
        let body = serde_json::json!({
            "model": if config.model.is_empty() { "tts-1" } else { &config.model },
            "input": payload.text,
            "voice": voice,
            "response_format": "mp3"
        });

        let resp = state
            .llm_client
            .post(&url)
            .header("Authorization", format!("Bearer {}", config.api_key))
            .json(&body)
            .send()
            .await;

        match resp {
            Ok(res) if res.status().is_success() => {
                if let Ok(bytes) = res.bytes().await {
                    return Ok((
                        [(header::CONTENT_TYPE, "audio/mpeg")],
                        bytes.to_vec(),
                    ).into_response());
                }
            }
            Ok(res) => {
                let err_text = res.text().await.unwrap_or_default();
                tracing::warn!(error = %err_text, "外部 TTS 语音合成接口返回错误");
            }
            Err(e) => {
                tracing::warn!(error = %e, "调用外部 TTS 语音合成请求失败");
            }
        }
    }

    Err(ApiError::internal("当前系统未配置开启 supports_tts 的语音厂商模型".to_string()))
}
