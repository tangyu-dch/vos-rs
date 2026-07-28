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
