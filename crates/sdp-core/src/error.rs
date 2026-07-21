use std::{error::Error, fmt};

pub type SdpResult<T> = Result<T, SdpError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SdpError {
    InvalidLine(String),
    InvalidConnectionLine(String),
    InvalidMediaLine(String),
    InvalidPort(String),
    MissingAudioRtpMedia,
    MissingCompatibleAudioCodec,
    MissingConnectionAddress,
    TooLarge,
}

impl fmt::Display for SdpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLine(line) => write!(f, "invalid SDP line: {line}"),
            Self::InvalidConnectionLine(line) => {
                write!(f, "invalid SDP connection line: {line}")
            }
            Self::InvalidMediaLine(line) => write!(f, "invalid SDP media line: {line}"),
            Self::InvalidPort(port) => write!(f, "invalid SDP media port: {port}"),
            Self::MissingAudioRtpMedia => write!(f, "missing audio RTP media description"),
            Self::MissingCompatibleAudioCodec => {
                write!(
                    f,
                    "SDP codec mismatch: missing compatible audio codec (SIP 488 Not Acceptable Here)"
                )
            }
            Self::MissingConnectionAddress => write!(f, "missing SDP connection address"),
            Self::TooLarge => write!(f, "SDP session description is too large"),
        }
    }
}

impl Error for SdpError {}
