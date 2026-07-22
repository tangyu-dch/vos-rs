//! Bounded asynchronous call recording with stereo WAV output.

mod format;
mod model;
mod pool;
mod retention;
mod wav;
mod worker;
mod writer;

pub use format::RecordingFormat;
pub use model::{RecordingChannel, RecordingLeg, RecordingSession, RecordingSessionInfo};
pub use pool::{RecordedRtpPacket, RecordingFinalizer, RecordingPool};
pub use retention::{
    available_disk_bytes, cleanup_expired_recordings, recording_file_stem, recording_segment_path,
    write_recording_segment_metadata,
};
pub use wav::{decode_pcma, decode_pcmu, write_wav_header, WavCallRecorder};
pub use worker::RecordingCommand;
pub use writer::{
    RecordingFile, RecordingWriter, RecordingWriterFactory, StandardWavWriterFactory,
};

pub const RECORDING_SAMPLE_RATE: u32 = 8_000;
pub const RECORDING_CHANNELS: u16 = 2;
pub const RECORDING_BITS_PER_SAMPLE: u16 = 16;
pub const RECORDING_FLUSH_INTERVAL_FRAMES: u64 = 1_024;
pub const RECORDING_WORKER_DRAIN_LIMIT: usize = 256;

pub(crate) fn unix_timestamp_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}
