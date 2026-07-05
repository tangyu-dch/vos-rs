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
                write!(f, "missing compatible audio codec in SDP")
            }
            Self::MissingConnectionAddress => write!(f, "missing SDP connection address"),
        }
    }
}

impl Error for SdpError {}
