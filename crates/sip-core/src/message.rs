use crate::{HeaderMap, HeaderName, HeaderValue, Method, SipParseError, SipResult, uri::SipUri};
use std::str::FromStr;
use std::borrow::Cow;

pub const SIP_VERSION: &str = "SIP/2.0";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartLine<'a> {
    Request {
        method: Method,
        uri: SipUri<'a>,
        version: Cow<'a, str>,
    },
    Response {
        version: Cow<'a, str>,
        status_code: u16,
        reason_phrase: Cow<'a, str>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SipRequest<'a> {
    pub method: Method,
    pub uri: SipUri<'a>,
    pub version: Cow<'a, str>,
    pub headers: HeaderMap<'a>,
    pub body: Cow<'a, [u8]>,
}

impl<'a> SipRequest<'a> {
    pub fn into_owned(self) -> SipRequest<'static> {
        SipRequest {
            method: self.method,
            uri: self.uri.into_owned(),
            version: Cow::Owned(self.version.into_owned()),
            headers: self.headers.into_owned(),
            body: Cow::Owned(self.body.into_owned()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SipResponse<'a> {
    pub version: Cow<'a, str>,
    pub status_code: u16,
    pub reason_phrase: Cow<'a, str>,
    pub headers: HeaderMap<'a>,
    pub body: Cow<'a, [u8]>,
}

impl<'a> SipResponse<'a> {
    pub fn into_owned(self) -> SipResponse<'static> {
        SipResponse {
            version: Cow::Owned(self.version.into_owned()),
            status_code: self.status_code,
            reason_phrase: Cow::Owned(self.reason_phrase.into_owned()),
            headers: self.headers.into_owned(),
            body: Cow::Owned(self.body.into_owned()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SipMessage<'a> {
    Request(SipRequest<'a>),
    Response(SipResponse<'a>),
}

impl<'a> SipMessage<'a> {
    pub fn headers(&self) -> &HeaderMap<'a> {
        match self {
            Self::Request(request) => &request.headers,
            Self::Response(response) => &response.headers,
        }
    }

    pub fn body(&self) -> &[u8] {
        match self {
            Self::Request(request) => &request.body,
            Self::Response(response) => &response.body,
        }
    }

    pub fn into_owned(self) -> SipMessage<'static> {
        match self {
            Self::Request(req) => SipMessage::Request(req.into_owned()),
            Self::Response(resp) => SipMessage::Response(resp.into_owned()),
        }
    }
}

pub fn parse_message(raw: &[u8]) -> SipResult<SipMessage<'_>> {
    let (head_bytes, body_bytes) = split_head_body(raw);
    let head = std::str::from_utf8(head_bytes)
        .map_err(|_| SipParseError::InvalidStartLine("invalid utf-8 headers".to_string()))?;

    let mut lines_iter = head.lines().map(trim_trailing_cr);

    // Parse start line
    let start_line_str = lines_iter
        .by_ref()
        .find(|line| !line.trim().is_empty())
        .ok_or(SipParseError::EmptyMessage)?;
    let start_line = parse_start_line(start_line_str)?;

    let mut headers = HeaderMap::new();

    for line in lines_iter {
        if line.trim().is_empty() {
            continue;
        }

        // Handle header folding: line starting with SP/HTAB continues previous header value
        if line.starts_with(' ') || line.starts_with('\t') {
            headers.fold_last(line.trim());
            continue;
        }

        if let Some((name, value)) = line.split_once(':') {
            headers.insert(HeaderName::new(name)?, HeaderValue::new(value));
        } else {
            return Err(SipParseError::InvalidHeaderLine(line.to_string()));
        }
    }

    match start_line {
        StartLine::Request {
            method,
            uri,
            version,
        } => {
            let body = parse_body(&headers, body_bytes)?;
            Ok(SipMessage::Request(SipRequest {
                method,
                uri,
                version,
                headers,
                body: Cow::Borrowed(body),
            }))
        }
        StartLine::Response {
            version,
            status_code,
            reason_phrase,
        } => {
            let body = parse_body(&headers, body_bytes)?;
            Ok(SipMessage::Response(SipResponse {
                version,
                status_code,
                reason_phrase,
                headers,
                body: Cow::Borrowed(body),
            }))
        }
    }
}

fn parse_body<'a>(headers: &HeaderMap<'_>, body: &'a [u8]) -> SipResult<&'a [u8]> {
    let Some(content_length) = headers.get("content-length") else {
        return Ok(body);
    };

    let length = content_length
        .as_str()
        .parse::<usize>()
        .map_err(|_| SipParseError::InvalidContentLength(content_length.as_str().to_string()))?;

    if body.len() < length {
        return Err(SipParseError::InvalidContentLength(
            content_length.as_str().to_string(),
        ));
    }

    Ok(&body[..length])
}

fn parse_start_line(line: &str) -> SipResult<StartLine<'_>> {
    if line.starts_with(SIP_VERSION) {
        return parse_response_start_line(line);
    }

    parse_request_start_line(line)
}

fn parse_request_start_line(line: &str) -> SipResult<StartLine<'_>> {
    let mut parts = line.split_whitespace();
    let method = parts
        .next()
        .ok_or_else(|| SipParseError::InvalidStartLine(line.to_string()))?;
    let uri = parts
        .next()
        .ok_or_else(|| SipParseError::InvalidStartLine(line.to_string()))?;
    let version = parts
        .next()
        .ok_or_else(|| SipParseError::InvalidStartLine(line.to_string()))?;

    if parts.next().is_some() || version != SIP_VERSION {
        return Err(SipParseError::InvalidStartLine(line.to_string()));
    }

    Ok(StartLine::Request {
        method: Method::from_str(method)?,
        uri: SipUri::parse(uri)?,
        version: Cow::Borrowed(version),
    })
}

fn parse_response_start_line(line: &str) -> SipResult<StartLine<'_>> {
    let mut parts = line.splitn(3, ' ');
    let version = parts
        .next()
        .ok_or_else(|| SipParseError::InvalidStartLine(line.to_string()))?;
    let status_code = parts
        .next()
        .ok_or_else(|| SipParseError::InvalidStartLine(line.to_string()))?;
    let reason_phrase = parts.next().unwrap_or_default();

    if version != SIP_VERSION || status_code.len() != 3 {
        return Err(SipParseError::InvalidStartLine(line.to_string()));
    }

    let status_code = status_code
        .parse::<u16>()
        .map_err(|_| SipParseError::InvalidStatusCode(status_code.to_string()))?;

    Ok(StartLine::Response {
        version: Cow::Borrowed(version),
        status_code,
        reason_phrase: Cow::Borrowed(reason_phrase.trim()),
    })
}

fn split_head_body(raw: &[u8]) -> (&[u8], &[u8]) {
    if let Some(index) = raw.windows(4).position(|window| window == b"\r\n\r\n") {
        (&raw[..index], &raw[index + 4..])
    } else if let Some(index) = raw.windows(2).position(|window| window == b"\n\n") {
        (&raw[..index], &raw[index + 2..])
    } else {
        (raw, &[])
    }
}

fn trim_trailing_cr(line: &str) -> &str {
    line.strip_suffix('\r').unwrap_or(line)
}
