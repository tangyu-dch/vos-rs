//! G.711 PCMA (A-law) and PCMU (u-law) transcoding conversions.

pub fn linear_to_ulaw(mut pcm: i16) -> u8 {
    let sign = if pcm < 0 {
        pcm = -pcm;
        0x80
    } else {
        0
    };
    if pcm > 32635 {
        pcm = 32635;
    }
    let pcm = pcm + 0x84;
    let mut exponent = 7;
    let mut mask = 0x4000;
    while (pcm & mask) == 0 && exponent > 0 {
        exponent -= 1;
        mask >>= 1;
    }
    let mantissa = (pcm >> (exponent + 3)) & 0x0f;
    let ulaw = (sign | (exponent << 4) | mantissa) as u8;
    !ulaw
}

pub fn linear_to_alaw(mut pcm: i16) -> u8 {
    let sign = if pcm < 0 {
        pcm = -pcm;
        0
    } else {
        0x80
    };
    if pcm > 32635 {
        pcm = 32635;
    }
    let mut exponent = 7;
    let mut mask = 0x4000;
    while (pcm & mask) == 0 && exponent > 0 {
        exponent -= 1;
        mask >>= 1;
    }
    let mantissa = if exponent == 0 {
        (pcm >> 4) & 0x0f
    } else {
        (pcm >> (exponent + 3)) & 0x0f
    };
    let alaw = (sign | (exponent << 4) | mantissa) as u8;
    alaw ^ 0x55
}

pub fn transcode_pcma_to_pcmu(payload: &[u8]) -> Vec<u8> {
    payload
        .iter()
        .map(|&a| {
            let pcm = crate::media::recording::decode_pcma(a);
            linear_to_ulaw(pcm)
        })
        .collect()
}

pub fn transcode_pcmu_to_pcma(payload: &[u8]) -> Vec<u8> {
    payload
        .iter()
        .map(|&u| {
            let pcm = crate::media::recording::decode_pcmu(u);
            linear_to_alaw(pcm)
        })
        .collect()
}

/// 录音后处理转码格式。
///
/// `Wav` — 不做任何处理（默认）
/// `Opus` — 通过 ffmpeg 转码为 Opus/WebM，适合存储与流媒体
/// `Amr` — 通过 ffmpeg 转码为 AMR-NB，适合移动端存档
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordingFormat {
    Wav,
    Opus,
    Amr,
}

impl RecordingFormat {
    /// 从配置字符串解析格式，大小写不敏感。未知值 fallback 到 `Wav`。
    pub fn from_str(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "opus" => Self::Opus,
            "amr" => Self::Amr,
            _ => Self::Wav,
        }
    }

    /// 目标文件扩展名
    pub fn extension(&self) -> &'static str {
        match self {
            Self::Wav => "wav",
            Self::Opus => "opus",
            Self::Amr => "amr",
        }
    }
}

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
        let pcma = vec![0xd5, 0x55, 0x50];
        let pcmu = transcode_pcma_to_pcmu(&pcma);
        assert_eq!(pcmu.len(), 3);

        let pcmu_back = vec![0xff, 0x00, 0x7f];
        let pcma_back = transcode_pcmu_to_pcma(&pcmu_back);
        assert_eq!(pcma_back.len(), 3);
    }

    #[test]
    fn test_recording_format_from_str() {
        assert_eq!(RecordingFormat::from_str("wav"), RecordingFormat::Wav);
        assert_eq!(RecordingFormat::from_str("WAV"), RecordingFormat::Wav);
        assert_eq!(RecordingFormat::from_str("opus"), RecordingFormat::Opus);
        assert_eq!(RecordingFormat::from_str("Opus"), RecordingFormat::Opus);
        assert_eq!(RecordingFormat::from_str("amr"), RecordingFormat::Amr);
        assert_eq!(RecordingFormat::from_str("unknown"), RecordingFormat::Wav);
    }

    #[test]
    fn test_recording_format_extension() {
        assert_eq!(RecordingFormat::Wav.extension(), "wav");
        assert_eq!(RecordingFormat::Opus.extension(), "opus");
        assert_eq!(RecordingFormat::Amr.extension(), "amr");
    }
}
