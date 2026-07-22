//! G.711 PCMA (A-law) and PCMU (u-law) transcoding conversions.

pub use media_core::g711::*;
pub use media_core::recording::RecordingFormat;

/// 通话结束后异步转码录音文件。
///
/// 若 `format` 为 `Wav` 则立即返回（无转码）。
/// 否则以 `tokio::spawn` 在后台启动 ffmpeg 子进程，完成后删除原始 WAV 文件。
///
/// # 参数
/// - `wav_path`  — 原始 WAV 录音路径
/// - `format`    — 目标格式
///
/// # ffmpeg 命令示例
/// ```text
/// ffmpeg -y -i input.wav -c:a libopus output.opus
/// ffmpeg -y -i input.wav -ar 8000 -ab 12.2k output.amr
/// ```
#[allow(dead_code)]
pub fn transcode_recording_async(wav_path: std::path::PathBuf, format: RecordingFormat) {
    if format == RecordingFormat::Wav {
        return; // 无需转码
    }

    tokio::spawn(async move {
        let Some(stem) = wav_path.file_stem().and_then(|s| s.to_str()) else {
            tracing::warn!(path = ?wav_path, "transcode: cannot determine file stem");
            return;
        };
        let parent = wav_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        let out_path = parent.join(format!("{}.{}", stem, format.extension()));

        // 构建 ffmpeg 参数
        let mut cmd = tokio::process::Command::new("ffmpeg");
        cmd.args(["-y", "-i"])
            .arg(&wav_path)
            .args(match format {
                RecordingFormat::Opus => vec!["-c:a", "libopus", "-b:a", "32k"],
                RecordingFormat::Amr => vec!["-ar", "8000", "-ab", "12200"],
                RecordingFormat::Wav => unreachable!(),
            })
            .arg(&out_path)
            // 抑制 ffmpeg 控制台输出（日志已由 tracing 负责）
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());

        match cmd.status().await {
            Ok(status) if status.success() => {
                tracing::info!(
                    src = ?wav_path,
                    dst = ?out_path,
                    "recording transcoded successfully"
                );
                // 删除原始 WAV（节省磁盘）
                let _ = tokio::fs::remove_file(&wav_path).await;
            }
            Ok(status) => {
                tracing::warn!(
                    src = ?wav_path,
                    dst = ?out_path,
                    exit_code = ?status.code(),
                    "ffmpeg transcoding failed, keeping original WAV"
                );
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "ffmpeg not available or spawn failed, keeping original WAV"
                );
            }
        }
    });
}

/// 转码录音文件并在完成后上传到统一存储后端（OSS/本地双写）。
pub fn transcode_and_upload_recording_async(
    wav_path: std::path::PathBuf,
    format: RecordingFormat,
    call_id: String,
    tokio_handle: Option<tokio::runtime::Handle>,
    storage: Option<std::sync::Arc<dyn storage_core::StorageBackend>>,
) {
    let Some(handle) = tokio_handle else {
        tracing::warn!("Tokio 句柄不可用，跳过录音上传同步");
        return;
    };
    handle.spawn(async move {
        let final_path = if format != RecordingFormat::Wav {
            let Some(stem) = wav_path.file_stem().and_then(|s| s.to_str()) else {
                tracing::warn!(path = ?wav_path, "transcode: cannot determine file stem");
                return;
            };
            let parent = wav_path
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| std::path::PathBuf::from("."));
            let out_path = parent.join(format!("{}.{}", stem, format.extension()));

            let mut cmd = tokio::process::Command::new("ffmpeg");
            cmd.args(["-y", "-i"])
                .arg(&wav_path)
                .args(match format {
                    RecordingFormat::Opus => vec!["-c:a", "libopus", "-b:a", "32k"],
                    RecordingFormat::Amr => vec!["-ar", "8000", "-ab", "12200"],
                    RecordingFormat::Wav => unreachable!(),
                })
                .arg(&out_path)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());

            match cmd.status().await {
                Ok(status) if status.success() => {
                    tracing::info!(
                        src = ?wav_path,
                        dst = ?out_path,
                        "recording transcoded successfully"
                    );
                    let _ = tokio::fs::remove_file(&wav_path).await;
                    out_path
                }
                Ok(status) => {
                    tracing::warn!(
                        src = ?wav_path,
                        dst = ?out_path,
                        code = ?status.code(),
                        "ffmpeg failed, fallback to raw wav"
                    );
                    wav_path
                }
                Err(error) => {
                    tracing::warn!(
                        src = ?wav_path,
                        dst = ?out_path,
                        %error,
                        "failed to execute ffmpeg, fallback to raw wav"
                    );
                    wav_path
                }
            }
        } else {
            wav_path
        };

        if let Some(storage) = storage {
            let extension = if final_path.extension().is_some_and(|e| e == "opus") {
                "opus"
            } else if final_path.extension().is_some_and(|e| e == "amr") {
                "amr"
            } else {
                "wav"
            };
            let key = recording_storage_key(&final_path, &call_id, extension);
            match tokio::fs::read(&final_path).await {
                Ok(data) => {
                    let content_type = match extension {
                        "wav" => Some("audio/wav"),
                        "opus" => Some("audio/ogg"),
                        "amr" => Some("audio/amr"),
                        _ => None,
                    };
                    match storage
                        .put(&key, axum::body::Bytes::from(data), content_type)
                        .await
                    {
                        Ok(()) => {
                            tracing::info!(key, "录音同步完成");
                        }
                        Err(error) => {
                            tracing::error!(%error, key, "录音上传存储后端失败");
                        }
                    }
                }
                Err(error) => {
                    tracing::error!(%error, path = ?final_path, "读取录音文件失败，无法同步");
                }
            }
        }
    });
}

fn recording_storage_key(final_path: &std::path::Path, call_id: &str, extension: &str) -> String {
    final_path
        .file_name()
        .and_then(|value| value.to_str())
        .map(str::to_owned)
        .unwrap_or_else(|| format!("rec-{call_id}.{extension}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_g711_roundtrip() {
        // Test a few sample values to verify roundtrip mapping behaves correctly
        for sample in [0, 100, -100, 1000, -1000, 5000, -5000] {
            let u = linear_to_ulaw(sample);
            let decoded_u = crate::media::recording::decode_pcmu(u);
            // G.711 compression is lossy, so check proximity
            assert!(
                (decoded_u - sample).abs() < 250,
                "pcm: {}, decoded: {}",
                sample,
                decoded_u
            );

            let a = linear_to_alaw(sample);
            let decoded_a = crate::media::recording::decode_pcma(a);
            assert!(
                (decoded_a - sample).abs() < 250,
                "pcm: {}, decoded: {}",
                sample,
                decoded_a
            );
        }
    }

    #[test]
    fn test_transcode_payloads() {
        let mut pcmu = vec![0xd5, 0x55, 0x50];
        transcode_pcma_to_pcmu_inplace(&mut pcmu);
        assert_eq!(pcmu.len(), 3);

        let mut pcma_back = vec![0xff, 0x00, 0x7f];
        transcode_pcmu_to_pcma_inplace(&mut pcma_back);
        assert_eq!(pcma_back.len(), 3);
    }

    #[test]
    fn recording_storage_keys_preserve_segment_file_names() {
        let first = recording_storage_key(
            std::path::Path::new("/recordings/call-123.wav"),
            "call-123",
            "wav",
        );
        let second = recording_storage_key(
            std::path::Path::new("/recordings/call-123-part-0001.wav"),
            "call-123",
            "wav",
        );

        assert_eq!(first, "call-123.wav");
        assert_eq!(second, "call-123-part-0001.wav");
        assert_ne!(first, second);
    }
}
