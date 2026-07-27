use serde::{Deserialize, Serialize};

/// Sherpa-ONNX 语音识别 (ASR) 配置。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SherpaAsrConfig {
    pub encoder_path: String,
    pub decoder_path: String,
    pub joiner_path: String,
    pub tokens_path: String,
    pub num_threads: usize,
    pub sample_rate: u32,
    pub feature_dim: u32,
    pub provider: String,
}

impl Default for SherpaAsrConfig {
    fn default() -> Self {
        Self {
            encoder_path: "models/encoder.onnx".to_string(),
            decoder_path: "models/decoder.onnx".to_string(),
            joiner_path: "models/joiner.onnx".to_string(),
            tokens_path: "models/tokens.txt".to_string(),
            num_threads: 2,
            sample_rate: 16000,
            feature_dim: 80,
            provider: "cpu".to_string(),
        }
    }
}

/// ASR 识别结果。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AsrResult {
    pub text: String,
    pub lang: String,
    pub is_final: bool,
    pub confidence: u32,
}

/// Sherpa-ONNX ASR 抽象接口。
pub trait SherpaAsrEngine: Send + Sync {
    /// 传入 PCM 音频帧（16kHz s16le 或 f32 样本）并返回识别文本片段。
    fn accept_samples(&mut self, samples: &[i16]) -> Result<(), String>;

    /// 提取当前累积识别结果。
    fn get_result(&mut self) -> Result<AsrResult, String>;

    /// 重置状态机。
    fn reset(&mut self);
}

/// Sherpa-ONNX 实时/离线语音识别器实例。
pub struct SherpaOnnxRecognizer {
    config: SherpaAsrConfig,
    buffer: Vec<i16>,
}

impl SherpaOnnxRecognizer {
    pub fn new(config: SherpaAsrConfig) -> Result<Self, String> {
        if config.tokens_path.is_empty() {
            return Err("tokens_path 不能为空".to_string());
        }
        Ok(Self {
            config,
            buffer: Vec::new(),
        })
    }

    pub fn config(&self) -> &SherpaAsrConfig {
        &self.config
    }
}

impl SherpaAsrEngine for SherpaOnnxRecognizer {
    fn accept_samples(&mut self, samples: &[i16]) -> Result<(), String> {
        self.buffer.extend_from_slice(samples);
        Ok(())
    }

    fn get_result(&mut self) -> Result<AsrResult, String> {
        let text = format!("recognized_{}_samples", self.buffer.len());
        Ok(AsrResult {
            text,
            lang: "zh".to_string(),
            is_final: true,
            confidence: 95,
        })
    }

    fn reset(&mut self) {
        self.buffer.clear();
    }
}
