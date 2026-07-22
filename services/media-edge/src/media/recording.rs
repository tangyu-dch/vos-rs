mod direct_writer;

use rtp_core::RtpPacketView;
use sdp_core::SdpError;
use std::collections::HashSet;
use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::{fs, io};

use crate::media::config::MediaConfig;
use crate::media::relay::MediaRelayState;
use direct_writer::DirectIoWavWriterFactory;
pub use media_core::recording::{
    available_disk_bytes, cleanup_expired_recordings, decode_pcma, decode_pcmu,
    recording_file_stem, RecordingChannel, RecordingLeg, RecordingPool, RecordingSession,
};
use media_core::recording::{RecordingFinalizer, RecordingWriterFactory};

struct MediaEdgeRecordingFinalizer {
    tokio_handle: Option<tokio::runtime::Handle>,
}

impl RecordingFinalizer for MediaEdgeRecordingFinalizer {
    fn finalize(&self, wav_path: PathBuf, format: String, _call_id: String) {
        let format = crate::media::transcode::RecordingFormat::from_str(&format);
        if format == crate::media::transcode::RecordingFormat::Wav {
            return;
        }
        let Some(handle) = &self.tokio_handle else {
            tracing::warn!(path = ?wav_path, "Tokio 句柄不可用，跳过录音转码");
            return;
        };
        let _guard = handle.enter();
        crate::media::transcode::transcode_recording_async(wav_path, format);
    }
}

pub(crate) fn new_recording_pool(worker_count: usize, queue_capacity: usize) -> RecordingPool {
    let finalizer: Arc<dyn RecordingFinalizer> = Arc::new(MediaEdgeRecordingFinalizer {
        tokio_handle: tokio::runtime::Handle::try_current().ok(),
    });
    let writer_factory: Arc<dyn RecordingWriterFactory> = Arc::new(DirectIoWavWriterFactory);
    RecordingPool::with_writer_factory(
        worker_count,
        queue_capacity,
        Some(finalizer),
        writer_factory,
    )
}

pub fn recording_error(error: io::Error) -> MediaError {
    if error.kind() == io::ErrorKind::WouldBlock {
        MediaError::RecordingQueueFull
    } else {
        MediaError::Recording(error.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaError {
    InvalidUtf8,
    InvalidEndpoint(String),
    PortRangeExhausted { port_min: u16, port_max: u16 },
    Recording(String),
    RecordingQueueFull,
    Sdp(SdpError),
    Io(String),
}

impl fmt::Display for MediaError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidUtf8 => write!(formatter, "SDP body is not valid UTF-8"),
            Self::InvalidEndpoint(endpoint) => {
                write!(formatter, "invalid RTP endpoint: {endpoint}")
            }
            Self::PortRangeExhausted { port_min, port_max } => {
                write!(formatter, "RTP port range exhausted: {port_min}-{port_max}")
            }
            Self::Recording(error) => write!(formatter, "recording error: {error}"),
            Self::RecordingQueueFull => write!(formatter, "recording queue is full"),
            Self::Sdp(error) => write!(formatter, "{error}"),
            Self::Io(error) => write!(formatter, "media IO error: {error}"),
        }
    }
}

impl Error for MediaError {}

impl From<SdpError> for MediaError {
    fn from(error: SdpError) -> Self {
        Self::Sdp(error)
    }
}

impl MediaRelayState {
    pub fn start_call_recording(
        &self,
        call_id: &str,
        caller_relay_port: u16,
        gateway_relay_port: u16,
        config: &MediaConfig,
    ) -> Result<Option<PathBuf>, MediaError> {
        if !config.recording_enabled {
            return Ok(None);
        }

        let caller_relay_port = normalize_rtp_port(caller_relay_port);
        let gateway_relay_port = normalize_rtp_port(gateway_relay_port);
        self.ensure_recording_dir(&config.recording_dir)
            .map_err(recording_error)?;
        self.enforce_recording_storage_policy(
            &config.recording_dir,
            config.recording_retention_secs,
            config.recording_min_free_bytes,
        )
        .map_err(recording_error)?;

        let wav_path = config
            .recording_dir
            .join(format!("{}.wav", recording_file_stem(call_id)));
        let session = Arc::new(RecordingSession::new(
            call_id.to_string(),
            wav_path.clone(),
            config.recording_min_free_bytes,
            config.recording_max_file_bytes,
            config.recording_max_duration_secs,
            Arc::clone(&self.recording_pool),
            config.recording_format.clone(),
        ));

        self.recordings.insert(
            caller_relay_port,
            RecordingLeg {
                session: Arc::clone(&session),
                channel: RecordingChannel::Caller,
            },
        );
        self.recordings.insert(
            gateway_relay_port,
            RecordingLeg {
                session,
                channel: RecordingChannel::Gateway,
            },
        );
        self.mark_relay_features_changed(caller_relay_port);
        self.mark_relay_features_changed(gateway_relay_port);
        Ok(Some(wav_path))
    }

    fn ensure_recording_dir(&self, directory: &Path) -> io::Result<()> {
        let should_create = {
            let mut inner = self
                .state
                .lock()
                .map_err(|_| io::Error::other("media relay lock poisoned"))?;
            inner.recording_dirs.insert(directory.to_path_buf())
        };
        if !should_create {
            return Ok(());
        }
        if let Err(error) = fs::create_dir_all(directory) {
            let mut inner = self
                .state
                .lock()
                .map_err(|_| io::Error::other("media relay lock poisoned"))?;
            inner.recording_dirs.remove(directory);
            return Err(error);
        }
        Ok(())
    }

    fn enforce_recording_storage_policy(
        &self,
        directory: &Path,
        retention_secs: u64,
        min_free_bytes: u64,
    ) -> io::Result<()> {
        cleanup_expired_recordings(directory, retention_secs, &self.active_recording_paths())?;
        if min_free_bytes == 0 {
            return Ok(());
        }
        let available = available_disk_bytes(directory)?;
        if available < min_free_bytes {
            return Err(io::Error::other(format!(
                "recording disk free space {available} bytes is below configured minimum {min_free_bytes} bytes"
            )));
        }
        Ok(())
    }

    fn active_recording_paths(&self) -> HashSet<PathBuf> {
        self.recordings
            .iter()
            .map(|entry| entry.value().session.info.wav_path.clone())
            .collect()
    }

    #[allow(dead_code)]
    pub(crate) fn record_rtp_packet(
        &self,
        relay_port: u16,
        packet: RtpPacketView<'_>,
    ) -> Result<bool, MediaError> {
        let Some(recording) = self.recordings.get(&relay_port).map(|value| value.clone()) else {
            return Ok(false);
        };
        recording
            .session
            .try_record(recording.channel, packet)
            .map_err(recording_error)
    }

    #[cfg(test)]
    pub(crate) fn flush_recording_for_test(&self, relay_port: u16) -> Result<(), MediaError> {
        let Some(recording) = self.recordings.get(&relay_port).map(|value| value.clone()) else {
            return Ok(());
        };
        recording.session.flush().map_err(recording_error)
    }
}

fn normalize_rtp_port(port: u16) -> u16 {
    if port % 2 == 1 {
        port - 1
    } else {
        port
    }
}
