use std::collections::VecDeque;
use rtp_core::AudioCodec;
use rubato::{Resampler, FftFixedOut};

pub struct LiveTranscoder {
    local_codec: AudioCodec,
    peer_codec: AudioCodec,
    g711_to_opus: Option<G711ToOpusTranscoder>,
    opus_to_g711: Option<OpusToG711Transcoder>,
}

impl LiveTranscoder {
    pub fn new(local_codec: AudioCodec, peer_codec: AudioCodec) -> Result<Self, String> {
        let mut g711_to_opus = None;
        let mut opus_to_g711 = None;

        if local_codec == AudioCodec::Pcma || local_codec == AudioCodec::Pcmu {
            if peer_codec == AudioCodec::Opus {
                g711_to_opus = Some(G711ToOpusTranscoder::new(local_codec)?);
            }
        } else if local_codec == AudioCodec::Opus {
            if peer_codec == AudioCodec::Pcma || peer_codec == AudioCodec::Pcmu {
                opus_to_g711 = Some(OpusToG711Transcoder::new(peer_codec)?);
            }
        }

        Ok(Self {
            local_codec,
            peer_codec,
            g711_to_opus,
            opus_to_g711,
        })
    }

    pub fn transcode(&mut self, payload: &[u8]) -> Result<Vec<u8>, String> {
        if let Some(transcoder) = &mut self.g711_to_opus {
            transcoder.transcode(payload)
        } else if let Some(transcoder) = &mut self.opus_to_g711 {
            transcoder.transcode(payload)
        } else {
            Err(format!(
                "Unsupported transcoding path: {:?} -> {:?}",
                self.local_codec, self.peer_codec
            ))
        }
    }
}

struct G711ToOpusTranscoder {
    codec: AudioCodec,
    resampler: FftFixedOut<f32>,
    encoder: opus::Encoder,
    fifo: VecDeque<f32>,
}

impl G711ToOpusTranscoder {
    fn new(codec: AudioCodec) -> Result<Self, String> {
        let resampler = FftFixedOut::<f32>::new(
            8000,
            48000,
            960, // Fixed 20ms Opus frame output size at 48kHz
            2,
            1,
        )
        .map_err(|e| format!("Failed to create FFT resampler: {e}"))?;

        let encoder = opus::Encoder::new(48000, opus::Channels::Mono, opus::Application::Voip)
            .map_err(|e| format!("Failed to create Opus encoder: {e}"))?;

        Ok(Self {
            codec,
            resampler,
            encoder,
            fifo: VecDeque::new(),
        })
    }

    fn transcode(&mut self, payload: &[u8]) -> Result<Vec<u8>, String> {
        // 1. Decode G.711 payload to PCM f32
        for &byte in payload {
            let pcm = match self.codec {
                AudioCodec::Pcma => crate::media::recording::decode_pcma(byte),
                AudioCodec::Pcmu => crate::media::recording::decode_pcmu(byte),
                _ => return Err("Invalid G.711 codec".to_string()),
            };
            self.fifo.push_back(pcm as f32 / 32768.0);
        }

        // 2. Perform Resampling
        let needed = self.resampler.input_frames_next();
        if self.fifo.len() < needed {
            return Ok(Vec::new()); // Wait for enough samples
        }

        let mut input_channel = Vec::with_capacity(needed);
        for _ in 0..needed {
            input_channel.push(self.fifo.pop_front().unwrap_or(0.0));
        }

        let output = self.resampler.process(&[input_channel], None)
            .map_err(|e| format!("Resampling error: {e}"))?;
        let output_pcm = &output[0];

        // 3. Convert resampled f32 PCM to i16 for Opus
        let mut opus_input = vec![0i16; output_pcm.len()];
        for i in 0..output_pcm.len() {
            let val = (output_pcm[i] * 32767.0).clamp(-32768.0, 32767.0) as i16;
            opus_input[i] = val;
        }

        // 4. Encode to Opus
        let mut out_buf = vec![0u8; 1275]; // Maximum Opus packet size
        let len = self.encoder.encode(&opus_input, &mut out_buf)
            .map_err(|e| format!("Opus encode error: {e}"))?;

        out_buf.truncate(len);
        Ok(out_buf)
    }
}

struct OpusToG711Transcoder {
    codec: AudioCodec,
    decoder: opus::Decoder,
    resampler: FftFixedOut<f32>,
    fifo: VecDeque<f32>,
}

impl OpusToG711Transcoder {
    fn new(codec: AudioCodec) -> Result<Self, String> {
        let decoder = opus::Decoder::new(48000, opus::Channels::Mono)
            .map_err(|e| format!("Failed to create Opus decoder: {e}"))?;

        let resampler = FftFixedOut::<f32>::new(
            48000,
            8000,
            160, // Fixed 20ms G.711 frame output size at 8kHz
            2,
            1,
        )
        .map_err(|e| format!("Failed to create FFT resampler: {e}"))?;

        Ok(Self {
            codec,
            decoder,
            resampler,
            fifo: VecDeque::new(),
        })
    }

    fn transcode(&mut self, payload: &[u8]) -> Result<Vec<u8>, String> {
        // 1. Decode Opus packet to PCM i16 (20ms mono = 960 samples)
        let mut pcm_buf = vec![0i16; 960];
        let len = self.decoder.decode(payload, &mut pcm_buf, false)
            .map_err(|e| format!("Opus decode error: {e}"))?;
        pcm_buf.truncate(len);

        // 2. Add decoded PCM f32 samples to FIFO
        for sample in pcm_buf {
            self.fifo.push_back(sample as f32 / 32768.0);
        }

        // 3. Perform Resampling
        let needed = self.resampler.input_frames_next();
        if self.fifo.len() < needed {
            return Ok(Vec::new());
        }

        let mut input_channel = Vec::with_capacity(needed);
        for _ in 0..needed {
            input_channel.push(self.fifo.pop_front().unwrap_or(0.0));
        }

        let output = self.resampler.process(&[input_channel], None)
            .map_err(|e| format!("Resampling error: {e}"))?;
        let output_pcm = &output[0];

        // 4. Encode to G.711 (PCMA/PCMU)
        let mut g711_payload = Vec::with_capacity(output_pcm.len());
        for &sample in output_pcm {
            let val = (sample * 32767.0).clamp(-32768.0, 32767.0) as i16;
            let byte = match self.codec {
                AudioCodec::Pcma => crate::media::transcode::linear_to_alaw(val),
                AudioCodec::Pcmu => crate::media::transcode::linear_to_ulaw(val),
                _ => return Err("Invalid target G.711 codec".to_string()),
            };
            g711_payload.push(byte);
        }

        Ok(g711_payload)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_g711_to_opus_and_back() {
        let mut g711_to_opus = LiveTranscoder::new(AudioCodec::Pcma, AudioCodec::Opus).unwrap();
        let mut opus_to_g711 = LiveTranscoder::new(AudioCodec::Opus, AudioCodec::Pcma).unwrap();

        // 模拟一个 1000Hz 正弦波信号
        let sample_rate = 8000;
        let mut pcm_samples = Vec::new();
        for i in 0..1600 { // 200ms = 10 packets
            let t = i as f32 / sample_rate as f32;
            let sample = (2.0 * std::f32::consts::PI * 1000.0 * t).sin();
            let val = (sample * 32767.0) as i16;
            pcm_samples.push(crate::media::transcode::linear_to_alaw(val));
        }

        let mut opus_packets = Vec::new();
        // 1. 将 PCMA 转换为 Opus
        for chunk in pcm_samples.chunks(160) {
            let res = g711_to_opus.transcode(chunk).unwrap();
            if !res.is_empty() {
                opus_packets.push(res);
            }
        }

        assert!(!opus_packets.is_empty(), "应当能输出 Opus 帧");

        // 2. 将 Opus 转换回 PCMA
        let mut recovered_pcm = Vec::new();
        for packet in &opus_packets {
            let res = opus_to_g711.transcode(packet).unwrap();
            recovered_pcm.extend(res);
        }

        assert!(!recovered_pcm.is_empty(), "应当能转码回 PCMA 字节");
    }
}

