//! RMS energy-based speech activity detection for RTP audio.

use rtp_core::AudioCodec;

#[derive(Debug, Clone)]
pub struct RtpEnergyDetector {
    threshold_db: f32,
    required_frames: usize,
    active_frames: usize,
}

impl RtpEnergyDetector {
    /// Creates a detector with a dB threshold and consecutive-frame requirement.
    pub fn new(threshold_db: f32, required_frames: usize) -> Self {
        Self {
            threshold_db,
            required_frames: required_frames.max(1),
            active_frames: 0,
        }
    }

    /// Processes a G.711 RTP payload and returns whether speech is active.
    pub fn process_packet(&mut self, payload: &[u8], codec: AudioCodec) -> bool {
        if payload.is_empty() {
            self.active_frames = 0;
            return false;
        }

        let sum_squares = match codec {
            AudioCodec::Pcma => payload.iter().fold(0.0_f64, |sum, &byte| {
                let sample = f64::from(crate::recording::decode_pcma(byte));
                sum + sample * sample
            }),
            AudioCodec::Pcmu => payload.iter().fold(0.0_f64, |sum, &byte| {
                let sample = f64::from(crate::recording::decode_pcmu(byte));
                sum + sample * sample
            }),
            _ => {
                self.active_frames = 0;
                return false;
            }
        };
        self.update_from_energy(sum_squares, payload.len())
    }

    /// Returns whether the configured number of active frames has been observed.
    pub fn is_talking(&self) -> bool {
        self.active_frames >= self.required_frames
    }

    /// Processes decoded PCM samples and returns whether speech is active.
    pub fn process_pcm_sample(&mut self, pcm_samples: &[i16]) -> bool {
        if pcm_samples.is_empty() {
            self.active_frames = 0;
            return false;
        }

        let sum_squares = pcm_samples.iter().fold(0.0_f64, |sum, &sample| {
            let sample = f64::from(sample);
            sum + sample * sample
        });
        self.update_from_energy(sum_squares, pcm_samples.len())
    }

    fn update_from_energy(&mut self, sum_squares: f64, sample_count: usize) -> bool {
        let rms = (sum_squares / sample_count as f64).sqrt();
        let db = if rms > 0.0 {
            20.0 * (rms / 32_768.0).log10()
        } else {
            f64::NEG_INFINITY
        };

        if db >= f64::from(self.threshold_db) {
            self.active_frames = self.active_frames.saturating_add(1);
        } else {
            self.active_frames = 0;
        }
        self.is_talking()
    }

    /// Clears consecutive-frame state.
    pub fn reset(&mut self) {
        self.active_frames = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requires_consecutive_loud_frames() {
        let mut detector = RtpEnergyDetector::new(-45.0, 3);
        let loud = vec![0x2a; 160];
        assert!(!detector.process_packet(&loud, AudioCodec::Pcma));
        assert!(!detector.process_packet(&loud, AudioCodec::Pcma));
        assert!(detector.process_packet(&loud, AudioCodec::Pcma));

        assert!(!detector.process_packet(&[0xd5; 160], AudioCodec::Pcma));
        assert!(!detector.is_talking());
    }

    #[test]
    fn supports_pcmu_and_pcm_samples() {
        let mut detector = RtpEnergyDetector::new(-50.0, 1);
        assert!(detector.process_packet(&[0x00; 160], AudioCodec::Pcmu));
        detector.reset();
        assert!(detector.process_pcm_sample(&[20_000; 160]));
    }

    #[test]
    fn rejects_empty_and_unsupported_payloads() {
        let mut detector = RtpEnergyDetector::new(-45.0, 1);
        assert!(!detector.process_packet(&[], AudioCodec::Pcma));
        assert!(!detector.process_packet(&[0xff], AudioCodec::Opus));
    }

    #[test]
    fn zero_required_frames_is_normalized_to_one() {
        let detector = RtpEnergyDetector::new(-45.0, 0);
        assert!(!detector.is_talking());
    }
}
