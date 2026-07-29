use media_core::live_transcode::LiveTranscoder;
use media_core::recording::{
    decode_pcma, decode_pcmu, RecordingChannel, RecordingWriter, RecordingWriterFactory,
    RECORDING_BITS_PER_SAMPLE, RECORDING_CHANNELS, RECORDING_SAMPLE_RATE,
};
use rtp_core::AudioCodec;
use std::fs::{self, File};
use std::io::{self, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

const DIRECT_HEADER_BYTES: usize = 4_096;
const DIRECT_BLOCK_FRAMES: u64 = 1_024;

#[derive(Debug, Default)]
pub(super) struct DirectIoWavWriterFactory;

impl RecordingWriterFactory for DirectIoWavWriterFactory {
    fn create(&self, path: &Path) -> io::Result<Box<dyn RecordingWriter>> {
        Ok(Box::new(DirectIoWavWriter::create(path.to_path_buf())?))
    }

    fn header_bytes(&self) -> u64 {
        DIRECT_HEADER_BYTES as u64
    }
}

struct DirectIoWavWriter {
    file: File,
    frames_written: u64,
    flushed_frames: u64,
    base_timestamps: [Option<u32>; 2],
    frames_since_flush: u64,
    interleaved_samples: Vec<i16>,
    write_buffer: Vec<u8>,
    opus_to_pcma: [Option<LiveTranscoder>; 2],
}

impl DirectIoWavWriter {
    fn create(path: PathBuf) -> io::Result<Self> {
        let mut file = open_direct_io_file(&path)?;
        write_direct_wav_header(&mut file, 0)?;
        Ok(Self {
            file,
            frames_written: 0,
            flushed_frames: 0,
            base_timestamps: [None, None],
            frames_since_flush: 0,
            interleaved_samples: Vec::new(),
            write_buffer: Vec::new(),
            opus_to_pcma: [None, None],
        })
    }

    fn start_frame(&mut self, channel: RecordingChannel, codec: AudioCodec, timestamp: u32) -> u64 {
        let base = self.base_timestamps[channel.index()].get_or_insert(timestamp);
        recording_frame(timestamp.wrapping_sub(*base), codec)
    }

    fn ensure_frames(&mut self, target_frames: u64) {
        if self.frames_written >= target_frames || target_frames <= self.flushed_frames {
            return;
        }
        let buffered_frames = target_frames - self.flushed_frames;
        self.interleaved_samples.resize(
            buffered_frames as usize * usize::from(RECORDING_CHANNELS),
            0,
        );
        self.frames_written = target_frames;
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
                self.file.flush()?;
            }
            return Ok(());
        }

        let frames_to_write = if final_flush {
            self.pad_final_block(buffered_frames)
        } else {
            buffered_frames / DIRECT_BLOCK_FRAMES * DIRECT_BLOCK_FRAMES
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
        self.flushed_frames += buffered_frames;
        self.refresh_header()?;
        self.file.flush()
    }

    fn pad_final_block(&mut self, buffered_frames: u64) -> u64 {
        let remainder = buffered_frames % DIRECT_BLOCK_FRAMES;
        if remainder == 0 {
            return buffered_frames;
        }
        let padding = DIRECT_BLOCK_FRAMES - remainder;
        self.interleaved_samples.resize(
            self.interleaved_samples.len() + padding as usize * usize::from(RECORDING_CHANNELS),
            0,
        );
        buffered_frames + padding
    }

    fn refresh_header(&mut self) -> io::Result<()> {
        let data_bytes = u32::try_from(self.flushed_frames * u64::from(RECORDING_CHANNELS) * 2)
            .map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "WAV recording is too large")
            })?;
        self.file.seek(SeekFrom::Start(0))?;
        write_direct_wav_header(&mut self.file, data_bytes)?;
        self.file.seek(SeekFrom::End(0))?;
        Ok(())
    }
}

impl RecordingWriter for DirectIoWavWriter {
    fn record(
        &mut self,
        channel: RecordingChannel,
        codec: AudioCodec,
        timestamp: u32,
        payload: &[u8],
    ) -> io::Result<bool> {
        if payload.is_empty() {
            return Ok(false);
        }
        let start_frame = self.start_frame(channel, codec, timestamp);
        let transcoded_payload;
        let (recording_codec, recording_payload) = match codec {
            AudioCodec::Pcma | AudioCodec::Pcmu => (codec, payload),
            AudioCodec::Opus => {
                let transcoder = &mut self.opus_to_pcma[channel.index()];
                if transcoder.is_none() {
                    *transcoder = Some(
                        LiveTranscoder::new(AudioCodec::Opus, AudioCodec::Pcma)
                            .map_err(io::Error::other)?,
                    );
                }
                transcoded_payload = transcoder
                    .as_mut()
                    .ok_or_else(|| io::Error::other("Opus recording transcoder unavailable"))?
                    .transcode(payload)
                    .map_err(io::Error::other)?;
                if transcoded_payload.is_empty() {
                    return Ok(true);
                }
                (AudioCodec::Pcma, transcoded_payload.as_slice())
            }
            AudioCodec::G722 | AudioCodec::G729 => return Ok(false),
        };
        self.ensure_frames(start_frame + recording_payload.len() as u64);
        if start_frame < self.flushed_frames {
            return Ok(true);
        }
        for (sample_index, &payload_byte) in recording_payload.iter().enumerate() {
            let sample = match recording_codec {
                AudioCodec::Pcmu => decode_pcmu(payload_byte),
                AudioCodec::Pcma => decode_pcma(payload_byte),
                _ => continue,
            };
            self.set_sample(start_frame + sample_index as u64, channel, sample);
        }
        self.frames_since_flush += recording_payload.len() as u64;
        if self.frames_since_flush >= DIRECT_BLOCK_FRAMES {
            self.flush_ready_frames(false)?;
            self.frames_since_flush = 0;
        }
        Ok(true)
    }

    fn would_exceed_limit(
        &self,
        channel: RecordingChannel,
        codec: AudioCodec,
        timestamp: u32,
        payload_len: usize,
        max_frames: Option<u64>,
    ) -> bool {
        let Some(max_frames) = max_frames else {
            return false;
        };
        let base = self.base_timestamps[channel.index()].unwrap_or(timestamp);
        let start_frame = recording_frame(timestamp.wrapping_sub(base), codec);
        self.frames_written > 0
            && (start_frame.saturating_add(payload_len as u64) > max_frames
                || self.frames_written.saturating_add(payload_len as u64) > max_frames)
    }

    fn flush_recording(&mut self) -> io::Result<()> {
        self.flush_ready_frames(true)
    }

    fn flushed_frames(&self) -> u64 {
        self.flushed_frames
    }
}

fn recording_frame(timestamp_delta: u32, codec: AudioCodec) -> u64 {
    u64::from(timestamp_delta).saturating_mul(u64::from(RECORDING_SAMPLE_RATE))
        / u64::from(codec.clock_rate())
}

fn write_direct_wav_header(file: &mut File, data_bytes: u32) -> io::Result<()> {
    let byte_rate = RECORDING_SAMPLE_RATE
        * u32::from(RECORDING_CHANNELS)
        * u32::from(RECORDING_BITS_PER_SAMPLE)
        / 8;
    let block_align = RECORDING_CHANNELS * RECORDING_BITS_PER_SAMPLE / 8;
    let mut header = [0_u8; DIRECT_HEADER_BYTES];
    header[0..4].copy_from_slice(b"RIFF");
    let riff_size = (DIRECT_HEADER_BYTES as u32)
        .saturating_sub(8)
        .saturating_add(data_bytes);
    header[4..8].copy_from_slice(&riff_size.to_le_bytes());
    header[8..12].copy_from_slice(b"WAVE");
    header[12..16].copy_from_slice(b"fmt ");
    header[16..20].copy_from_slice(&16_u32.to_le_bytes());
    header[20..22].copy_from_slice(&1_u16.to_le_bytes());
    header[22..24].copy_from_slice(&RECORDING_CHANNELS.to_le_bytes());
    header[24..28].copy_from_slice(&RECORDING_SAMPLE_RATE.to_le_bytes());
    header[28..32].copy_from_slice(&byte_rate.to_le_bytes());
    header[32..34].copy_from_slice(&block_align.to_le_bytes());
    header[34..36].copy_from_slice(&RECORDING_BITS_PER_SAMPLE.to_le_bytes());
    header[36..40].copy_from_slice(b"JUNK");
    header[40..44].copy_from_slice(&((DIRECT_HEADER_BYTES - 52) as u32).to_le_bytes());
    header[DIRECT_HEADER_BYTES - 8..DIRECT_HEADER_BYTES - 4].copy_from_slice(b"data");
    header[DIRECT_HEADER_BYTES - 4..].copy_from_slice(&data_bytes.to_le_bytes());
    file.write_all(&header)
}

#[cfg(target_os = "linux")]
fn open_direct_io_file(path: &Path) -> io::Result<File> {
    // Recording runs on dedicated workers. Buffered I/O avoids O_DIRECT's
    // platform-specific memory-alignment contract and keeps writes reliable.
    fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
}

#[cfg(target_os = "macos")]
fn open_direct_io_file(path: &Path) -> io::Result<File> {
    use std::os::unix::io::AsRawFd;
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)?;
    unsafe {
        let _ = libc::fcntl(file.as_raw_fd(), libc::F_NOCACHE, 1);
    }
    Ok(file)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn open_direct_io_file(path: &Path) -> io::Result<File> {
    fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
}
