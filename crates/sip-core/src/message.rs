use crate::{HeaderMap, HeaderName, HeaderValue, Method, SipParseError, SipResult, SipUri};
use std::str::FromStr;

pub const SIP_VERSION: &str = "SIP/2.0";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartLine {
    Request {
        method: Method,
        uri: SipUri,
        version: String,
    },
    Response {
        version: String,
        status_code: u16,
        reason_phrase: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SipRequest {
    pub method: Method,
    pub uri: SipUri,
    pub version: String,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SipResponse {
    pub version: String,
    pub status_code: u16,
    pub reason_phrase: String,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SipMessage {
    Request(SipRequest),
    Response(SipResponse),
}

impl SipMessage {
    pub fn headers(&self) -> &HeaderMap {
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
}

pub fn parse_message(raw: &[u8]) -> SipResult<SipMessage> {
    let (head, body) = split_head_body(raw);
    let head = String::from_utf8_lossy(head);

    let mut lines_iter = head.lines().map(trim_trailing_cr);

    // Parse start line
    let start_line = lines_iter
        .by_ref()
        .find(|line| !line.trim().is_empty())
        .ok_or(SipParseError::EmptyMessage)?;
    let start_line = parse_start_line(start_line)?;

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
            let body = parse_body(&headers, body)?;
            Ok(SipMessage::Request(SipRequest {
                method,
                uri,
                version,
                headers,
                body,
            }))
        }
        StartLine::Response {
            version,
            status_code,
            reason_phrase,
        } => {
            let body = parse_body(&headers, body)?;
            Ok(SipMessage::Response(SipResponse {
                version,
                status_code,
                reason_phrase,
                headers,
                body,
            }))
        }
    }
}

fn parse_body(headers: &HeaderMap, body: &[u8]) -> SipResult<Vec<u8>> {
    let Some(content_length) = headers.get("content-length") else {
        return Ok(body.to_vec());
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

    Ok(body[..length].to_vec())
}

fn parse_start_line(line: &str) -> SipResult<StartLine> {
    if line.starts_with(SIP_VERSION) {
        return parse_response_start_line(line);
    }

    parse_request_start_line(line)
}

fn parse_request_start_line(line: &str) -> SipResult<StartLine> {
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
        uri: SipUri::from_str(uri)?,
        version: version.to_string(),
    })
}

fn parse_response_start_line(line: &str) -> SipResult<StartLine> {
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
        version: version.to_string(),
        status_code,
        reason_phrase: reason_phrase.trim().to_string(),
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
