//! Completed call-recording output formats.

/// Post-processing format for a completed call recording.
///
/// `Wav` keeps the original WAV recording, while `Opus` and `Amr` identify
/// formats produced by the service-level FFmpeg adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordingFormat {
    /// Unmodified WAV audio.
    Wav,
    /// Opus audio.
    Opus,
    /// AMR narrowband audio.
    Amr,
}

impl RecordingFormat {
    /// Parses a case-insensitive configuration value.
    ///
    /// Unknown and empty values preserve the existing fallback to `Wav`.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Self {
        match value.to_ascii_lowercase().as_str() {
            "opus" => Self::Opus,
            "amr" => Self::Amr,
            _ => Self::Wav,
        }
    }

    /// Returns the file extension for the completed recording.
    pub fn extension(&self) -> &'static str {
        match self {
            Self::Wav => "wav",
            Self::Opus => "opus",
            Self::Amr => "amr",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::RecordingFormat;

    #[test]
    fn parses_supported_formats_case_insensitively() {
        assert_eq!(RecordingFormat::from_str("wav"), RecordingFormat::Wav);
        assert_eq!(RecordingFormat::from_str("WAV"), RecordingFormat::Wav);
        assert_eq!(RecordingFormat::from_str("opus"), RecordingFormat::Opus);
        assert_eq!(RecordingFormat::from_str("Opus"), RecordingFormat::Opus);
        assert_eq!(RecordingFormat::from_str("amr"), RecordingFormat::Amr);
        assert_eq!(RecordingFormat::from_str("AMR"), RecordingFormat::Amr);
    }

    #[test]
    fn unknown_and_empty_formats_fall_back_to_wav() {
        assert_eq!(RecordingFormat::from_str("unknown"), RecordingFormat::Wav);
        assert_eq!(RecordingFormat::from_str(""), RecordingFormat::Wav);
    }

    #[test]
    fn returns_expected_file_extensions() {
        assert_eq!(RecordingFormat::Wav.extension(), "wav");
        assert_eq!(RecordingFormat::Opus.extension(), "opus");
        assert_eq!(RecordingFormat::Amr.extension(), "amr");
    }
}
