use super::{recording_segment_path, RecordingSessionInfo, WavCallRecorder};
use std::io;
use std::path::PathBuf;
use std::sync::Arc;

pub trait RecordingWriter: Send {
    fn record(
        &mut self,
        channel: super::RecordingChannel,
        payload_type: u8,
        timestamp: u32,
        payload: &[u8],
    ) -> io::Result<bool>;

    fn would_exceed_limit(
        &self,
        channel: super::RecordingChannel,
        timestamp: u32,
        payload_len: usize,
        max_frames: Option<u64>,
    ) -> bool;

    fn flush_recording(&mut self) -> io::Result<()>;

    fn flushed_frames(&self) -> u64;
}

pub trait RecordingWriterFactory: Send + Sync {
    fn create(&self, path: &std::path::Path) -> io::Result<Box<dyn RecordingWriter>>;

    fn header_bytes(&self) -> u64;
}

#[derive(Debug, Default)]
pub struct StandardWavWriterFactory;

impl RecordingWriterFactory for StandardWavWriterFactory {
    fn create(&self, path: &std::path::Path) -> io::Result<Box<dyn RecordingWriter>> {
        Ok(Box::new(WavCallRecorder::create(path.to_path_buf())?))
    }

    fn header_bytes(&self) -> u64 {
        44
    }
}

pub struct RecordingFile {
    pub segment_index: u32,
    pub path: PathBuf,
    pub session: Arc<RecordingSessionInfo>,
    pub recorder: Box<dyn RecordingWriter>,
}

impl RecordingFile {
    pub fn create(
        session: &Arc<RecordingSessionInfo>,
        segment_index: u32,
        factory: &dyn RecordingWriterFactory,
    ) -> io::Result<Self> {
        let wav_path = recording_segment_path(session, segment_index);
        let recorder = factory.create(&wav_path)?;
        Ok(Self {
            segment_index,
            path: wav_path,
            session: Arc::clone(session),
            recorder,
        })
    }
}
