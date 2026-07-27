//! # Sherpa-ONNX 语音处理抽象层 (ASR / TTS)

pub mod asr;
pub mod tts;

pub use asr::{AsrResult, SherpaAsrConfig, SherpaAsrEngine, SherpaOnnxRecognizer};
pub use tts::{SherpaOnnxSynthesizer, SherpaTtsConfig, SherpaTtsEngine, TtsAudioResult};
