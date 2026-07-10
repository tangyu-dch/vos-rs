use rtp_core::{AudioCodec, RtpPacketView};
use sdp_core::SdpError;
use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::ffi::CString;
use std::fmt;
use std::fs::File;
use std::io::{Seek, SeekFrom, Write};
use std::mem::MaybeUninit;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use std::{fs, io};
use tracing::warn;

use crate::media::config::MediaConfig;
use crate::media::relay::MediaRelayState;
use crate::media::utils::unix_timestamp_millis;

pub const RECORDING_SAMPLE_RATE: u32 = 8_000;
pub const RECORDING_CHANNELS: u16 = 2;
pub const RECORDING_BITS_PER_SAMPLE: u16 = 16;
pub const RECORDING_FLUSH_INTERVAL_FRAMES: u64 = RECORDING_SAMPLE_RATE as u64 * 2;
pub const RECORDING_WORKER_DRAIN_LIMIT: usize = 256;

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
    pub fn new(
        wav_path: PathBuf,
        min_free_bytes: u64,
        max_file_bytes: u64,
        max_duration_secs: u64,
        pool: Arc<RecordingPool>,
    ) -> Self {
        Self {
            info: Arc::new(RecordingSessionInfo {
                id: NEXT_RECORDING_SESSION_ID.fetch_add(1, Ordering::Relaxed),
                wav_path,
                min_free_bytes,
                max_file_bytes,
                max_duration_secs,
                last_disk_check_ms: AtomicU64::new(0),
            }),
            pool,
        }
    }

    pub fn try_record(
        &self,
        channel: RecordingChannel,
        packet: RtpPacketView<'_>,
    ) -> io::Result<bool> {
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
                payload: packet.payload.to_vec(),
            },
        )
    }

    #[cfg(test)]
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
    pub wav_path: PathBuf,
    pub min_free_bytes: u64,
    pub max_file_bytes: u64,
    pub max_duration_secs: u64,
    pub last_disk_check_ms: AtomicU64,
}

impl RecordingSessionInfo {
    pub fn max_file_frames(&self) -> Option<u64> {
        let duration_frames = (self.max_duration_secs > 0).then(|| {
            self.max_duration_secs
                .saturating_mul(u64::from(RECORDING_SAMPLE_RATE))
        });
        let size_frames = (self.max_file_bytes > 44)
            .then(|| (self.max_file_bytes - 44) / u64::from(RECORDING_CHANNELS * 2));
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

        let now = unix_timestamp_millis() as u64;
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

#[derive(Debug)]
pub struct RecordingPool {
    workers: Vec<RecordingWorkerHandle>,
    queue_capacity: usize,
}

#[derive(Debug)]
pub struct RecordingWorkerHandle {
    sender: std::sync::mpsc::SyncSender<RecordingCommand>,
    pending_commands: Arc<AtomicUsize>,
}

impl RecordingPool {
    pub fn new(worker_count: usize, queue_capacity: usize) -> Self {
        let worker_count = worker_count.max(1);
        let queue_capacity = queue_capacity.max(1);
        let mut workers = Vec::with_capacity(worker_count);
        for worker_index in 0..worker_count {
            let (sender, receiver) = std::sync::mpsc::sync_channel(queue_capacity);
            let pending_commands = Arc::new(AtomicUsize::new(0));
            match spawn_recording_worker(worker_index, receiver, Arc::clone(&pending_commands)) {
                Ok(()) => workers.push(RecordingWorkerHandle {
                    sender,
                    pending_commands,
                }),
                Err(error) => warn!(%error, worker_index, "failed to spawn recording worker"),
            }
        }
        Self {
            workers,
            queue_capacity,
        }
    }

    pub fn worker_count(&self) -> usize {
        self.workers.len()
    }

    pub fn queued_commands(&self) -> usize {
        self.workers
            .iter()
            .map(|worker| worker.pending_commands.load(Ordering::Relaxed))
            .sum()
    }

    pub fn total_capacity(&self) -> usize {
        self.workers.len() * self.queue_capacity
    }

    pub fn try_record(
        &self,
        session: Arc<RecordingSessionInfo>,
        packet: RecordedRtpPacket,
    ) -> io::Result<bool> {
        self.try_send(session.id, RecordingCommand::Packet { session, packet })?;
        Ok(true)
    }

    #[cfg(test)]
    pub fn flush(&self, session: Arc<RecordingSessionInfo>) -> io::Result<()> {
        let (sender, receiver) = std::sync::mpsc::channel();
        self.send(
            session.id,
            RecordingCommand::Flush {
                session,
                reply: sender,
            },
        )?;
        receiver
            .recv()
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "recording worker is stopped"))?
    }

    pub fn finish(&self, session_id: u64) {
        let _ = self.try_send(session_id, RecordingCommand::Finish { session_id });
    }

    fn try_send(&self, session_id: u64, command: RecordingCommand) -> io::Result<()> {
        let worker = self.worker(session_id)?;
        match worker.sender.try_send(command) {
            Ok(()) => {
                worker.pending_commands.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }
            Err(std::sync::mpsc::TrySendError::Full(_)) => Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "recording queue is full",
            )),
            Err(std::sync::mpsc::TrySendError::Disconnected(_)) => Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "recording worker is stopped",
            )),
        }
    }

    #[cfg(test)]
    fn send(&self, session_id: u64, command: RecordingCommand) -> io::Result<()> {
        let worker = self.worker(session_id)?;
        worker.sender.send(command).map_err(|_| {
            io::Error::new(io::ErrorKind::BrokenPipe, "recording worker is stopped")
        })?;
        worker.pending_commands.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn worker(&self, session_id: u64) -> io::Result<&RecordingWorkerHandle> {
        if self.workers.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "recording worker pool is unavailable",
            ));
        }
        let index = session_id as usize % self.workers.len();
        Ok(&self.workers[index])
    }
}

fn spawn_recording_worker(
    worker_index: usize,
    receiver: std::sync::mpsc::Receiver<RecordingCommand>,
    pending_commands: Arc<AtomicUsize>,
) -> io::Result<()> {
    std::thread::Builder::new()
        .name(format!("vos-rs-recording-{worker_index}"))
        .spawn(move || run_recording_worker(worker_index, receiver, pending_commands))
        .map(|_| ())
        .map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("failed to spawn recording worker: {error}"),
            )
        })
}

fn run_recording_worker(
    worker_index: usize,
    receiver: std::sync::mpsc::Receiver<RecordingCommand>,
    pending_commands: Arc<AtomicUsize>,
) {
    let mut recorders = HashMap::<u64, RecordingFile>::new();
    while let Ok(command) = receiver.recv() {
        pending_commands.fetch_sub(1, Ordering::Relaxed);
        handle_recording_command(command, &mut recorders);

        for _ in 0..RECORDING_WORKER_DRAIN_LIMIT {
            let Ok(command) = receiver.try_recv() else {
                break;
            };
            pending_commands.fetch_sub(1, Ordering::Relaxed);
            handle_recording_command(command, &mut recorders);
        }
    }

    for (session_id, mut recording_file) in recorders {
        if let Err(error) = recording_file.recorder.flush_recording() {
            warn!(%error, session_id, worker_index, "failed to finalize call recording");
        }
    }
}

fn handle_recording_command(
    command: RecordingCommand,
    recorders: &mut HashMap<u64, RecordingFile>,
) {
    match command {
        RecordingCommand::Packet { session, packet } => {
            if let Err(error) = session.ensure_disk_space() {
                warn!(%error, session_id = session.id, "recording disk protection stopped packet write");
                return;
            }
            let should_rotate = recorders
                .get(&session.id)
                .map(|recording_file| {
                    recording_file.recorder.would_exceed_limit(
                        packet.channel,
                        packet.timestamp,
                        packet.payload.len(),
                        session.max_file_frames(),
                    )
                })
                .unwrap_or(false);
            if should_rotate {
                if let Err(error) = rotate_recording(recorders, &session) {
                    warn!(%error, session_id = session.id, "failed to rotate call recording");
                    return;
                }
            }
            let recording_file = match recorder_for_session(recorders, &session) {
                Ok(recording_file) => recording_file,
                Err(error) => {
                    warn!(%error, session_id = session.id, "failed to open call recording");
                    return;
                }
            };
            if let Err(error) = recording_file.recorder.record(
                packet.channel,
                packet.payload_type,
                packet.timestamp,
                &packet.payload,
            ) {
                warn!(%error, session_id = session.id, "failed to write RTP packet to recording");
            }
        }
        #[cfg(test)]
        RecordingCommand::Flush { session, reply } => {
            let result = recorder_for_session(recorders, &session)
                .and_then(|recording_file| recording_file.recorder.flush_recording());
            let _ = reply.send(result);
        }
        RecordingCommand::Finish { session_id } => {
            if let Some(mut recording_file) = recorders.remove(&session_id) {
                if let Err(error) = recording_file.recorder.flush_recording() {
                    warn!(%error, session_id, "failed to finalize call recording");
                }
            }
        }
    }
}

fn recorder_for_session<'a>(
    recorders: &'a mut HashMap<u64, RecordingFile>,
    session: &RecordingSessionInfo,
) -> io::Result<&'a mut RecordingFile> {
    if let std::collections::hash_map::Entry::Vacant(entry) = recorders.entry(session.id) {
        entry.insert(RecordingFile::create(session, 0)?);
    }
    recorders
        .get_mut(&session.id)
        .ok_or_else(|| io::Error::other("recording session was not initialized"))
}

fn rotate_recording(
    recorders: &mut HashMap<u64, RecordingFile>,
    session: &RecordingSessionInfo,
) -> io::Result<()> {
    let segment_index = recorders
        .get(&session.id)
        .map(|recording_file| recording_file.segment_index + 1)
        .unwrap_or(1);
    if let Some(mut previous) = recorders.remove(&session.id) {
        previous.recorder.flush_recording()?;
    }
    recorders.insert(session.id, RecordingFile::create(session, segment_index)?);
    Ok(())
}

pub struct RecordingFile {
    pub segment_index: u32,
    pub recorder: WavCallRecorder,
}

impl RecordingFile {
    pub fn create(session: &RecordingSessionInfo, segment_index: u32) -> io::Result<Self> {
        let wav_path = recording_segment_path(session, segment_index);
        let recorder = WavCallRecorder::create(wav_path)?;
        Ok(Self {
            segment_index,
            recorder,
        })
    }
}

pub struct RecordedRtpPacket {
    pub channel: RecordingChannel,
    pub payload_type: u8,
    pub timestamp: u32,
    pub payload: Vec<u8>,
}

pub enum RecordingCommand {
    Packet {
        session: Arc<RecordingSessionInfo>,
        packet: RecordedRtpPacket,
    },
    #[cfg(test)]
    Flush {
        session: Arc<RecordingSessionInfo>,
        reply: std::sync::mpsc::Sender<io::Result<()>>,
    },
    Finish {
        session_id: u64,
    },
}

#[derive(Debug)]
pub struct WavCallRecorder {
    file: File,
    frames_written: u64,
    flushed_frames: u64,
    base_timestamps: [Option<u32>; 2],
    frames_since_flush: u64,
    interleaved_samples: Vec<i16>,
    write_buffer: Vec<u8>,
}

impl WavCallRecorder {
    pub fn create(path: PathBuf) -> io::Result<Self> {
        let mut file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)?;
        write_wav_header(&mut file, 0)?;
        Ok(Self {
            file,
            frames_written: 0,
            flushed_frames: 0,
            base_timestamps: [None, None],
            frames_since_flush: 0,
            interleaved_samples: Vec::new(),
            write_buffer: Vec::new(),
        })
    }

    pub fn record(
        &mut self,
        channel: RecordingChannel,
        payload_type: u8,
        timestamp: u32,
        payload: &[u8],
    ) -> io::Result<bool> {
        let codec = match AudioCodec::from_static_payload_type(payload_type) {
            Some(c) => c,
            None => return Ok(false),
        };
        if payload.is_empty() {
            return Ok(false);
        }

        let num_samples = payload.len();
        let start_frame = self.start_frame(channel, timestamp);
        self.ensure_frames(start_frame + num_samples as u64)?;
        if start_frame < self.flushed_frames {
            return Ok(true);
        }

        for (sample_index, &payload_byte) in payload.iter().enumerate() {
            let sample = match codec {
                AudioCodec::Pcmu => decode_pcmu(payload_byte),
                AudioCodec::Pcma => decode_pcma(payload_byte),
                _ => continue, // G722, G729, Opus not supported for recording
            };
            let frame = start_frame + sample_index as u64;
            self.set_sample(frame, channel, sample);
        }

        self.frames_since_flush += num_samples as u64;
        if self.frames_since_flush >= RECORDING_FLUSH_INTERVAL_FRAMES {
            self.flush_ready_frames(false)?;
            self.frames_since_flush = 0;
        }
        Ok(true)
    }

    pub fn would_exceed_limit(
        &self,
        channel: RecordingChannel,
        timestamp: u32,
        payload_len: usize,
        max_frames: Option<u64>,
    ) -> bool {
        let Some(max_frames) = max_frames else {
            return false;
        };
        let base = self.base_timestamps[channel.index()].unwrap_or(timestamp);
        let start_frame = u64::from(timestamp.wrapping_sub(base));
        self.frames_written > 0 && start_frame.saturating_add(payload_len as u64) > max_frames
    }

    fn start_frame(&mut self, channel: RecordingChannel, timestamp: u32) -> u64 {
        let base = self.base_timestamps[channel.index()].get_or_insert(timestamp);
        u64::from(timestamp.wrapping_sub(*base))
    }

    fn ensure_frames(&mut self, target_frames: u64) -> io::Result<()> {
        if self.frames_written >= target_frames || target_frames <= self.flushed_frames {
            return Ok(());
        }

        let buffered_frames = target_frames - self.flushed_frames;
        let samples = buffered_frames as usize * usize::from(RECORDING_CHANNELS);
        self.interleaved_samples.resize(samples, 0);
        self.frames_written = target_frames;
        Ok(())
    }

    fn set_sample(&mut self, frame: u64, channel: RecordingChannel, sample: i16) {
        let relative_frame = frame - self.flushed_frames;
        let offset = relative_frame as usize * usize::from(RECORDING_CHANNELS) + channel.index();
        if let Some(slot) = self.interleaved_samples.get_mut(offset) {
            *slot = sample;
        }
    }

    fn flush_ready_frames(&mut self, final_flush: bool) -> io::Result<()> {
        let buffered_frames = self.frames_written.saturating_sub(self.flushed_frames);
        if buffered_frames == 0 {
            if final_flush {
                self.refresh_header()?;
                self.flush()?;
            }
            return Ok(());
        }

        let frames_to_write = if final_flush {
            buffered_frames
        } else {
            buffered_frames.saturating_sub(RECORDING_FLUSH_INTERVAL_FRAMES)
        };
        if frames_to_write == 0 {
            return Ok(());
        }

        let sample_count = frames_to_write as usize * usize::from(RECORDING_CHANNELS);
        self.write_buffer.clear();
        self.write_buffer.reserve(sample_count * 2);
        for sample in self.interleaved_samples.iter().take(sample_count) {
            self.write_buffer.extend_from_slice(&sample.to_le_bytes());
        }

        self.file.seek(SeekFrom::End(0))?;
        self.file.write_all(&self.write_buffer)?;
        self.interleaved_samples.drain(..sample_count);
        self.flushed_frames += frames_to_write;
        self.refresh_header()?;
        self.flush()
    }

    fn refresh_header(&mut self) -> io::Result<()> {
        let data_bytes = u32::try_from(self.flushed_frames * u64::from(RECORDING_CHANNELS) * 2)
            .map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "WAV recording is too large")
            })?;
        self.file.seek(SeekFrom::Start(0))?;
        write_wav_header(&mut self.file, data_bytes)?;
        self.file.seek(SeekFrom::End(0))?;
        Ok(())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }

    pub fn flush_recording(&mut self) -> io::Result<()> {
        self.flush_ready_frames(true)
    }
}

pub fn write_wav_header(file: &mut File, data_bytes: u32) -> io::Result<()> {
    let byte_rate = RECORDING_SAMPLE_RATE
        * u32::from(RECORDING_CHANNELS)
        * u32::from(RECORDING_BITS_PER_SAMPLE)
        / 8;
    let block_align = RECORDING_CHANNELS * RECORDING_BITS_PER_SAMPLE / 8;
    let riff_size = 36_u32.saturating_add(data_bytes);

    file.write_all(b"RIFF")?;
    file.write_all(&riff_size.to_le_bytes())?;
    file.write_all(b"WAVE")?;
    file.write_all(b"fmt ")?;
    file.write_all(&16_u32.to_le_bytes())?;
    file.write_all(&1_u16.to_le_bytes())?;
    file.write_all(&RECORDING_CHANNELS.to_le_bytes())?;
    file.write_all(&RECORDING_SAMPLE_RATE.to_le_bytes())?;
    file.write_all(&byte_rate.to_le_bytes())?;
    file.write_all(&block_align.to_le_bytes())?;
    file.write_all(&RECORDING_BITS_PER_SAMPLE.to_le_bytes())?;
    file.write_all(b"data")?;
    file.write_all(&data_bytes.to_le_bytes())?;
    Ok(())
}

pub(crate) fn decode_pcmu(sample: u8) -> i16 {
    let sample = !sample;
    let sign = sample & 0x80;
    let exponent = (sample >> 4) & 0x07;
    let mantissa = sample & 0x0f;
    let magnitude = (((i16::from(mantissa)) << 3) + 0x84) << exponent;

    if sign != 0 {
        0x84 - magnitude
    } else {
        magnitude - 0x84
    }
}

pub(crate) fn decode_pcma(sample: u8) -> i16 {
    let sample = sample ^ 0x55;
    let sign = sample & 0x80;
    let exponent = (sample & 0x70) >> 4;
    let mantissa = sample & 0x0f;
    let magnitude = if exponent == 0 {
        (i16::from(mantissa) << 4) + 8
    } else {
        ((i16::from(mantissa) << 4) + 0x108) << (exponent - 1)
    };

    if sign != 0 {
        magnitude
    } else {
        -magnitude
    }
}

pub fn recording_file_stem(call_id: &str) -> String {
    let sanitized = call_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    format!("{}-{}", sanitized, unix_timestamp_millis())
}

pub fn recording_segment_path(session: &RecordingSessionInfo, segment_index: u32) -> PathBuf {
    if segment_index == 0 {
        return session.wav_path.clone();
    }

    let directory = session.wav_path.parent().unwrap_or_else(|| Path::new("."));
    let stem = session
        .wav_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("recording");
    let segment_stem = format!("{stem}-part-{segment_index:04}");
    directory.join(format!("{segment_stem}.wav"))
}

pub fn cleanup_expired_recordings(
    dir: &Path,
    retention_secs: u64,
    protected_paths: &HashSet<PathBuf>,
) -> io::Result<()> {
    if retention_secs == 0 {
        return Ok(());
    }

    let retention = Duration::from_secs(retention_secs);
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if protected_paths.contains(&path) || !entry.file_type()?.is_file() {
            continue;
        }

        let is_recording_artifact = path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(|extension| extension.eq_ignore_ascii_case("wav"))
            .unwrap_or(false);
        if !is_recording_artifact {
            continue;
        }

        let is_expired = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .and_then(|modified| modified.elapsed().map_err(io::Error::other))
            .map(|age| age >= retention)
            .unwrap_or(false);
        if is_expired {
            fs::remove_file(path)?;
        }
    }
    Ok(())
}

pub fn available_disk_bytes(path: &Path) -> io::Result<u64> {
    #[cfg(unix)]
    {
        let path_str = CString::new(path.as_os_str().to_str().unwrap_or(".")).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidInput, "recording path contains NUL")
        })?;
        let mut statistics = MaybeUninit::<libc::statvfs>::uninit();
        let result = unsafe { libc::statvfs(path_str.as_ptr(), statistics.as_mut_ptr()) };
        if result != 0 {
            return Err(io::Error::last_os_error());
        }

        let statistics = unsafe { statistics.assume_init() };
        let block_size = u128::from(if statistics.f_frsize == 0 {
            statistics.f_bsize
        } else {
            statistics.f_frsize
        });
        let available = block_size * u128::from(statistics.f_bavail);
        Ok(available.min(u128::from(u64::MAX)) as u64)
    }

    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(u64::MAX)
    }
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
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidUtf8 => write!(f, "SDP body is not valid UTF-8"),
            Self::InvalidEndpoint(endpoint) => write!(f, "invalid RTP endpoint: {endpoint}"),
            Self::PortRangeExhausted { port_min, port_max } => {
                write!(f, "RTP port range exhausted: {port_min}-{port_max}")
            }
            Self::Recording(error) => write!(f, "recording error: {error}"),
            Self::RecordingQueueFull => write!(f, "recording queue is full"),
            Self::Sdp(error) => write!(f, "{error}"),
            Self::Io(error) => write!(f, "media IO error: {error}"),
        }
    }
}

impl Error for MediaError {}

impl From<SdpError> for MediaError {
    fn from(error: SdpError) -> Self {
        Self::Sdp(error)
    }
}

// Implement recording-specific actions on MediaRelayState
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

        let rtp_port_for = |port: u16| {
            if port % 2 == 1 {
                Some(port - 1)
            } else {
                Some(port)
            }
        };

        let caller_relay_port = rtp_port_for(caller_relay_port).unwrap_or(caller_relay_port);
        let gateway_relay_port = rtp_port_for(gateway_relay_port).unwrap_or(gateway_relay_port);
        self.ensure_recording_dir(&config.recording_dir)
            .map_err(recording_error)?;
        self.enforce_recording_storage_policy(
            &config.recording_dir,
            config.recording_retention_secs,
            config.recording_min_free_bytes,
        )
        .map_err(recording_error)?;

        let stem = recording_file_stem(call_id);
        let wav_path = config.recording_dir.join(format!("{stem}.wav"));
        let session = Arc::new(RecordingSession::new(
            wav_path.clone(),
            config.recording_min_free_bytes,
            config.recording_max_file_bytes,
            config.recording_max_duration_secs,
            Arc::clone(&self.recording_pool),
        ));

        self.recordings.insert(
            caller_relay_port,
            RecordingLeg {
                session: session.clone(),
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

        Ok(Some(wav_path))
    }

    fn ensure_recording_dir(&self, dir: &Path) -> io::Result<()> {
        let should_create = {
            let mut inner = self
                .state
                .lock()
                .map_err(|_| io::Error::other("media relay lock poisoned"))?;
            inner.recording_dirs.insert(dir.to_path_buf())
        };
        if !should_create {
            return Ok(());
        }

        if let Err(error) = fs::create_dir_all(dir) {
            let mut inner = self
                .state
                .lock()
                .map_err(|_| io::Error::other("media relay lock poisoned"))?;
            inner.recording_dirs.remove(dir);
            return Err(error);
        }

        Ok(())
    }

    fn enforce_recording_storage_policy(
        &self,
        dir: &Path,
        retention_secs: u64,
        min_free_bytes: u64,
    ) -> io::Result<()> {
        let protected_paths = self.active_recording_paths();
        cleanup_expired_recordings(dir, retention_secs, &protected_paths)?;

        if min_free_bytes == 0 {
            return Ok(());
        }

        let available = available_disk_bytes(dir)?;
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
            .flat_map(|entry| {
                let info = &entry.value().session.info;
                [info.wav_path.clone()]
            })
            .collect()
    }

    pub(crate) fn record_rtp_packet(
        &self,
        relay_port: u16,
        packet: RtpPacketView<'_>,
    ) -> Result<bool, MediaError> {
        let recording = self.recordings.get(&relay_port).map(|v| v.clone());
        let Some(recording) = recording else {
            return Ok(false);
        };

        recording
            .session
            .try_record(recording.channel, packet)
            .map_err(recording_error)
    }

    #[cfg(test)]
    pub(crate) fn flush_recording_for_test(&self, relay_port: u16) -> Result<(), MediaError> {
        let recording = self.recordings.get(&relay_port).map(|v| v.clone());
        let Some(recording) = recording else {
            return Ok(());
        };

        recording.session.flush().map_err(recording_error)
    }
}
