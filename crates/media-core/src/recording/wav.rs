use super::{
    RecordingChannel, RecordingWriter, RECORDING_BITS_PER_SAMPLE, RECORDING_CHANNELS,
    RECORDING_FLUSH_INTERVAL_FRAMES, RECORDING_SAMPLE_RATE,
};
use rtp_core::AudioCodec;
use std::fs::{self, File};
use std::io::{self, Seek, SeekFrom, Write};
use std::path::PathBuf;

#[derive(Debug)]
pub struct WavCallRecorder {
    file: File,
    frames_written: u64,
    pub(crate) flushed_frames: u64,
    base_timestamps: [Option<u32>; 2],
    frames_since_flush: u64,
    interleaved_samples: Vec<i16>,
    write_buffer: Vec<u8>,
}

impl RecordingWriter for WavCallRecorder {
    fn record(
        &mut self,
        channel: RecordingChannel,
        payload_type: u8,
        timestamp: u32,
        payload: &[u8],
    ) -> io::Result<bool> {
        WavCallRecorder::record(self, channel, payload_type, timestamp, payload)
    }

    fn would_exceed_limit(
        &self,
        channel: RecordingChannel,
        timestamp: u32,
        payload_len: usize,
        max_frames: Option<u64>,
    ) -> bool {
        WavCallRecorder::would_exceed_limit(self, channel, timestamp, payload_len, max_frames)
    }

    fn flush_recording(&mut self) -> io::Result<()> {
        WavCallRecorder::flush_recording(self)
    }

    fn flushed_frames(&self) -> u64 {
        self.flushed_frames
    }
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
            Some(codec) => codec,
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
                _ => continue,
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
        self.frames_written > 0
            && (start_frame.saturating_add(payload_len as u64) > max_frames
                || self.frames_written.saturating_add(payload_len as u64) > max_frames)
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

pub fn decode_pcmu(sample: u8) -> i16 {
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

pub fn decode_pcma(sample: u8) -> i16 {
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
