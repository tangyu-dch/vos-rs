use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

/// 解码 WAV 文件并返回 8000Hz 16-bit 单声道 PCM 采样数据
pub fn load_wav_pcm<P: AsRef<Path>>(path: P) -> io::Result<Vec<i16>> {
    let mut file = File::open(path)?;
    let mut header = [0; 12];
    file.read_exact(&mut header)?;

    if &header[0..4] != b"RIFF" || &header[8..12] != b"WAVE" {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "不是标准的 WAV/RIFF 文件格式",
        ));
    }

    let mut channels = 1_u16;
    let mut sample_rate = 8000_u32;
    let mut bits_per_sample = 16_u16;
    let mut data_offset = None;
    let mut data_size = 0_u32;

    // 循环扫描 Chunk
    loop {
        let mut chunk_header = [0; 8];
        if file.read_exact(&mut chunk_header).is_err() {
            break; // 读取结束
        }

        let chunk_id = &chunk_header[0..4];
        let chunk_size = u32::from_le_bytes([
            chunk_header[4],
            chunk_header[5],
            chunk_header[6],
            chunk_header[7],
        ]);

        if chunk_id == b"fmt " {
            let mut fmt_data = vec![0; chunk_size as usize];
            file.read_exact(&mut fmt_data)?;

            if fmt_data.len() < 16 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "fmt chunk 格式数据长度不足",
                ));
            }

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
        } else if chunk_id == b"data" {
            let current_pos = file.stream_position()?;
            data_offset = Some(current_pos);
            data_size = chunk_size;
            file.seek(SeekFrom::Current(i64::from(chunk_size)))?;
        } else {
            // 跳过不关心的 Chunk
            file.seek(SeekFrom::Current(i64::from(chunk_size)))?;
        }
    }

    let Some(offset) = data_offset else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "未找到 WAV 文件的 data 数据段",
        ));
    };

    if bits_per_sample != 16 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("仅支持 16-bit 深度采样，当前为 {bits_per_sample}-bit"),
        ));
    }

    // 重新寻址到 data 起始点
    file.seek(SeekFrom::Start(offset))?;
    let mut raw_bytes = vec![0; data_size as usize];
    file.read_exact(&mut raw_bytes)?;

    let sample_count = raw_bytes.len() / 2;
    let mut samples = Vec::with_capacity(sample_count);
    for i in 0..sample_count {
        let sample = i16::from_le_bytes([raw_bytes[i * 2], raw_bytes[i * 2 + 1]]);
        samples.push(sample);
    }

    // 1. 如果是双声道，先转换为单声道 (通过声道均值混合)
    let mut mono_samples = if channels == 2 {
        let mut mono = Vec::with_capacity(samples.len() / 2);
        for chunk in samples.chunks_exact(2) {
            let mixed = (i32::from(chunk[0]) + i32::from(chunk[1])) / 2;
            mono.push(mixed as i16);
        }
        mono
    } else if channels == 1 {
        samples
    } else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("不支持的声道数: {channels}"),
        ));
    };

    // 2. 动态采样率重采样：如果输入的 WAV 采样率不是电信级 8000Hz，使用线性插值算法自动降采样或升采样至 8000Hz
    if sample_rate != 8000 {
        let ratio = sample_rate as f64 / 8000.0;
        let target_len = (mono_samples.len() as f64 / ratio).floor() as usize;
        let mut resampled = Vec::with_capacity(target_len);
        for i in 0..target_len {
            let src_index = i as f64 * ratio;
            let index_lower = src_index.floor() as usize;
            let index_upper = (index_lower + 1).min(mono_samples.len() - 1);
            let weight = src_index - index_lower as f64;
            let val = (1.0 - weight) * f64::from(mono_samples[index_lower]) + weight * f64::from(mono_samples[index_upper]);
            resampled.push(val as i16);
        }
        mono_samples = resampled;
    }

    Ok(mono_samples)
}