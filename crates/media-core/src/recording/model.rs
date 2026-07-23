use super::{
    available_disk_bytes, RecordedRtpPacket, RecordingPool, RECORDING_CHANNELS,
    RECORDING_SAMPLE_RATE,
};
use rtp_core::{AudioCodec, RtpPacketView};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

static NEXT_RECORDING_SESSION_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone)]
pub struct RecordingLeg {
    pub session: Arc<RecordingSession>,
    pub channel: RecordingChannel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordingChannel {
    Caller,
    Gateway,
}

impl RecordingChannel {
    pub fn index(self) -> usize {
        match self {
            Self::Caller => 0,
            Self::Gateway => 1,
        }
    }
}

#[derive(Debug)]
pub struct RecordingSession {
    pub info: Arc<RecordingSessionInfo>,
    pub pool: Arc<RecordingPool>,
}

impl RecordingSession {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        call_id: String,
        wav_path: PathBuf,
        min_free_bytes: u64,
        max_file_bytes: u64,
        max_duration_secs: u64,
        pool: Arc<RecordingPool>,
        format_str: String,
    ) -> Self {
        Self {
            info: Arc::new(RecordingSessionInfo {
                id: NEXT_RECORDING_SESSION_ID.fetch_add(1, Ordering::Relaxed),
                call_id,
                wav_path,
                min_free_bytes,
                max_file_bytes,
                max_duration_secs,
                header_bytes: pool.header_bytes(),
                last_disk_check_ms: AtomicU64::new(0),
                format_str,
                has_error: std::sync::atomic::AtomicBool::new(false),
            }),
            pool,
        }
    }

    pub fn try_record(
        &self,
        channel: RecordingChannel,
        packet: RtpPacketView<'_>,
    ) -> io::Result<bool> {
        if self.info.has_error.load(Ordering::Acquire) {
            return Err(io::Error::other("recording session has encountered error"));
        }

        if AudioCodec::from_static_payload_type(packet.payload_type).is_none()
            || packet.payload.is_empty()
        {
            return Ok(false);
        }

        self.pool.try_record(
            Arc::clone(&self.info),
            RecordedRtpPacket {
                channel,
                payload_type: packet.payload_type,
                timestamp: packet.timestamp,
                payload: self.pool.copy_payload(packet.payload),
            },
        )
    }

    #[doc(hidden)]
    pub fn flush(&self) -> io::Result<()> {
        self.pool.flush(Arc::clone(&self.info))
    }
}

impl Drop for RecordingSession {
    fn drop(&mut self) {
        self.pool.finish(self.info.id);
    }
}

#[derive(Debug)]
pub struct RecordingSessionInfo {
    pub id: u64,
    pub call_id: String,
    pub wav_path: PathBuf,
    pub min_free_bytes: u64,
    pub max_file_bytes: u64,
    pub max_duration_secs: u64,
    pub header_bytes: u64,
    pub last_disk_check_ms: AtomicU64,
    pub format_str: String,
    pub has_error: std::sync::atomic::AtomicBool,
}

impl RecordingSessionInfo {
    pub fn max_file_frames(&self) -> Option<u64> {
        let duration_frames = (self.max_duration_secs > 0).then(|| {
            self.max_duration_secs
                .saturating_mul(u64::from(RECORDING_SAMPLE_RATE))
        });
        let size_frames = (self.max_file_bytes > self.header_bytes)
            .then(|| (self.max_file_bytes - self.header_bytes) / u64::from(RECORDING_CHANNELS * 2));
        match (duration_frames, size_frames) {
            (Some(duration), Some(size)) => Some(duration.min(size)),
            (Some(duration), None) => Some(duration),
            (None, Some(size)) => Some(size),
            (None, None) => None,
        }
    }

    pub fn ensure_disk_space(&self) -> io::Result<()> {
        if self.min_free_bytes == 0 {
            return Ok(());
        }

        let now = super::unix_timestamp_millis() as u64;
        let last_check = self.last_disk_check_ms.load(Ordering::Relaxed);
        if now.saturating_sub(last_check) < 1_000 {
            return Ok(());
        }
        if self
            .last_disk_check_ms
            .compare_exchange(last_check, now, Ordering::Relaxed, Ordering::Relaxed)
            .is_err()
        {
            return Ok(());
        }

        let directory = self.wav_path.parent().unwrap_or_else(|| Path::new("."));
        let available = available_disk_bytes(directory)?;
        if available < self.min_free_bytes {
            return Err(io::Error::other(format!(
                "recording disk free space {available} bytes is below configured minimum {} bytes",
                self.min_free_bytes
            )));
        }
        Ok(())
    }
}
