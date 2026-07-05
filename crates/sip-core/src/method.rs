use crate::{SipParseError, SipResult};
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Method {
    Register,
    Invite,
    Ack,
    Bye,
    Cancel,
    Options,
    Info,
    Update,
    Refer,
    Subscribe,
    Notify,
    Message,
    Prack,
    Other(String),
}

impl Method {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Register => "REGISTER",
            Self::Invite => "INVITE",
            Self::Ack => "ACK",
            Self::Bye => "BYE",
            Self::Cancel => "CANCEL",
            Self::Options => "OPTIONS",
            Self::Info => "INFO",
            Self::Update => "UPDATE",
            Self::Refer => "REFER",
            Self::Subscribe => "SUBSCRIBE",
            Self::Notify => "NOTIFY",
            Self::Message => "MESSAGE",
            Self::Prack => "PRACK",
            Self::Other(value) => value.as_str(),
        }
    }
}

impl FromStr for Method {
    type Err = SipParseError;

    fn from_str(raw: &str) -> SipResult<Self> {
        if raw.is_empty() || raw.bytes().any(|byte| byte.is_ascii_whitespace()) {
            return Err(SipParseError::InvalidMethod(raw.to_string()));
        }

        Ok(match raw.to_ascii_uppercase().as_str() {
            "REGISTER" => Self::Register,
            "INVITE" => Self::Invite,
            "ACK" => Self::Ack,
            "BYE" => Self::Bye,
            "CANCEL" => Self::Cancel,
            "OPTIONS" => Self::Options,
            "INFO" => Self::Info,
            "UPDATE" => Self::Update,
            "REFER" => Self::Refer,
            "SUBSCRIBE" => Self::Subscribe,
            "NOTIFY" => Self::Notify,
            "MESSAGE" => Self::Message,
            "PRACK" => Self::Prack,
            _ => Self::Other(raw.to_ascii_uppercase()),
        })
    }
}

impl fmt::Display for Method {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
