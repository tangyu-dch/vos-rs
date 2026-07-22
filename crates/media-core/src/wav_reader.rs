//! PCM WAV loading and conversion to 8 kHz mono audio.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

pub fn load_wav_pcm<P: AsRef<Path>>(path: P) -> io::Result<Vec<i16>> {
    let mut file = File::open(path)?;
    let file_len = file.metadata()?.len();
    let mut header = [0; 12];
    file.read_exact(&mut header)?;

    if &header[0..4] != b"RIFF" || &header[8..12] != b"WAVE" {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "不是标准的 WAV/RIFF 文件格式",
        ));
    }

    let mut channels = 1_u16;
    let mut sample_rate = 8_000_u32;
    let mut bits_per_sample = 16_u16;
    let mut found_format = false;
    let mut data_offset = None;
    let mut data_size = 0_u32;

    loop {
        let mut chunk_header = [0; 8];
        if file.read_exact(&mut chunk_header).is_err() {
            break;
        }

        let chunk_id = &chunk_header[0..4];
        let chunk_size = u32::from_le_bytes([
            chunk_header[4],
            chunk_header[5],
            chunk_header[6],
            chunk_header[7],
        ]);

        if chunk_id == b"fmt " {
            if !(16..=1_048_576).contains(&chunk_size) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "WAV fmt chunk 大小无效",
                ));
            }
            let mut fmt_data = vec![0; chunk_size as usize];
            file.read_exact(&mut fmt_data)?;
            skip_chunk_padding(&mut file, chunk_size)?;

            let format_tag = u16::from_le_bytes([fmt_data[0], fmt_data[1]]);
            if format_tag != 1 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "仅支持未压缩的 LPCM 格式 WAV 文件",
                ));
            }

            channels = u16::from_le_bytes([fmt_data[2], fmt_data[3]]);
            sample_rate = u32::from_le_bytes([fmt_data[4], fmt_data[5], fmt_data[6], fmt_data[7]]);
            bits_per_sample = u16::from_le_bytes([fmt_data[14], fmt_data[15]]);
            found_format = true;
        } else if chunk_id == b"data" {
            data_offset = Some(file.stream_position()?);
            data_size = chunk_size;
            break;
        } else {
            skip_chunk(&mut file, chunk_size)?;
        }
    }

    let Some(offset) = data_offset else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "未找到 WAV 文件的 data 数据段",
        ));
    };

    if !found_format || sample_rate == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "WAV 缺少有效的 fmt chunk",
        ));
    }

    if bits_per_sample != 16 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("仅支持 16-bit 深度采样，当前为 {bits_per_sample}-bit"),
        ));
    }
    if data_size % 2 != 0 || offset.saturating_add(u64::from(data_size)) > file_len {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "WAV data chunk 长度无效或文件已截断",
        ));
    }

    file.seek(SeekFrom::Start(offset))?;
    let mut raw_bytes = vec![0; data_size as usize];
    file.read_exact(&mut raw_bytes)?;

    let mut samples = Vec::with_capacity(raw_bytes.len() / 2);
    for bytes in raw_bytes.chunks_exact(2) {
        samples.push(i16::from_le_bytes([bytes[0], bytes[1]]));
    }

    let mut mono_samples = if channels == 2 {
        samples
            .chunks_exact(2)
            .map(|chunk| ((i32::from(chunk[0]) + i32::from(chunk[1])) / 2) as i16)
            .collect()
    } else if channels == 1 {
        samples
    } else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("不支持的声道数: {channels}"),
        ));
    };

    if sample_rate != 8_000 && !mono_samples.is_empty() {
        let ratio = sample_rate as f64 / 8_000.0;
        let target_len = (mono_samples.len() as f64 / ratio).floor() as usize;
        let mut resampled = Vec::with_capacity(target_len);
        for index in 0..target_len {
            let source_index = index as f64 * ratio;
            let lower = source_index.floor() as usize;
            let upper = (lower + 1).min(mono_samples.len() - 1);
            let weight = source_index - lower as f64;
            let value = (1.0 - weight) * f64::from(mono_samples[lower])
                + weight * f64::from(mono_samples[upper]);
            resampled.push(value as i16);
        }
        mono_samples = resampled;
    }

    Ok(mono_samples)
}

fn skip_chunk(file: &mut File, chunk_size: u32) -> io::Result<()> {
    file.seek(SeekFrom::Current(i64::from(chunk_size)))?;
    skip_chunk_padding(file, chunk_size)
}

fn skip_chunk_padding(file: &mut File, chunk_size: u32) -> io::Result<()> {
    if chunk_size % 2 != 0 {
        file.seek(SeekFrom::Current(1))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn loads_and_downmixes_stereo_pcm() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("media-core-reader-{suffix}.wav"));
        let mut file = File::create(&path).unwrap();
        crate::recording::write_wav_header(&mut file, 8).unwrap();
        for sample in [100_i16, 300, -100, 100] {
            file.write_all(&sample.to_le_bytes()).unwrap();
        }
        file.flush().unwrap();

        assert_eq!(load_wav_pcm(&path).unwrap(), [200, 0]);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn skips_odd_sized_chunks_with_riff_padding() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("media-core-padded-{suffix}.wav"));
        let mut file = File::create(&path).unwrap();
        file.write_all(b"RIFF").unwrap();
        file.write_all(&48_u32.to_le_bytes()).unwrap();
        file.write_all(b"WAVEfmt ").unwrap();
        file.write_all(&16_u32.to_le_bytes()).unwrap();
        file.write_all(&1_u16.to_le_bytes()).unwrap();
        file.write_all(&1_u16.to_le_bytes()).unwrap();
        file.write_all(&8_000_u32.to_le_bytes()).unwrap();
        file.write_all(&16_000_u32.to_le_bytes()).unwrap();
        file.write_all(&2_u16.to_le_bytes()).unwrap();
        file.write_all(&16_u16.to_le_bytes()).unwrap();
        file.write_all(b"JUNK").unwrap();
        file.write_all(&1_u32.to_le_bytes()).unwrap();
        file.write_all(&[7, 0]).unwrap();
        file.write_all(b"data").unwrap();
        file.write_all(&2_u32.to_le_bytes()).unwrap();
        file.write_all(&123_i16.to_le_bytes()).unwrap();
        file.flush().unwrap();

        assert_eq!(load_wav_pcm(&path).unwrap(), [123]);
        std::fs::remove_file(path).unwrap();
    }
}
