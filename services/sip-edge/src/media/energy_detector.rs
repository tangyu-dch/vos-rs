use rtp_core::AudioCodec;

/// RTP 能量检测器，用于基于 RMS 能量的静音/说话检测 (VAD)。
#[derive(Debug, Clone)]
pub struct RtpEnergyDetector {
    threshold_db: f32,
    required_frames: usize,
    active_frames: usize,
}

impl RtpEnergyDetector {
    /// 创建一个新的 RTP 能量检测器。
    ///
    /// # Arguments
    /// * `threshold_db` - 触发说话状态的分贝阈值 (如 -45.0)
    /// * `required_frames` - 连续超过阈值多少帧后判定为说话 (如 4)
    pub fn new(threshold_db: f32, required_frames: usize) -> Self {
        Self {
            threshold_db,
            required_frames,
            active_frames: 0,
        }
    }

    /// 处理一个 RTP 负荷并返回当前是否判定为说话。
    pub fn process_packet(&mut self, payload: &[u8], codec: AudioCodec) -> bool {
        if payload.is_empty() {
            self.active_frames = 0;
            return false;
        }

        let mut sum_squares = 0.0_f64;
        let mut count = 0;

        match codec {
            AudioCodec::Pcma => {
                for &b in payload {
                    let sample = crate::media::recording::decode_pcma(b) as f64;
                    sum_squares += sample * sample;
                    count += 1;
                }
            }
            AudioCodec::Pcmu => {
                for &b in payload {
                    let sample = crate::media::recording::decode_pcmu(b) as f64;
                    sum_squares += sample * sample;
                    count += 1;
                }
            }
            _ => {
                // 当前仅支持 G.711 A-law / U-law 的直接能量计算
                self.active_frames = 0;
                return false;
            }
        }

        if count == 0 {
            self.active_frames = 0;
            return false;
        }

        let rms = (sum_squares / count as f64).sqrt();
        let db = if rms > 0.0 {
            20.0 * (rms / 32768.0).log10()
        } else {
            -f64::INFINITY
        };

        if db >= self.threshold_db as f64 {
            self.active_frames += 1;
        } else {
            self.active_frames = 0;
        }

        self.active_frames >= self.required_frames
    }

    /// 当前是否处于说话状态
    pub fn is_talking(&self) -> bool {
        self.active_frames >= self.required_frames
    }

    /// 处理解出来的 PCM 样本，返回当前是否处于说话状态
    pub fn process_pcm_sample(&mut self, pcm_samples: &[i16]) -> bool {
        if pcm_samples.is_empty() {
            self.active_frames = 0;
            return false;
        }

        let mut sum_squares = 0.0_f64;
        let count = pcm_samples.len();

        for &sample in pcm_samples {
            let sample_f64 = sample as f64;
            sum_squares += sample_f64 * sample_f64;
        }

        let rms = (sum_squares / count as f64).sqrt();
        let db = if rms > 0.0 {
            20.0 * (rms / 32768.0).log10()
        } else {
            -f64::INFINITY
        };

        if db >= self.threshold_db as f64 {
            self.active_frames += 1;
        } else {
            self.active_frames = 0;
        }

        self.active_frames >= self.required_frames
    }

    /// 重置检测器状态
    pub fn reset(&mut self) {
        self.active_frames = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rtp_core::AudioCodec;

    // 测试用的简单 mock decode_pcma/decode_pcmu 函数
    // 在实际的库中会使用 crate::media::recording::decode_pcma
    // 但为了确保测试能独立计算，这里构造特定的 payload 来触发不同的 RMS

    #[test]
    fn test_energy_detector_below_threshold() {
        let mut detector = RtpEnergyDetector::new(-40.0, 3);
        
        // 生成全 0 负荷，G.711 A-law 的 0xD5 解码后接近 0
        // 不过我们这里随便给点小数据
        let silence_payload = vec![0xD5; 160]; 
        
        assert_eq!(detector.process_packet(&silence_payload, AudioCodec::Pcma), false);
        assert_eq!(detector.active_frames, 0);
    }

    #[test]
    fn test_energy_detector_above_threshold() {
        let mut detector = RtpEnergyDetector::new(-45.0, 3);
        
        // 构造一个会解码出较大振幅的负荷。
        // A-law 中 0x2a 代表一个较大的正值。
        let loud_payload = vec![0x2a; 160];
        
        assert_eq!(detector.process_packet(&loud_payload, AudioCodec::Pcma), false);
        assert_eq!(detector.active_frames, 1);
        
        assert_eq!(detector.process_packet(&loud_payload, AudioCodec::Pcma), false);
        assert_eq!(detector.active_frames, 2);
        
        // 第三帧，达到 required_frames = 3
        assert_eq!(detector.process_packet(&loud_payload, AudioCodec::Pcma), true);
        assert_eq!(detector.active_frames, 3);
        assert!(detector.is_talking());
        
        // 回落到静音
        let silence_payload = vec![0xD5; 160];
        assert_eq!(detector.process_packet(&silence_payload, AudioCodec::Pcma), false);
        assert_eq!(detector.active_frames, 0);
        assert!(!detector.is_talking());
    }

    #[test]
    fn test_energy_detector_unsupported_codec() {
        let mut detector = RtpEnergyDetector::new(-45.0, 1);
        let payload = vec![0xFF; 160];
        
        assert_eq!(detector.process_packet(&payload, AudioCodec::Opus), false);
        assert_eq!(detector.active_frames, 0);
    }

    #[test]
    fn test_energy_detector_reset() {
        let mut detector = RtpEnergyDetector::new(-45.0, 2);
        let loud_payload = vec![0x2a; 160];
        
        detector.process_packet(&loud_payload, AudioCodec::Pcma);
        assert_eq!(detector.active_frames, 1);
        
        detector.reset();
        assert_eq!(detector.active_frames, 0);
    }
    
    #[test]
    fn test_energy_detector_empty_payload() {
        let mut detector = RtpEnergyDetector::new(-45.0, 1);
        let payload = vec![];
        
        assert_eq!(detector.process_packet(&payload, AudioCodec::Pcma), false);
    }
}
