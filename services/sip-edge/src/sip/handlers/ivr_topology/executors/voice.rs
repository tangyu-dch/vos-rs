//! 语音节点执行器：tts / asr
//!
//! 接入 [`super::super::voice_engine`] 提供的 TTS/ASR 引擎：
//! - TTS：文本合成 PCM -> 写入临时 WAV -> 复用媒体层 `start_playback` 播放
//! - ASR：返回 `WaitForAsr` 信号，由 [`super::super::engine::TopologyEngine`] 负责实际等待
//!
//! TTS/ASR 引擎通过环境变量启用，未启用时节点降级为走 default 端口继续。

use super::super::types::*;
use crate::EdgeState;
use tracing::{info, warn};

/// 默认 ASR 超时秒数
const DEFAULT_ASR_TIMEOUT_SECS: u32 = 10;

/// 执行 tts 节点：合成并播放语音
///
/// 读取配置：
/// - `text`：待合成文本（必填，支持 `{{var}}` 模板渲染）
/// - `voice`：发音人（可选，记录到日志，实际发音人由 TTS 模型决定）
/// - `speed`：语速（可选，0.5 ~ 2.0，默认 1.0，记录到日志）
///
/// 流程：从 `edge_state.voice_engine` 取 `TtsEngine` -> `synthesize` 得到 PCM i16 ->
/// 写入临时 WAV 文件 -> 调用 `media_relay.start_playback` 播放 -> 走 default 端口。
pub async fn execute_tts(
    node: &TopologyNode,
    context: &mut IvrExecutionContext,
    a_port: u16,
    edge_state: &EdgeState,
) -> NodeExecuteResult {
    let text = match node.config.get("text").and_then(|v| v.as_str()) {
        Some(s) if !s.is_empty() => context.render_template(s),
        _ => {
            warn!(
                call_id = %context.call_id,
                node_id = %node.id,
                "tts 节点未配置 text, 跳过"
            );
            return NodeExecuteResult::Continue {
                port: "default".to_string(),
            };
        }
    };
    let voice = node
        .config
        .get("voice")
        .and_then(|v| v.as_str())
        .unwrap_or("default");
    let speed = node
        .config
        .get("speed")
        .and_then(|v| v.as_f64())
        .unwrap_or(1.0);

    let voice_mgr = edge_state.voice_engine();
    let Some(tts_engine) = voice_mgr.as_ref().and_then(|m| m.tts.as_ref()) else {
        warn!(
            call_id = %context.call_id,
            node_id = %node.id,
            "TTS 引擎未启用 (VOS_RS_IVR_TTS_ENABLED), 跳过"
        );
        return NodeExecuteResult::Continue {
            port: "default".to_string(),
        };
    };

    info!(
        call_id = %context.call_id,
        node_id = %node.id,
        a_port,
        voice,
        speed,
        text_len = text.len(),
        "IVR tts 节点开始合成"
    );

    match tts_engine.synthesize(&text).await {
        Ok(result) => {
            info!(
                call_id = %context.call_id,
                node_id = %node.id,
                samples = result.samples.len(),
                sample_rate = result.sample_rate,
                "IVR tts 节点合成完成, 写入临时 WAV 并播放"
            );
            match write_pcm_to_temp_wav(&result.samples, result.sample_rate) {
                Ok(path) => {
                    let playback_mode = crate::media::relay::PlaybackMode::Exclusive;
                    match edge_state
                        .media_relay
                        .start_playback(
                            a_port,
                            std::path::PathBuf::from(&path),
                            playback_mode,
                            false,
                        )
                        .await
                    {
                        Ok(_) => NodeExecuteResult::Continue {
                            port: "default".to_string(),
                        },
                        Err(e) => {
                            warn!(
                                call_id = %context.call_id,
                                node_id = %node.id,
                                error = %e,
                                "tts 节点播放失败"
                            );
                            NodeExecuteResult::Error {
                                message: format!("TTS 播放失败: {e}"),
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!(
                        call_id = %context.call_id,
                        node_id = %node.id,
                        error = %e,
                        "tts 节点写入临时 WAV 失败"
                    );
                    NodeExecuteResult::Error {
                        message: format!("TTS 写入临时文件失败: {e}"),
                    }
                }
            }
        }
        Err(e) => {
            warn!(
                call_id = %context.call_id,
                node_id = %node.id,
                error = %e,
                "tts 节点合成失败"
            );
            NodeExecuteResult::Error {
                message: format!("TTS 合成失败: {e}"),
            }
        }
    }
}

/// 执行 asr 节点：等待语音识别输入
///
/// 读取配置：
/// - `timeout_secs`：超时秒数（默认 10）
/// - `max_speech`：最大语音时长（秒，可选，记录到日志，实际限制由媒体层负责）
///
/// 返回 [`NodeExecuteResult::WaitForAsr`]，由拓扑引擎调用 `wait_for_asr` 接入 ASR 引擎。
pub async fn execute_asr(
    node: &TopologyNode,
    context: &mut IvrExecutionContext,
) -> NodeExecuteResult {
    let timeout_secs = node
        .config
        .get("timeout_secs")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32)
        .unwrap_or(DEFAULT_ASR_TIMEOUT_SECS);
    let max_speech = node
        .config
        .get("max_speech")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32);

    info!(
        call_id = %context.call_id,
        node_id = %node.id,
        timeout_secs,
        ?max_speech,
        "IVR asr 节点返回 WaitForAsr, 由引擎接入 ASR"
    );
    NodeExecuteResult::WaitForAsr { timeout_secs }
}

/// 将 16-bit PCM 单声道样本写入临时 WAV 文件，返回文件路径。
///
/// 手动写入 44 字节 WAV 头 + PCM 数据，避免引入额外音频库依赖。
fn write_pcm_to_temp_wav(samples: &[i16], sample_rate: u32) -> Result<String, String> {
    use std::io::Write;
    let temp_dir = std::env::temp_dir();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let filename = format!("vos_tts_{nanos}.wav");
    let path = temp_dir.join(filename);

    let data_len = (samples.len() * 2) as u32;
    let byte_rate = sample_rate * 2; // mono * 16-bit
    let block_align: u16 = 2; // mono * 16-bit / 8

    let mut file = std::fs::File::create(&path).map_err(|e| format!("创建临时文件失败: {e}"))?;

    // RIFF header
    file.write_all(b"RIFF").map_err(|e| e.to_string())?;
    file.write_all(&(36 + data_len).to_le_bytes())
        .map_err(|e| e.to_string())?;
    file.write_all(b"WAVE").map_err(|e| e.to_string())?;
    // fmt chunk
    file.write_all(b"fmt ").map_err(|e| e.to_string())?;
    file.write_all(&16u32.to_le_bytes())
        .map_err(|e| e.to_string())?; // PCM fmt chunk size
    file.write_all(&1u16.to_le_bytes())
        .map_err(|e| e.to_string())?; // audio format = PCM
    file.write_all(&1u16.to_le_bytes())
        .map_err(|e| e.to_string())?; // mono
    file.write_all(&sample_rate.to_le_bytes())
        .map_err(|e| e.to_string())?;
    file.write_all(&byte_rate.to_le_bytes())
        .map_err(|e| e.to_string())?;
    file.write_all(&block_align.to_le_bytes())
        .map_err(|e| e.to_string())?;
    file.write_all(&16u16.to_le_bytes())
        .map_err(|e| e.to_string())?; // bits per sample
                                      // data chunk
    file.write_all(b"data").map_err(|e| e.to_string())?;
    file.write_all(&data_len.to_le_bytes())
        .map_err(|e| e.to_string())?;

    for sample in samples {
        file.write_all(&sample.to_le_bytes())
            .map_err(|e| e.to_string())?;
    }

    Ok(path.to_string_lossy().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_pcm_to_temp_wav_roundtrip_header() {
        // 极小 PCM 数据: 4 个采样
        let samples = [0i16, 1, -1, 32767];
        let path = write_pcm_to_temp_wav(&samples, 8000).expect("write wav");
        let bytes = std::fs::read(&path).expect("read wav");
        // 44 字节头 + 8 字节数据
        assert_eq!(bytes.len(), 44 + samples.len() * 2);
        // RIFF 标识
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
        assert_eq!(&bytes[12..16], b"fmt ");
        // data 标识在偏移 36
        assert_eq!(&bytes[36..40], b"data");
        // data_len = 8
        assert_eq!(u32::from_le_bytes(bytes[40..44].try_into().unwrap()), 8);
        let _ = std::fs::remove_file(&path);
    }
}
