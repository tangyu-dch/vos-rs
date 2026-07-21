//! # 硬件加速编解码抽象层 (Hardware Accelerated Audio/Video Codec Layer)
//! 
//! 本模块定义了用于 DSP / GPU / VA-API / Intel QSV 编解码器调用的硬件加速 Trait 接口，
//! 用于高并发场景下的录音重采样、音频 Opus ↔ G.711 转码与视频硬件加速。

use std::error::Error;
use std::fmt;

#[derive(Debug)]
pub enum HwAccelError {
    DeviceNotFound(String),
    CodecUnsupported(String),
    ExecutionFailed(String),
}

impl fmt::Display for HwAccelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DeviceNotFound(dev) => write!(f, "Hardware device not found: {dev}"),
            Self::CodecUnsupported(codec) => write!(f, "Codec not supported by HW accelerator: {codec}"),
            Self::ExecutionFailed(msg) => write!(f, "HW acceleration execution failed: {msg}"),
        }
    }
}

impl Error for HwAccelError {}

/// 硬件加速编码器 Trait 抽象
pub trait HardwareAudioEncoder: Send + Sync {
    /// 编码器名称（如 "nvenc-cuda", "intel-qsv", "vaapi"）
    fn name(&self) -> &str;

    /// 处理 PCM 音频块，进行硬件加速重采样与转码
    fn encode_pcm(&self, input_pcm: &[i16], sample_rate: u32, channels: u16) -> Result<Vec<u8>, HwAccelError>;
}

/// 默认 CPU 软件 Fallback 编码器实现
#[derive(Debug, Default)]
pub struct SoftwareFallbackEncoder;

impl HardwareAudioEncoder for SoftwareFallbackEncoder {
    fn name(&self) -> &str {
        "software-fallback-cpu"
    }

    fn encode_pcm(&self, input_pcm: &[i16], _sample_rate: u32, _channels: u16) -> Result<Vec<u8>, HwAccelError> {
        let mut bytes = Vec::with_capacity(input_pcm.len() * 2);
        for &sample in input_pcm {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_software_fallback_encoder() {
        let encoder = SoftwareFallbackEncoder;
        assert_eq!(encoder.name(), "software-fallback-cpu");
        let pcm = vec![100, -200, 300];
        let encoded = encoder.encode_pcm(&pcm, 8000, 1).unwrap();
        assert_eq!(encoded.len(), 6);
    }

    #[test]
    fn test_hw_accel_error_display() {
        let err1 = HwAccelError::DeviceNotFound("cuda:0".into());
        let err2 = HwAccelError::CodecUnsupported("h265".into());
        let err3 = HwAccelError::ExecutionFailed("out of memory".into());

        assert!(err1.to_string().contains("cuda:0"));
        assert!(err2.to_string().contains("h265"));
        assert!(err3.to_string().contains("out of memory"));
    }
}
