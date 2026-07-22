//! Live G.711 and Opus transcoding.

use std::collections::VecDeque;

use rtp_core::AudioCodec;
use rubato::{FftFixedOut, Resampler};

const OPUS_SAMPLE_RATE: u32 = 48_000;
const OPUS_FRAME_SAMPLES: usize = 960;
const OPUS_MAX_PACKET_BYTES: usize = 1_275;
const G711_SAMPLE_RATE: usize = 8_000;
const G711_FRAME_SAMPLES: usize = 160;

/// Stateful live transcoder for supported G.711 and Opus codec pairs.
pub struct LiveTranscoder {
    local_codec: AudioCodec,
    peer_codec: AudioCodec,
    g711_to_opus: Option<G711ToOpusTranscoder>,
    opus_to_g711: Option<OpusToG711Transcoder>,
}

impl LiveTranscoder {
    /// Creates a transcoder for a local-to-peer codec direction.
    pub fn new(local_codec: AudioCodec, peer_codec: AudioCodec) -> Result<Self, String> {
        let mut g711_to_opus = None;
        let mut opus_to_g711 = None;

        if matches!(local_codec, AudioCodec::Pcma | AudioCodec::Pcmu)
            && peer_codec == AudioCodec::Opus
        {
            g711_to_opus = Some(G711ToOpusTranscoder::new(local_codec)?);
        } else if local_codec == AudioCodec::Opus
            && matches!(peer_codec, AudioCodec::Pcma | AudioCodec::Pcmu)
        {
            opus_to_g711 = Some(OpusToG711Transcoder::new(peer_codec)?);
        }

        if g711_to_opus.is_none() && opus_to_g711.is_none() {
            return Err(format!(
                "Unsupported transcoding path: {local_codec:?} -> {peer_codec:?}"
            ));
        }

        Ok(Self {
            local_codec,
            peer_codec,
            g711_to_opus,
            opus_to_g711,
        })
    }

    /// Transcodes one RTP payload, returning an empty payload until a full output frame is ready.
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
            G711_SAMPLE_RATE,
            OPUS_SAMPLE_RATE as usize,
            OPUS_FRAME_SAMPLES,
            2,
            1,
        )
        .map_err(|error| format!("Failed to create FFT resampler: {error}"))?;

        let encoder = opus::Encoder::new(
            OPUS_SAMPLE_RATE,
            opus::Channels::Mono,
            opus::Application::Voip,
        )
        .map_err(|error| format!("Failed to create Opus encoder: {error}"))?;

        Ok(Self {
            codec,
            resampler,
            encoder,
            fifo: VecDeque::new(),
        })
    }

    fn transcode(&mut self, payload: &[u8]) -> Result<Vec<u8>, String> {
        for &byte in payload {
            let sample = match self.codec {
                AudioCodec::Pcma => crate::recording::decode_pcma(byte),
                AudioCodec::Pcmu => crate::recording::decode_pcmu(byte),
                _ => return Err("Invalid G.711 codec".to_string()),
            };
            self.fifo.push_back(sample as f32 / 32_768.0);
        }

        let needed = self.resampler.input_frames_next();
        if self.fifo.len() < needed {
            return Ok(Vec::new());
        }

        let input_channel = self.fifo.drain(..needed).collect::<Vec<_>>();
        let output = self
            .resampler
            .process(&[input_channel], None)
            .map_err(|error| format!("Resampling error: {error}"))?;
        let output_pcm = &output[0];

        let opus_input = output_pcm
            .iter()
            .map(|sample| (sample * 32_767.0).clamp(-32_768.0, 32_767.0) as i16)
            .collect::<Vec<_>>();
        let mut output_payload = vec![0_u8; OPUS_MAX_PACKET_BYTES];
        let output_len = self
            .encoder
            .encode(&opus_input, &mut output_payload)
            .map_err(|error| format!("Opus encode error: {error}"))?;

        output_payload.truncate(output_len);
        Ok(output_payload)
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
        let decoder = opus::Decoder::new(OPUS_SAMPLE_RATE, opus::Channels::Mono)
            .map_err(|error| format!("Failed to create Opus decoder: {error}"))?;
        let resampler = FftFixedOut::<f32>::new(
            OPUS_SAMPLE_RATE as usize,
            G711_SAMPLE_RATE,
            G711_FRAME_SAMPLES,
            2,
            1,
        )
        .map_err(|error| format!("Failed to create FFT resampler: {error}"))?;

        Ok(Self {
            codec,
            decoder,
            resampler,
            fifo: VecDeque::new(),
        })
    }

    fn transcode(&mut self, payload: &[u8]) -> Result<Vec<u8>, String> {
        let mut pcm_buffer = vec![0_i16; OPUS_FRAME_SAMPLES];
        let decoded_len = self
            .decoder
            .decode(payload, &mut pcm_buffer, false)
            .map_err(|error| format!("Opus decode error: {error}"))?;
        pcm_buffer.truncate(decoded_len);
        self.fifo.extend(
            pcm_buffer
                .into_iter()
                .map(|sample| sample as f32 / 32_768.0),
        );

        let needed = self.resampler.input_frames_next();
        if self.fifo.len() < needed {
            return Ok(Vec::new());
        }

        let input_channel = self.fifo.drain(..needed).collect::<Vec<_>>();
        let output = self
            .resampler
            .process(&[input_channel], None)
            .map_err(|error| format!("Resampling error: {error}"))?;

        output[0]
            .iter()
            .map(|sample| {
                let linear = (sample * 32_767.0).clamp(-32_768.0, 32_767.0) as i16;
                match self.codec {
                    AudioCodec::Pcma => Ok(crate::g711::linear_to_alaw(linear)),
                    AudioCodec::Pcmu => Ok(crate::g711::linear_to_ulaw(linear)),
                    _ => Err("Invalid target G.711 codec".to_string()),
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transcodes_g711_to_opus_and_back() {
        let mut g711_to_opus = LiveTranscoder::new(AudioCodec::Pcma, AudioCodec::Opus).unwrap();
        let mut opus_to_g711 = LiveTranscoder::new(AudioCodec::Opus, AudioCodec::Pcma).unwrap();

        let samples = (0..1_600)
            .map(|index| {
                let time = index as f32 / G711_SAMPLE_RATE as f32;
                let sample = (2.0 * std::f32::consts::PI * 1_000.0 * time).sin();
                crate::g711::linear_to_alaw((sample * 32_767.0) as i16)
            })
            .collect::<Vec<_>>();

        let opus_packets = samples
            .chunks(G711_FRAME_SAMPLES)
            .filter_map(|chunk| {
                let payload = g711_to_opus.transcode(chunk).unwrap();
                (!payload.is_empty()).then_some(payload)
            })
            .collect::<Vec<_>>();
        assert!(!opus_packets.is_empty());

        let recovered = opus_packets
            .iter()
            .flat_map(|packet| opus_to_g711.transcode(packet).unwrap())
            .collect::<Vec<_>>();
        assert!(!recovered.is_empty());
    }

    #[test]
    fn rejects_unsupported_codec_path_when_transcoding() {
        assert!(LiveTranscoder::new(AudioCodec::Pcma, AudioCodec::Pcmu).is_err());
    }
}
