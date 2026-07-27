use serde::{Deserialize, Serialize};

/// Sherpa-ONNX 语音合成 (TTS) 配置。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SherpaTtsConfig {
    pub model_path: String,
    pub acoustic_model_path: String,
    pub vocoder_path: String,
    pub lexicon_path: String,
    pub tokens_path: String,
    pub num_threads: usize,
    pub sample_rate: u32,
    pub speaker_id: i32,
    pub speed: f32,
}

impl Default for SherpaTtsConfig {
    fn default() -> Self {
        Self {
            model_path: "models/tts_vits.onnx".to_string(),
            acoustic_model_path: String::new(),
            vocoder_path: String::new(),
            lexicon_path: "models/lexicon.txt".to_string(),
            tokens_path: "models/tokens.txt".to_string(),
            num_threads: 2,
            sample_rate: 16000,
            speaker_id: 0,
            speed: 1.0,
        }
    }
}

/// TTS 合成生成的音频片段。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TtsAudioResult {
    pub samples: Vec<i16>,
    pub sample_rate: u32,
}

/// Sherpa-ONNX TTS 抽象接口。
pub trait SherpaTtsEngine: Send + Sync {
    /// 将指定文本合成 raw PCM (s16le 16kHz) 样本。
    fn synthesize(&self, text: &str, speaker_id: Option<i32>) -> Result<TtsAudioResult, String>;
}

/// Sherpa-ONNX 语音合成器实例。
pub struct SherpaOnnxSynthesizer {
    config: SherpaTtsConfig,
}

impl SherpaOnnxSynthesizer {
    pub fn new(config: SherpaTtsConfig) -> Result<Self, String> {
        if config.model_path.is_empty() {
            return Err("model_path 不能为空".to_string());
        }
        Ok(Self { config })
    }

    pub fn config(&self) -> &SherpaTtsConfig {
        &self.config
    }
}

impl SherpaTtsEngine for SherpaOnnxSynthesizer {
    fn synthesize(&self, text: &str, speaker_id: Option<i32>) -> Result<TtsAudioResult, String> {
        if text.trim().is_empty() {
            return Err("文本不能为空".to_string());
        }
        let _spk = speaker_id.unwrap_or(self.config.speaker_id);
        // 生成对应 sample_rate 的 PCM 音频模拟样本帧
        let num_samples = (self.config.sample_rate as usize) / 5; // 200ms audio
        let samples = vec![0_i16; num_samples];

        Ok(TtsAudioResult {
            samples,
            sample_rate: self.config.sample_rate,
        })
    }
}
