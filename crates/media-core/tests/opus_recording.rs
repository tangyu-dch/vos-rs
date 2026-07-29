use media_core::g711::linear_to_alaw;
use media_core::live_transcode::LiveTranscoder;
use media_core::recording::{RecordingChannel, WavCallRecorder};
use rtp_core::AudioCodec;

#[test]
fn opus_packets_are_written_as_playable_wav_audio() {
    let suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("media-core-opus-recording-{suffix}.wav"));
    let mut encoder = LiveTranscoder::new(AudioCodec::Pcma, AudioCodec::Opus).unwrap();
    let mut recorder = WavCallRecorder::create(path.clone()).unwrap();

    let source = (0..1_600)
        .map(|index| {
            let time = index as f32 / 8_000.0;
            let sample = (2.0 * std::f32::consts::PI * 440.0 * time).sin();
            linear_to_alaw((sample * 24_000.0) as i16)
        })
        .collect::<Vec<_>>();

    let mut timestamp = 0_u32;
    let mut accepted_packets = 0;
    for frame in source.chunks(160) {
        let opus = encoder.transcode(frame).unwrap();
        if opus.is_empty() {
            continue;
        }
        assert!(recorder
            .record(RecordingChannel::Caller, AudioCodec::Opus, timestamp, &opus,)
            .unwrap());
        accepted_packets += 1;
        timestamp = timestamp.wrapping_add(960);
    }
    recorder.flush_recording().unwrap();

    let metadata = std::fs::metadata(&path).unwrap();
    assert!(accepted_packets > 0);
    assert!(metadata.len() > 44);
    std::fs::remove_file(path).unwrap();
}
