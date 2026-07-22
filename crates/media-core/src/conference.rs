//! Stateless conference audio processing primitives.

use crate::g711::{linear_to_alaw, linear_to_ulaw};
use crate::recording::{decode_pcma, decode_pcmu};

/// Number of 8 kHz PCM samples in one 20 ms conference frame.
pub const CONFERENCE_FRAME_SAMPLES: usize = 160;

/// One 20 ms, 8 kHz mono PCM conference frame.
pub type ConferenceFrame = [i16; CONFERENCE_FRAME_SAMPLES];

/// G.711 codec used to encode or decode a conference frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConferenceCodec {
    /// G.711 A-law (PCMA).
    Pcma,
    /// G.711 mu-law (PCMU).
    Pcmu,
}

/// Removes one conference frame from an input buffer.
///
/// Missing samples are padded with silence. A muted participant's samples are
/// consumed but the returned frame remains silent, preventing buffered audio
/// from playing after the participant is unmuted.
pub fn take_conference_frame(buffer: &mut Vec<i16>, muted: bool) -> ConferenceFrame {
    let mut frame = [0_i16; CONFERENCE_FRAME_SAMPLES];
    let consumed = buffer.len().min(CONFERENCE_FRAME_SAMPLES);

    if !muted {
        frame[..consumed].copy_from_slice(&buffer[..consumed]);
    }
    buffer.drain(..consumed);

    frame
}

/// Mixes input frames and clamps the result to signed 16-bit PCM.
///
/// Callers implement mix-minus by omitting the destination participant's own
/// frame from the iterator.
pub fn mix_conference_frames<'a>(
    frames: impl IntoIterator<Item = &'a ConferenceFrame>,
) -> ConferenceFrame {
    let mut mixed = [0_i32; CONFERENCE_FRAME_SAMPLES];
    for frame in frames {
        for (output, sample) in mixed.iter_mut().zip(frame) {
            *output += i32::from(*sample);
        }
    }

    mixed.map(|sample| sample.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16)
}

/// Encodes a PCM conference frame as G.711 A-law or mu-law.
pub fn encode_conference_frame(frame: &ConferenceFrame, codec: ConferenceCodec) -> Vec<u8> {
    match codec {
        ConferenceCodec::Pcma => frame.iter().map(|&sample| linear_to_alaw(sample)).collect(),
        ConferenceCodec::Pcmu => frame.iter().map(|&sample| linear_to_ulaw(sample)).collect(),
    }
}

/// Decodes G.711 A-law or mu-law bytes into signed 16-bit PCM samples.
pub fn decode_conference_audio(payload: &[u8], codec: ConferenceCodec) -> Vec<i16> {
    match codec {
        ConferenceCodec::Pcma => payload.iter().map(|&sample| decode_pcma(sample)).collect(),
        ConferenceCodec::Pcmu => payload.iter().map(|&sample| decode_pcmu(sample)).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn take_frame_pads_short_input_and_consumes_only_one_frame() {
        let mut input: Vec<i16> = (1..=200).collect();
        let frame = take_conference_frame(&mut input, false);

        assert_eq!(frame[0], 1);
        assert_eq!(frame[159], 160);
        assert_eq!(input, (161..=200).collect::<Vec<_>>());

        let short_frame = take_conference_frame(&mut input, false);
        assert_eq!(short_frame[0], 161);
        assert_eq!(short_frame[39], 200);
        assert!(short_frame[40..].iter().all(|sample| *sample == 0));
        assert!(input.is_empty());
    }

    #[test]
    fn muted_frame_is_silent_and_consumes_buffered_audio() {
        let mut input = vec![123_i16; CONFERENCE_FRAME_SAMPLES + 1];
        let frame = take_conference_frame(&mut input, true);

        assert!(frame.iter().all(|sample| *sample == 0));
        assert_eq!(input, vec![123]);
    }

    #[test]
    fn mixes_frames_with_i16_saturation() {
        let positive = [30_000_i16; CONFERENCE_FRAME_SAMPLES];
        let positive_two = [10_000_i16; CONFERENCE_FRAME_SAMPLES];
        let negative = [-30_000_i16; CONFERENCE_FRAME_SAMPLES];
        let negative_two = [-10_000_i16; CONFERENCE_FRAME_SAMPLES];

        let clipped_positive = mix_conference_frames([&positive, &positive_two]);
        let clipped_negative = mix_conference_frames([&negative, &negative_two]);

        assert!(clipped_positive.iter().all(|sample| *sample == i16::MAX));
        assert!(clipped_negative.iter().all(|sample| *sample == i16::MIN));
    }

    #[test]
    fn mix_minus_is_expressed_by_omitting_the_destination_frame() {
        let own = [100_i16; CONFERENCE_FRAME_SAMPLES];
        let peer_one = [200_i16; CONFERENCE_FRAME_SAMPLES];
        let peer_two = [-50_i16; CONFERENCE_FRAME_SAMPLES];
        let frames = [(1_u16, own), (2, peer_one), (3, peer_two)];

        let mixed = mix_conference_frames(
            frames
                .iter()
                .filter_map(|(port, frame)| (*port != 1).then_some(frame)),
        );

        assert!(mixed.iter().all(|sample| *sample == 150));
    }

    #[test]
    fn g711_codecs_encode_and_decode_a_full_frame() {
        let mut frame = [0_i16; CONFERENCE_FRAME_SAMPLES];
        frame[0] = i16::MIN;
        frame[1] = i16::MAX;

        for codec in [ConferenceCodec::Pcma, ConferenceCodec::Pcmu] {
            let encoded = encode_conference_frame(&frame, codec);
            let decoded = decode_conference_audio(&encoded, codec);

            assert_eq!(encoded.len(), CONFERENCE_FRAME_SAMPLES);
            assert_eq!(decoded.len(), CONFERENCE_FRAME_SAMPLES);
        }
    }
}
