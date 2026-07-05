use crate::{SipParseError, SipResult};
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SipUri {
    pub secure: bool,
    pub user: Option<String>,
    pub host: String,
    pub port: Option<u16>,
    pub params: Vec<(String, Option<String>)>,
}

impl FromStr for SipUri {
    type Err = SipParseError;

    fn from_str(raw: &str) -> SipResult<Self> {
        let (secure, rest) = if let Some(rest) = raw.strip_prefix("sip:") {
            (false, rest)
        } else if let Some(rest) = raw.strip_prefix("sips:") {
            (true, rest)
        } else {
            return Err(SipParseError::InvalidUri(raw.to_string()));
        };

        let mut sections = rest.split(';');
        let authority = sections
            .next()
            .filter(|part| !part.is_empty())
            .ok_or_else(|| SipParseError::InvalidUri(raw.to_string()))?;

        let params = sections
            .filter(|part| !part.is_empty())
            .map(parse_param)
            .collect::<Vec<_>>();

        let (user, host_port) = match authority.rsplit_once('@') {
            Some((user, host_port)) if !user.is_empty() && !host_port.is_empty() => {
                (Some(user.to_string()), host_port)
            }
            Some(_) => return Err(SipParseError::InvalidUri(raw.to_string())),
            None => (None, authority),
        };

        let (host, port) = parse_host_port(host_port, raw)?;

        Ok(Self {
            secure,
            user,
            host,
            port,
            params,
        })
    }
}

impl fmt::Display for SipUri {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(if self.secure { "sips:" } else { "sip:" })?;

        if let Some(user) = &self.user {
            write!(f, "{user}@")?;
        }

        f.write_str(&self.host)?;

        if let Some(port) = self.port {
            write!(f, ":{port}")?;
        }

        for (name, value) in &self.params {
            write!(f, ";{name}")?;
            if let Some(value) = value {
                write!(f, "={value}")?;
            }
        }

        Ok(())
    }
}

fn parse_param(raw: &str) -> (String, Option<String>) {
    match raw.split_once('=') {
        Some((name, value)) => (
            name.trim().to_ascii_lowercase(),
            Some(value.trim().to_string()),
        ),
        None => (raw.trim().to_ascii_lowercase(), None),
    }
}

fn parse_host_port(raw: &str, original: &str) -> SipResult<(String, Option<u16>)> {
    if raw.is_empty() {
        return Err(SipParseError::InvalidUri(original.to_string()));
    }

    if raw.starts_with('[') {
        let end = raw
            .find(']')
            .ok_or_else(|| SipParseError::InvalidUri(original.to_string()))?;
        let host = raw[..=end].to_string();
        let rest = &raw[end + 1..];
        let port = parse_optional_port(rest, original)?;
        return Ok((host, port));
    }

    match raw.rsplit_once(':') {
        Some((host, port)) if !host.is_empty() && port.chars().all(|c| c.is_ascii_digit()) => {
            let port = port
                .parse::<u16>()
                .map_err(|_| SipParseError::InvalidUri(original.to_string()))?;
            Ok((host.to_ascii_lowercase(), Some(port)))
        }
        Some(("", _)) => Err(SipParseError::InvalidUri(original.to_string())),
        _ => Ok((raw.to_ascii_lowercase(), None)),
    }
}

fn parse_optional_port(raw: &str, original: &str) -> SipResult<Option<u16>> {
    if raw.is_empty() {
        return Ok(None);
    }

    let port = raw
        .strip_prefix(':')
        .filter(|port| !port.is_empty())
        .ok_or_else(|| SipParseError::InvalidUri(original.to_string()))?;

    port.parse::<u16>()
        .map(Some)
        .map_err(|_| SipParseError::InvalidUri(original.to_string()))
}
