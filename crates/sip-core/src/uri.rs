use crate::{SipParseError, SipResult};
use std::borrow::Cow;
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SipUri<'a> {
    pub secure: bool,
    pub user: Option<Cow<'a, str>>,
    pub host: Cow<'a, str>,
    pub port: Option<u16>,
    pub params: Vec<(Cow<'a, str>, Option<Cow<'a, str>>)>,
}

impl<'a> SipUri<'a> {
    pub fn parse(raw: &'a str) -> SipResult<Self> {
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
                (Some(Cow::Borrowed(user)), host_port)
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

    pub fn into_owned(self) -> SipUri<'static> {
        SipUri {
            secure: self.secure,
            user: self.user.map(|u| Cow::Owned(u.into_owned())),
            host: Cow::Owned(self.host.into_owned()),
            port: self.port,
            params: self
                .params
                .into_iter()
                .map(|(k, v)| {
                    (
                        Cow::Owned(k.into_owned()),
                        v.map(|x| Cow::Owned(x.into_owned())),
                    )
                })
                .collect(),
        }
    }
}

impl FromStr for SipUri<'static> {
    type Err = SipParseError;

    fn from_str(raw: &str) -> SipResult<Self> {
        SipUri::parse(raw).map(|uri| uri.into_owned())
    }
}

impl fmt::Display for SipUri<'_> {
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

fn to_lowercase_cow(s: &str) -> Cow<'_, str> {
    let trimmed = s.trim();
    if trimmed.chars().any(|c| c.is_ascii_uppercase()) {
        Cow::Owned(trimmed.to_ascii_lowercase())
    } else {
        Cow::Borrowed(trimmed)
    }
}

fn parse_param(raw: &str) -> (Cow<'_, str>, Option<Cow<'_, str>>) {
    match raw.split_once('=') {
        Some((name, value)) => (to_lowercase_cow(name), Some(Cow::Borrowed(value.trim()))),
        None => (to_lowercase_cow(raw), None),
    }
}

fn parse_host_port<'a>(raw: &'a str, original: &str) -> SipResult<(Cow<'a, str>, Option<u16>)> {
    if raw.is_empty() {
        return Err(SipParseError::InvalidUri(original.to_string()));
    }

    if raw.starts_with('[') {
        let end = raw
            .find(']')
            .ok_or_else(|| SipParseError::InvalidUri(original.to_string()))?;
        let host = Cow::Borrowed(&raw[..=end]);
        let rest = &raw[end + 1..];
        let port = parse_optional_port(rest, original)?;
        return Ok((host, port));
    }

    match raw.rsplit_once(':') {
        Some((host, port)) if !host.is_empty() && port.chars().all(|c| c.is_ascii_digit()) => {
            let port = port
                .parse::<u16>()
                .map_err(|_| SipParseError::InvalidUri(original.to_string()))?;
            Ok((to_lowercase_cow(host), Some(port)))
        }
        Some(("", _)) => Err(SipParseError::InvalidUri(original.to_string())),
        _ => Ok((to_lowercase_cow(raw), None)),
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
