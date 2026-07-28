use std::fmt;

pub type RtpResult<T> = Result<T, RtpError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RtpError {
    PacketTooShort,
    UnsupportedVersion(u8),
    InvalidCsrcCount,
    InvalidExtensionLength,
    InvalidPadding,
    PayloadTypeOutOfRange(u8),
    RtcpPacketTooShort,
    RtcpInvalidLength,
    RtcpInvalidPadding,
    RtcpCountOutOfRange(u8),
    RtcpInvalidReportLength,
    TelephoneEventPayloadTooShort,
    /// UDPTL/T.38 包格式错误，携带具体描述。
    InvalidPacketFormat(String),
}

impl fmt::Display for RtpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PacketTooShort => write!(f, "RTP packet is too short"),
            Self::UnsupportedVersion(version) => write!(f, "unsupported RTP version: {version}"),
            Self::InvalidCsrcCount => write!(f, "invalid RTP CSRC count"),
            Self::InvalidExtensionLength => write!(f, "invalid RTP extension length"),
            Self::InvalidPadding => write!(f, "invalid RTP padding"),
            Self::PayloadTypeOutOfRange(payload_type) => {
                write!(f, "RTP payload type out of range: {payload_type}")
            }
            Self::RtcpPacketTooShort => write!(f, "RTCP packet is too short"),
            Self::RtcpInvalidLength => write!(f, "invalid RTCP packet length"),
            Self::RtcpInvalidPadding => write!(f, "invalid RTCP padding"),
            Self::RtcpCountOutOfRange(count) => write!(f, "RTCP count out of range: {count}"),
            Self::RtcpInvalidReportLength => write!(f, "invalid RTCP report length"),
            Self::TelephoneEventPayloadTooShort => {
                write!(f, "RTP telephone-event payload is too short")
            }
            Self::InvalidPacketFormat(message) => {
                write!(f, "invalid packet format: {message}")
            }
        }
    }
}

impl std::error::Error for RtpError {}
