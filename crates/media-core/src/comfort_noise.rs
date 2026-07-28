//! 舒适噪音（CN）生成、DTX 不连续传输与静音抑制。
//!
//! 设计目标：
//! 1. 当 VAD 检测到静音时，进入 DTX 状态：不再转发原始 RTP，节省带宽
//! 2. 按 CN 周期（默认每 50ms 一帧）生成舒适噪音帧注入到出向 RTP 流
//!    以避免对端检测到完全静音导致挂机或硬件噪声门问题
//! 3. CN 帧的能量基于近期背景噪声估计（长时能量均值）动态调整
//! 4. 在静音→语音切换时立即恢复原始 RTP 转发，确保前导音不丢失

use rtp_core::AudioCodec;

/// DTX 决策结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DtxDecision {
    /// 语音活动，按原样转发 RTP 包。
    Forward,
    /// 静音抑制：丢弃当前包，由 CN 生成器决定何时注入 CN 帧。
    Suppress,
}

/// 舒适噪音生成器配置。
#[derive(Debug, Clone)]
pub struct ComfortNoiseConfig {
    /// CN 帧间隔（毫秒）。G.711 默认 50ms（每 6 个 20ms RTP 包发一个 CN 包）。
    pub cn_frame_interval_ms: u32,
    /// CN 帧大小（samples，对 G.711 即字节数）。
    pub cn_frame_samples: usize,
    /// 静音判定阈值（dB），低于此能量视为背景噪声。
    pub silence_threshold_db: f32,
    /// 进入静音抑制前需要连续观察到的静音帧数。
    pub hangover_frames: usize,
    /// 从静音回到语音的过渡帧数（避免 VAD 抖动）。
    pub speech_recovery_frames: usize,
}

impl Default for ComfortNoiseConfig {
    fn default() -> Self {
        Self {
            cn_frame_interval_ms: 50,
            cn_frame_samples: 160, // 20ms @ 8kHz
            silence_threshold_db: -45.0,
            hangover_frames: 4,
            speech_recovery_frames: 1,
        }
    }
}

/// 背景噪声能量估计器（长时平均）。
///
/// 使用一阶 IIR 低通滤波跟踪背景噪声能量，
/// 仅在静音期更新估计值，避免被语音活动污染。
#[derive(Debug, Clone)]
pub struct BackgroundNoiseEstimator {
    /// 当前噪声 RMS 估计（线性值，非 dB）。
    noise_rms: f64,
    /// 平滑因子（0~1），越大越平滑。
    smoothing: f64,
    /// 是否已初始化。
    initialized: bool,
}

impl BackgroundNoiseEstimator {
    /// 创建估计器，默认平滑因子 0.95。
    pub fn new() -> Self {
        Self::with_smoothing(0.95)
    }

    /// 使用指定平滑因子创建估计器。
    pub fn with_smoothing(smoothing: f64) -> Self {
        Self {
            noise_rms: 0.0,
            smoothing: smoothing.clamp(0.0, 0.99),
            initialized: false,
        }
    }

    /// 用新的静音帧能量更新噪声估计。
    pub fn observe_silence(&mut self, rms: f64) {
        if !self.initialized {
            self.noise_rms = rms;
            self.initialized = true;
        } else {
            // y[n] = α * y[n-1] + (1-α) * x[n]
            self.noise_rms = self.smoothing * self.noise_rms + (1.0 - self.smoothing) * rms;
        }
    }

    /// 返回当前噪声 RMS（线性值）。
    pub fn noise_rms(&self) -> f64 {
        self.noise_rms
    }

    /// 返回当前噪声能量（dB）。
    pub fn noise_db(&self) -> f64 {
        if self.noise_rms > 0.0 {
            20.0 * (self.noise_rms / 32_768.0).log10()
        } else {
            -96.0
        }
    }

    /// 重置估计器。
    pub fn reset(&mut self) {
        self.noise_rms = 0.0;
        self.initialized = false;
    }
}

impl Default for BackgroundNoiseEstimator {
    fn default() -> Self {
        Self::new()
    }
}

/// DTX 状态机：决定当前包是转发还是抑制，以及何时生成 CN 帧。
pub struct DtxController {
    config: ComfortNoiseConfig,
    bg_noise: BackgroundNoiseEstimator,
    /// 连续静音帧计数（用于 hangover 判定）。
    silence_frame_count: usize,
    /// 连续语音帧计数（用于过渡判定）。
    speech_frame_count: usize,
    /// 是否已进入 DTX 抑制状态。
    in_dtx: bool,
    /// 距离上次注入 CN 帧的累计毫秒数。
    cn_accumulated_ms: u32,
    /// 当前 CN 帧序号（用于 RTP 头生成）。
    cn_sequence: u16,
    /// 当前 CN 帧时间戳基准。
    cn_timestamp_base: u32,
    /// CN 帧使用的 SSRC（应与原 RTP 流一致）。
    cn_ssrc: u32,
    /// CN 帧使用的 payload type（13 = CN 静态 PT，RFC 3389）。
    cn_payload_type: u8,
    /// CN 帧是否已初始化（SSRC/时间戳基准）。
    cn_initialized: bool,
}

impl DtxController {
    /// 使用指定配置创建 DTX 控制器。
    pub fn new(config: ComfortNoiseConfig) -> Self {
        Self {
            config,
            bg_noise: BackgroundNoiseEstimator::new(),
            silence_frame_count: 0,
            speech_frame_count: 0,
            in_dtx: false,
            cn_accumulated_ms: 0,
            cn_sequence: 0,
            cn_timestamp_base: 0,
            cn_ssrc: 0,
            cn_payload_type: 13, // RFC 3389 CN
            cn_initialized: false,
        }
    }

    /// 使用默认配置创建 DTX 控制器。
    pub fn with_default_config() -> Self {
        Self::new(ComfortNoiseConfig::default())
    }

    /// 配置 CN 帧使用的 SSRC 与 payload type。
    pub fn configure_cn(&mut self, ssrc: u32, payload_type: u8) {
        self.cn_ssrc = ssrc;
        self.cn_payload_type = payload_type;
    }

    /// 处理一帧 RTP 包，返回 DTX 决策。
    ///
    /// `rms` 为本帧 PCM 样本的 RMS（线性值）。
    /// `frame_duration_ms` 为本帧时长（毫秒）。
    pub fn process_frame(&mut self, rms: f64, frame_duration_ms: u32) -> DtxDecision {
        let db = if rms > 0.0 {
            20.0 * (rms / 32_768.0).log10()
        } else {
            f64::NEG_INFINITY
        };
        let is_silent = db < f64::from(self.config.silence_threshold_db);

        match (is_silent, self.in_dtx) {
            (true, false) => {
                // 语音→静音过渡：累积 hangover 帧
                self.silence_frame_count = self.silence_frame_count.saturating_add(1);
                self.speech_frame_count = 0;
                // 在 hangover 期间更新背景噪声估计
                self.bg_noise.observe_silence(rms);
                if self.silence_frame_count >= self.config.hangover_frames {
                    self.in_dtx = true;
                    self.cn_accumulated_ms = 0;
                    DtxDecision::Suppress
                } else {
                    DtxDecision::Forward
                }
            }
            (true, true) => {
                // 持续静音：抑制并更新噪声估计
                self.bg_noise.observe_silence(rms);
                self.cn_accumulated_ms = self.cn_accumulated_ms.saturating_add(frame_duration_ms);
                DtxDecision::Suppress
            }
            (false, true) => {
                // 静音→语音过渡：需要 speech_recovery_frames 确认
                self.speech_frame_count = self.speech_frame_count.saturating_add(1);
                if self.speech_frame_count >= self.config.speech_recovery_frames {
                    self.in_dtx = false;
                    self.silence_frame_count = 0;
                }
                DtxDecision::Forward
            }
            (false, false) => {
                // 持续语音：重置计数
                self.silence_frame_count = 0;
                self.speech_frame_count = self.speech_frame_count.saturating_add(1);
                DtxDecision::Forward
            }
        }
    }

    /// 判断当前是否应该注入 CN 帧。
    ///
    /// 仅在 `in_dtx` 状态下有效，根据累计时间判断是否到达 CN 发送点。
    pub fn should_emit_cn(&self) -> bool {
        self.in_dtx && self.cn_accumulated_ms >= self.config.cn_frame_interval_ms
    }

    /// 生成一个 CN 帧的 RTP payload。
    ///
    /// 对于 G.711 编解码器，CN payload 为固定大小的随机噪声样本，
    /// 能量由背景噪声估计器决定。返回 `(payload, sequence, timestamp, ssrc, payload_type)`。
    pub fn emit_cn_frame(
        &mut self,
        codec: AudioCodec,
        previous_timestamp: u32,
        previous_sequence: u16,
    ) -> Option<(Vec<u8>, u16, u32, u32, u8)> {
        if !self.in_dtx {
            return None;
        }

        if !self.cn_initialized {
            self.cn_timestamp_base = previous_timestamp;
            self.cn_sequence = previous_sequence;
            self.cn_initialized = true;
        }

        // 推进时间戳与序号
        let timestamp_increment = self.config.cn_frame_samples as u32;
        self.cn_timestamp_base = self.cn_timestamp_base.wrapping_add(timestamp_increment);
        self.cn_sequence = self.cn_sequence.wrapping_add(1);

        // 重置累计时间
        self.cn_accumulated_ms = 0;

        // 生成 CN payload
        let payload = generate_cn_payload(
            codec,
            self.config.cn_frame_samples,
            self.bg_noise.noise_rms(),
        );

        Some((
            payload,
            self.cn_sequence,
            self.cn_timestamp_base,
            self.cn_ssrc,
            self.cn_payload_type,
        ))
    }

    /// 重置 DTX 状态机（例如 SSRC 切换、CODEC 变更）。
    pub fn reset(&mut self) {
        self.bg_noise.reset();
        self.silence_frame_count = 0;
        self.speech_frame_count = 0;
        self.in_dtx = false;
        self.cn_accumulated_ms = 0;
        self.cn_initialized = false;
    }

    /// 是否处于 DTX 抑制状态。
    pub fn is_in_dtx(&self) -> bool {
        self.in_dtx
    }

    /// 当前背景噪声 dB。
    pub fn noise_db(&self) -> f64 {
        self.bg_noise.noise_db()
    }
}

/// 生成单个 CN 帧的 payload。
///
/// 实现 RFC 3389 兼容的简单 CN：
/// - G.711 (PCMA/PCMU)：基于噪声 RMS 生成高斯白噪声并编码为 G.711
/// - 其他编解码器：返回零长度 payload，调用方应跳过 CN 注入
pub fn generate_cn_payload(codec: AudioCodec, samples: usize, target_rms: f64) -> Vec<u8> {
    match codec {
        AudioCodec::Pcma | AudioCodec::Pcmu => {
            let mut payload = Vec::with_capacity(samples);
            let scale = if target_rms > 1.0 {
                target_rms
            } else {
                // 默认低能量噪声（约 -55dB）
                100.0
            };
            for i in 0..samples {
                // 简易 Box-Muller 生成高斯样本
                let noise = gaussian_sample(i as u64, scale);
                let clamped = noise.clamp(-32_768.0, 32_767.0) as i16;
                let encoded = match codec {
                    AudioCodec::Pcma => crate::g711::linear_to_alaw(clamped),
                    AudioCodec::Pcmu => crate::g711::linear_to_ulaw(clamped),
                    _ => unreachable!(),
                };
                payload.push(encoded);
            }
            payload
        }
        _ => Vec::new(),
    }
}

/// 简易确定性高斯样本生成（基于 hash + Box-Muller 近似）。
///
/// 使用确定性 seed 保证 CN 帧可复现，避免引入真实随机数依赖。
fn gaussian_sample(seed: u64, scale: f64) -> f64 {
    // 使用 splitmix64 生成均匀分布
    let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    let u1 = (z as f64 / u64::MAX as f64).max(1e-10);
    let u2 = ((z.wrapping_mul(0x5851_F42D_4C95_7F2D)) as f64 / u64::MAX as f64).max(1e-10);

    // Box-Muller 变换
    let mag = (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos();
    mag * scale * 0.3 // 缩放至约 -20dBFS 以下的舒适噪音
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn background_noise_estimator_converges() {
        let mut est = BackgroundNoiseEstimator::with_smoothing(0.9);
        for _ in 0..20 {
            est.observe_silence(500.0);
        }
        let db = est.noise_db();
        // 500 RMS 约 -36 dB
        assert!(db > -40.0 && db < -32.0, "noise_db={db}");
    }

    #[test]
    fn background_noise_estimator_resets() {
        let mut est = BackgroundNoiseEstimator::new();
        est.observe_silence(1000.0);
        est.reset();
        assert!((est.noise_rms() - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn dtx_transitions_after_hangover() {
        let config = ComfortNoiseConfig {
            hangover_frames: 3,
            ..Default::default()
        };
        let mut dtx = DtxController::new(config);

        // 语音帧：应转发
        assert_eq!(dtx.process_frame(10_000.0, 20), DtxDecision::Forward);

        // 第 1-2 个静音帧：hangover 期间，仍转发
        assert_eq!(dtx.process_frame(10.0, 20), DtxDecision::Forward);
        assert_eq!(dtx.process_frame(10.0, 20), DtxDecision::Forward);

        // 第 3 个静音帧：进入 DTX，开始抑制
        assert_eq!(dtx.process_frame(10.0, 20), DtxDecision::Suppress);
        assert!(dtx.is_in_dtx());

        // 继续静音：抑制
        assert_eq!(dtx.process_frame(10.0, 20), DtxDecision::Suppress);
    }

    #[test]
    fn dtx_recovers_on_speech() {
        let config = ComfortNoiseConfig {
            hangover_frames: 1,
            speech_recovery_frames: 1,
            ..Default::default()
        };
        let mut dtx = DtxController::new(config);

        // 进入 DTX
        dtx.process_frame(10.0, 20);
        assert!(dtx.is_in_dtx());

        // 语音帧：立即恢复
        assert_eq!(dtx.process_frame(10_000.0, 20), DtxDecision::Forward);
        assert!(!dtx.is_in_dtx());
    }

    #[test]
    fn dtx_should_emit_cn_after_interval() {
        let config = ComfortNoiseConfig {
            cn_frame_interval_ms: 50,
            hangover_frames: 1,
            ..Default::default()
        };
        let mut dtx = DtxController::new(config);

        // 进入 DTX
        dtx.process_frame(10.0, 20);
        // 累计 20ms，不够 CN 间隔
        dtx.process_frame(10.0, 20);
        assert!(!dtx.should_emit_cn());

        // 再累计 20ms+20ms=60ms，超过 50ms
        dtx.process_frame(10.0, 20);
        dtx.process_frame(10.0, 20);
        assert!(dtx.should_emit_cn());
    }

    #[test]
    fn cn_frame_payload_is_generated_for_g711() {
        let payload_pcma = generate_cn_payload(AudioCodec::Pcma, 160, 500.0);
        assert_eq!(payload_pcma.len(), 160);

        let payload_pcmu = generate_cn_payload(AudioCodec::Pcmu, 160, 500.0);
        assert_eq!(payload_pcmu.len(), 160);

        // 不支持的编解码器应返回空 payload
        let payload_opus = generate_cn_payload(AudioCodec::Opus, 160, 500.0);
        assert!(payload_opus.is_empty());
    }

    #[test]
    fn emit_cn_frame_returns_valid_metadata() {
        let config = ComfortNoiseConfig {
            hangover_frames: 1,
            ..Default::default()
        };
        let mut dtx = DtxController::new(config);
        dtx.configure_cn(0xDEAD_BEEF, 13);

        // 进入 DTX
        dtx.process_frame(10.0, 20);
        dtx.process_frame(10.0, 20);
        dtx.process_frame(10.0, 20);

        let result = dtx.emit_cn_frame(AudioCodec::Pcma, 1000, 100);
        assert!(result.is_some());

        let (payload, seq, ts, ssrc, pt) = result.unwrap();
        assert_eq!(payload.len(), 160);
        assert_eq!(seq, 101);
        assert_eq!(ts, 1000 + 160);
        assert_eq!(ssrc, 0xDEAD_BEEF);
        assert_eq!(pt, 13);
    }

    #[test]
    fn emit_cn_frame_returns_none_outside_dtx() {
        let mut dtx = DtxController::with_default_config();
        let result = dtx.emit_cn_frame(AudioCodec::Pcma, 0, 0);
        assert!(result.is_none());
    }

    #[test]
    fn dtx_reset_clears_state() {
        let config = ComfortNoiseConfig {
            hangover_frames: 1,
            ..Default::default()
        };
        let mut dtx = DtxController::new(config);

        dtx.process_frame(10.0, 20);
        assert!(dtx.is_in_dtx());

        dtx.reset();
        assert!(!dtx.is_in_dtx());
        assert!((dtx.noise_db() + 96.0).abs() < 1.0);
    }

    #[test]
    fn gaussian_sample_is_bounded() {
        for i in 0..100_u64 {
            let sample = gaussian_sample(i, 1000.0);
            assert!(sample.abs() < 100_000.0, "sample={sample}");
        }
    }

    #[test]
    fn default_config_has_sane_values() {
        let config = ComfortNoiseConfig::default();
        assert_eq!(config.cn_frame_interval_ms, 50);
        assert_eq!(config.cn_frame_samples, 160);
        assert_eq!(config.hangover_frames, 4);
        assert!(config.silence_threshold_db < 0.0);
    }
}
