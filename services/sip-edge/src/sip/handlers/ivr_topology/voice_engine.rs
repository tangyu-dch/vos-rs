//! IVR TTS/ASR 引擎封装 (基于 sherpa-rs)
//!
//! 提供 TTS 合成和 ASR 识别能力，支持离线运行。
//! 引擎惰性初始化，首次调用时加载 ONNX 模型。
//! 同步模型推理在 `tokio::task::spawn_blocking` 中执行，避免阻塞 async runtime。

#[cfg(feature = "enterprise-ai-voice")]
use sherpa_rs::sense_voice::{SenseVoiceConfig, SenseVoiceRecognizer};
#[cfg(feature = "enterprise-ai-voice")]
use sherpa_rs::tts::{VitsTts, VitsTtsConfig};
#[cfg(feature = "enterprise-ai-voice")]
use sherpa_rs::OnnxConfig;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{error, info, warn};

/// TTS 语速基数 (sherpa-rs speed 参数, 1.0 = 正常语速)
const TTS_DEFAULT_SPEED: f32 = 1.0;
/// TTS 默认说话人 ID (单说话人模型固定为 0)
const TTS_DEFAULT_SID: i32 = 0;
/// PCM f32 -> i16 满量程
const PCM_I16_MAX: f32 = 32767.0;
const PCM_I16_MIN: f32 = -32768.0;

/// TTS 引擎配置
#[derive(Debug, Clone)]
pub struct TtsConfig {
    /// VITS ONNX 模型路径 (model.onnx)
    pub model_path: PathBuf,
    /// tokens.txt 路径
    pub tokens_path: PathBuf,
    /// lexicon.txt 路径 (中文模型需要，英文模型可为 None)
    pub lexicon_path: Option<PathBuf>,
    /// dict 目录路径 (可选，部分中文模型需要)
    pub dict_dir: Option<PathBuf>,
    /// data 目录路径 (可选，部分模型需要)
    pub data_dir: Option<PathBuf>,
    /// 噪声比例 (控制韵律变化)
    pub noise_scale: f32,
    /// 噪声比例 W (控制音素时长变化)
    pub noise_scale_w: f32,
    /// 语速控制: 越大越慢 (1.0 = 正常)
    pub length_scale: f32,
    /// ONNX 推理线程数
    pub num_threads: i32,
}

impl Default for TtsConfig {
    fn default() -> Self {
        Self {
            model_path: PathBuf::new(),
            tokens_path: PathBuf::new(),
            lexicon_path: None,
            dict_dir: None,
            data_dir: None,
            noise_scale: 0.667,
            noise_scale_w: 0.8,
            length_scale: 1.0,
            num_threads: 2,
        }
    }
}

/// TTS 合成结果 (16-bit PCM 单声道)
#[derive(Debug, Clone)]
pub struct TtsResult {
    /// 16-bit PCM 样本 (interleaved mono)
    pub samples: Vec<i16>,
    /// 采样率 (Hz)
    pub sample_rate: u32,
}

/// ASR 引擎配置
#[derive(Debug, Clone)]
pub struct AsrConfig {
    /// SenseVoice ONNX 模型路径 (model.int8.onnx)
    pub model_path: PathBuf,
    /// tokens.txt 路径
    pub tokens_path: PathBuf,
    /// 识别语言: "auto" / "zh" / "en" / "ja" / "ko" / "yue"
    pub language: String,
    /// 是否启用逆文本归一化 (数字/日期等格式化)
    pub use_itn: bool,
    /// ONNX 推理线程数
    pub num_threads: i32,
}

impl Default for AsrConfig {
    fn default() -> Self {
        Self {
            model_path: PathBuf::new(),
            tokens_path: PathBuf::new(),
            language: "auto".to_string(),
            use_itn: true,
            num_threads: 2,
        }
    }
}

/// TTS 引擎单例 (惰性初始化)
///
/// 内部用 `tokio::sync::Mutex` 保护 `VitsTts` 实例。
/// 模型加载与合成均在 `spawn_blocking` 中执行，避免阻塞异步运行时。
#[cfg(feature = "enterprise-ai-voice")]
pub struct TtsEngine {
    config: TtsConfig,
    inner: Arc<Mutex<Option<VitsTts>>>,
}

#[cfg(not(feature = "enterprise-ai-voice"))]
pub struct TtsEngine {
    config: TtsConfig,
}

impl TtsEngine {
    pub fn new(config: TtsConfig) -> Self {
        #[cfg(feature = "enterprise-ai-voice")]
        {
            Self {
                config,
                inner: Arc::new(Mutex::new(None)),
            }
        }
        #[cfg(not(feature = "enterprise-ai-voice"))]
        {
            Self { config }
        }
    }

    #[cfg(feature = "enterprise-ai-voice")]
    async fn ensure_loaded(&self) -> Result<(), String> {
        let mut guard = self.inner.lock().await;
        if guard.is_some() {
            return Ok(());
        }
        self.validate_paths()?;

        let config = self.config.clone();
        let result = tokio::task::spawn_blocking(move || -> Result<VitsTts, String> {
            let vits_config = build_vits_config(&config);
            Ok(VitsTts::new(vits_config))
        })
        .await
        .map_err(|e| format!("TTS 加载任务执行失败: {e}"))?;

        match result {
            Ok(tts) => {
                *guard = Some(tts);
                info!("TTS 引擎初始化成功 (model={:?})", self.config.model_path);
                Ok(())
            }
            Err(e) => {
                error!("TTS 引擎初始化失败: {e}");
                Err(format!("TTS 引擎初始化失败: {e}"))
            }
        }
    }

    fn validate_paths(&self) -> Result<(), String> {
        if self.config.model_path.as_os_str().is_empty() {
            return Err("TTS 模型路径未配置".to_string());
        }
        if !self.config.model_path.exists() {
            return Err(format!("TTS 模型文件不存在: {:?}", self.config.model_path));
        }
        if !self.config.tokens_path.exists() {
            return Err(format!(
                "TTS tokens 文件不存在: {:?}",
                self.config.tokens_path
            ));
        }
        Ok(())
    }

    pub async fn synthesize(&self, text: &str) -> Result<TtsResult, String> {
        if text.trim().is_empty() {
            return Err("TTS 合成文本为空".to_string());
        }
        #[cfg(feature = "enterprise-ai-voice")]
        {
            self.ensure_loaded().await?;

            let inner = self.inner.clone();
            let text_owned = text.to_string();
            let result = tokio::task::spawn_blocking(move || -> Result<TtsResult, String> {
                let mut guard = inner.blocking_lock();
                let tts = guard
                    .as_mut()
                    .ok_or_else(|| "TTS 引擎未初始化".to_string())?;
                let audio = tts
                    .create(&text_owned, TTS_DEFAULT_SID, TTS_DEFAULT_SPEED)
                    .map_err(|e| format!("TTS 合成失败: {e}"))?;
                let samples = audio
                    .samples
                    .iter()
                    .map(|s| (s * PCM_I16_MAX).clamp(PCM_I16_MIN, PCM_I16_MAX) as i16)
                    .collect();
                Ok(TtsResult {
                    samples,
                    sample_rate: audio.sample_rate,
                })
            })
            .await
            .map_err(|e| format!("TTS 任务执行失败: {e}"))?;

            result
        }
        #[cfg(not(feature = "enterprise-ai-voice"))]
        {
            Err("AI 语音引擎未开启 (企业版 feature: enterprise-ai-voice)".to_string())
        }
    }
}

#[cfg(feature = "enterprise-ai-voice")]
pub struct AsrEngine {
    config: AsrConfig,
    inner: Arc<Mutex<Option<SenseVoiceRecognizer>>>,
}

#[cfg(not(feature = "enterprise-ai-voice"))]
pub struct AsrEngine {
    config: AsrConfig,
}

impl AsrEngine {
    pub fn new(config: AsrConfig) -> Self {
        #[cfg(feature = "enterprise-ai-voice")]
        {
            Self {
                config,
                inner: Arc::new(Mutex::new(None)),
            }
        }
        #[cfg(not(feature = "enterprise-ai-voice"))]
        {
            Self { config }
        }
    }

    #[cfg(feature = "enterprise-ai-voice")]
    async fn ensure_loaded(&self) -> Result<(), String> {
        let mut guard = self.inner.lock().await;
        if guard.is_some() {
            return Ok(());
        }
        self.validate_paths()?;

        let config = self.config.clone();
        let result =
            tokio::task::spawn_blocking(move || -> Result<SenseVoiceRecognizer, String> {
                let sense_config = build_sense_config(&config);
                SenseVoiceRecognizer::new(sense_config)
                    .map_err(|e| format!("SenseVoice 初始化失败: {e}"))
            })
            .await
            .map_err(|e| format!("ASR 加载任务执行失败: {e}"))?;

        match result {
            Ok(recognizer) => {
                *guard = Some(recognizer);
                info!("ASR 引擎初始化成功 (model={:?})", self.config.model_path);
                Ok(())
            }
            Err(e) => {
                error!("ASR 引擎初始化失败: {e}");
                Err(e)
            }
        }
    }

    fn validate_paths(&self) -> Result<(), String> {
        if self.config.model_path.as_os_str().is_empty() {
            return Err("ASR 模型路径未配置".to_string());
        }
        if !self.config.model_path.exists() {
            return Err(format!("ASR 模型文件不存在: {:?}", self.config.model_path));
        }
        if !self.config.tokens_path.exists() {
            return Err(format!(
                "ASR tokens 文件不存在: {:?}",
                self.config.tokens_path
            ));
        }
        Ok(())
    }

    pub async fn recognize(&self, _samples: &[i16], _sample_rate: u32) -> Result<String, String> {
        #[cfg(feature = "enterprise-ai-voice")]
        {
            self.ensure_loaded().await?;
            let inner = self.inner.clone();
            let samples_vec = _samples.to_vec();
            let result = tokio::task::spawn_blocking(move || -> Result<String, String> {
                let mut guard = inner.blocking_lock();
                let asr = guard
                    .as_mut()
                    .ok_or_else(|| "ASR 引擎未初始化".to_string())?;
                let float_samples: Vec<f32> = samples_vec.iter().map(|&s| s as f32 / 32768.0).collect();
                let result = asr.transcribe(_sample_rate, &float_samples);
                Ok(result.text)
            })
            .await
            .map_err(|e| format!("ASR 任务执行失败: {e}"))?;
            result
        }
        #[cfg(not(feature = "enterprise-ai-voice"))]
        {
            Err("AI 语音引擎未开启 (企业版 feature: enterprise-ai-voice)".to_string())
        }
    }
}

#[cfg(feature = "enterprise-ai-voice")]
/// 从 [`TtsConfig`] 构建 sherpa-rs [`VitsTtsConfig`]
fn build_vits_config(config: &TtsConfig) -> VitsTtsConfig {
    VitsTtsConfig {
        model: config.model_path.to_string_lossy().to_string(),
        tokens: config.tokens_path.to_string_lossy().to_string(),
        lexicon: config
            .lexicon_path
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default(),
        dict_dir: config
            .dict_dir
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default(),
        data_dir: config
            .data_dir
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default(),
        noise_scale: config.noise_scale,
        noise_scale_w: config.noise_scale_w,
        length_scale: config.length_scale,
        silence_scale: 0.0,
        onnx_config: OnnxConfig {
            provider: sherpa_rs::get_default_provider(),
            debug: false,
            num_threads: config.num_threads,
        },
        tts_config: Default::default(),
    }
}

#[cfg(feature = "enterprise-ai-voice")]
/// 从 [`AsrConfig`] 构建 sherpa-rs [`SenseVoiceConfig`]
fn build_sense_config(config: &AsrConfig) -> SenseVoiceConfig {
    SenseVoiceConfig {
        model: config.model_path.to_string_lossy().to_string(),
        tokens: config.tokens_path.to_string_lossy().to_string(),
        language: config.language.clone(),
        use_itn: config.use_itn,
        provider: Some(sherpa_rs::get_default_provider()),
        num_threads: Some(config.num_threads),
        debug: false,
    }
}

/// 全局 TTS/ASR 引擎管理器
///
/// 由环境变量驱动配置，IVR 执行器通过 `Arc<VoiceEngineManager>` 共享实例。
pub struct VoiceEngineManager {
    /// TTS 引擎 (未启用时为 None)
    pub tts: Option<Arc<TtsEngine>>,
    /// ASR 引擎 (未启用时为 None)
    pub asr: Option<Arc<AsrEngine>>,
}

impl VoiceEngineManager {
    /// 从环境变量加载配置并创建引擎实例 (不立即加载模型)
    ///
    /// # 环境变量
    /// - `VOS_RS_IVR_TTS_ENABLED` - 是否启用 TTS ("true" 启用)
    /// - `VOS_RS_IVR_TTS_MODEL_PATH` - VITS 模型路径
    /// - `VOS_RS_IVR_TTS_TOKENS_PATH` - tokens.txt 路径
    /// - `VOS_RS_IVR_TTS_LEXICON_PATH` - lexicon.txt 路径 (可选)
    /// - `VOS_RS_IVR_ASR_ENABLED` - 是否启用 ASR ("true" 启用)
    /// - `VOS_RS_IVR_ASR_MODEL_PATH` - SenseVoice 模型路径
    /// - `VOS_RS_IVR_ASR_TOKENS_PATH` - tokens.txt 路径
    /// - `VOS_RS_IVR_ASR_LANGUAGE` - 识别语言 (默认 "auto")
    pub fn from_env() -> Self {
        let tts = if env_flag_enabled("VOS_RS_IVR_TTS_ENABLED") {
            let config = TtsConfig {
                model_path: PathBuf::from(env_or_empty("VOS_RS_IVR_TTS_MODEL_PATH")),
                tokens_path: PathBuf::from(env_or_empty("VOS_RS_IVR_TTS_TOKENS_PATH")),
                lexicon_path: env_path_opt("VOS_RS_IVR_TTS_LEXICON_PATH"),
                dict_dir: env_path_opt("VOS_RS_IVR_TTS_DICT_DIR"),
                data_dir: env_path_opt("VOS_RS_IVR_TTS_DATA_DIR"),
                ..Default::default()
            };
            if config.model_path.as_os_str().is_empty() {
                warn!("VOS_RS_IVR_TTS_ENABLED=true 但未配置 TTS_MODEL_PATH，TTS 不可用");
                None
            } else {
                Some(Arc::new(TtsEngine::new(config)))
            }
        } else {
            None
        };

        let asr = if env_flag_enabled("VOS_RS_IVR_ASR_ENABLED") {
            let language =
                std::env::var("VOS_RS_IVR_ASR_LANGUAGE").unwrap_or_else(|_| "auto".to_string());
            let config = AsrConfig {
                model_path: PathBuf::from(env_or_empty("VOS_RS_IVR_ASR_MODEL_PATH")),
                tokens_path: PathBuf::from(env_or_empty("VOS_RS_IVR_ASR_TOKENS_PATH")),
                language,
                ..Default::default()
            };
            if config.model_path.as_os_str().is_empty() {
                warn!("VOS_RS_IVR_ASR_ENABLED=true 但未配置 ASR_MODEL_PATH，ASR 不可用");
                None
            } else {
                Some(Arc::new(AsrEngine::new(config)))
            }
        } else {
            None
        };

        Self { tts, asr }
    }
}

/// 读取布尔型环境变量 (值为 "true" 时返回 true)
fn env_flag_enabled(key: &str) -> bool {
    std::env::var(key).map(|v| v == "true").unwrap_or(false)
}

/// 读取字符串环境变量，不存在时返回空字符串
fn env_or_empty(key: &str) -> String {
    std::env::var(key).unwrap_or_default()
}

/// 读取可选路径型环境变量
fn env_path_opt(key: &str) -> Option<PathBuf> {
    std::env::var(key).ok().map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tts_config_default() {
        let config = TtsConfig::default();
        assert_eq!(config.noise_scale, 0.667);
        assert_eq!(config.noise_scale_w, 0.8);
        assert_eq!(config.length_scale, 1.0);
        assert_eq!(config.num_threads, 2);
        assert!(config.model_path.as_os_str().is_empty());
    }

    #[test]
    fn test_asr_config_default() {
        let config = AsrConfig::default();
        assert_eq!(config.language, "auto");
        assert!(config.use_itn);
        assert_eq!(config.num_threads, 2);
    }

    #[test]
    fn test_voice_engine_manager_from_env_disabled() {
        // 默认未启用
        let manager = VoiceEngineManager::from_env();
        assert!(manager.tts.is_none());
        assert!(manager.asr.is_none());
    }

    #[test]
    #[cfg(feature = "enterprise-ai-voice")]
    fn test_build_vits_config_uses_paths() {
        let config = TtsConfig {
            model_path: PathBuf::from("/tmp/model.onnx"),
            tokens_path: PathBuf::from("/tmp/tokens.txt"),
            lexicon_path: Some(PathBuf::from("/tmp/lexicon.txt")),
            ..Default::default()
        };
        let vits = build_vits_config(&config);
        assert_eq!(vits.model, "/tmp/model.onnx");
        assert_eq!(vits.tokens, "/tmp/tokens.txt");
        assert_eq!(vits.lexicon, "/tmp/lexicon.txt");
    }

    #[test]
    #[cfg(feature = "enterprise-ai-voice")]
    fn test_build_sense_config_uses_paths() {
        let config = AsrConfig {
            model_path: PathBuf::from("/tmp/asr.onnx"),
            tokens_path: PathBuf::from("/tmp/tokens.txt"),
            language: "zh".to_string(),
            ..Default::default()
        };
        let sense = build_sense_config(&config);
        assert_eq!(sense.model, "/tmp/asr.onnx");
        assert_eq!(sense.tokens, "/tmp/tokens.txt");
        assert_eq!(sense.language, "zh");
    }
}
