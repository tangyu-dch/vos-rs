use super::{
    unix_timestamp_millis, RecordingSessionInfo, RECORDING_BITS_PER_SAMPLE, RECORDING_CHANNELS,
    RECORDING_SAMPLE_RATE,
};
use std::collections::HashSet;
use std::ffi::CString;
use std::io;
use std::mem::MaybeUninit;
use std::path::{Path, PathBuf};
use std::{fs, time::Duration};

pub fn write_recording_segment_metadata(
    session: &RecordingSessionInfo,
    segment_index: u32,
    flushed_frames: u64,
) -> io::Result<()> {
    let wav_path = recording_segment_path(session, segment_index);
    let meta_path = wav_path.with_extension("json");
    let duration_secs = flushed_frames as f64 / RECORDING_SAMPLE_RATE as f64;
    let json_content = serde_json::json!({
        "call_id": session.call_id,
        "session_id": session.id,
        "segment_index": segment_index,
        "wav_path": wav_path.to_string_lossy(),
        "sample_rate": RECORDING_SAMPLE_RATE,
        "channels": RECORDING_CHANNELS,
        "bits_per_sample": RECORDING_BITS_PER_SAMPLE,
        "flushed_frames": flushed_frames,
        "duration_secs": duration_secs,
        "format": session.format_str,
        "created_at_ms": unix_timestamp_millis(),
    });
    fs::write(&meta_path, serde_json::to_string_pretty(&json_content)?)?;
    Ok(())
}

pub fn recording_file_stem(call_id: &str) -> String {
    let sanitized = call_id
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
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
    directory: &Path,
    retention_secs: u64,
    protected_paths: &HashSet<PathBuf>,
) -> io::Result<()> {
    if retention_secs == 0 {
        return Ok(());
    }

    let retention = Duration::from_secs(retention_secs);
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        if is_protected_recording(&path, protected_paths) || !entry.file_type()?.is_file() {
            continue;
        }

        let is_recording_artifact = path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("wav"));
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

fn is_protected_recording(path: &Path, protected_paths: &HashSet<PathBuf>) -> bool {
    if protected_paths.contains(path) {
        return true;
    }

    let Some(candidate_stem) = path.file_stem().and_then(|value| value.to_str()) else {
        return false;
    };
    protected_paths.iter().any(|protected| {
        protected.parent() == path.parent()
            && protected
                .file_stem()
                .and_then(|value| value.to_str())
                .is_some_and(|stem| candidate_stem.starts_with(&format!("{stem}-part-")))
    })
}

pub fn available_disk_bytes(path: &Path) -> io::Result<u64> {
    #[cfg(unix)]
    {
        let path = CString::new(path.as_os_str().to_str().unwrap_or(".")).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidInput, "recording path contains NUL")
        })?;
        let mut statistics = MaybeUninit::<libc::statvfs>::uninit();
        let result = unsafe { libc::statvfs(path.as_ptr(), statistics.as_mut_ptr()) };
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

#[cfg(test)]
mod tests {
    use super::is_protected_recording;
    use std::collections::HashSet;
    use std::path::PathBuf;

    #[test]
    fn active_base_path_protects_rotated_segments() {
        let protected = HashSet::from([PathBuf::from("/recordings/call-1.wav")]);

        assert!(is_protected_recording(
            &PathBuf::from("/recordings/call-1-part-0001.wav"),
            &protected,
        ));
        assert!(!is_protected_recording(
            &PathBuf::from("/recordings/call-10-part-0001.wav"),
            &protected,
        ));
    }
}
