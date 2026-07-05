use std::fmt;

pub type SipResult<T> = Result<T, SipParseError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SipParseError {
    EmptyMessage,
    InvalidStartLine(String),
    InvalidStatusCode(String),
    InvalidHeaderLine(String),
    InvalidContentLength(String),
    InvalidMethod(String),
    InvalidUri(String),
}

impl fmt::Display for SipParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyMessage => write!(f, "empty SIP message"),
            Self::InvalidStartLine(line) => write!(f, "invalid SIP start line: {line}"),
            Self::InvalidStatusCode(code) => write!(f, "invalid SIP status code: {code}"),
            Self::InvalidHeaderLine(line) => write!(f, "invalid SIP header line: {line}"),
            Self::InvalidContentLength(value) => write!(f, "invalid SIP Content-Length: {value}"),
            Self::InvalidMethod(method) => write!(f, "invalid SIP method: {method}"),
            Self::InvalidUri(uri) => write!(f, "invalid SIP URI: {uri}"),
        }
    }
}

impl std::error::Error for SipParseError {}
