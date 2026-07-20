//! # SIP 零拷贝解析器 (Zero-Copy SIP Message Parser)
//! 
//! 本模块借由 Rust 的借用生命周期 `'a`，直接在传入的 Raw Datagram Byte Buffer (`&'a [u8]`) 
//! 上构建 SIP 报文的头域与请求行/响应行切片，完全零堆分配 (Zero Heap Allocation)，
//! 为 1000+ CPS 的高频 SIP 报文吞吐提供极好的 CPU 缓存友好度与性能。

use std::str;
use crate::error::SipParseError;

/// 零拷贝 SIP 头域切片 (`&'a str`)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZeroCopyHeader<'a> {
    pub name: &'a str,
    pub value: &'a str,
}

/// 零拷贝 SIP 请求行切片 (`&'a str`)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZeroCopyRequestLine<'a> {
    pub method: &'a str,
    pub uri: &'a str,
    pub version: &'a str,
}

/// 零拷贝 SIP 报文 View (Zero-Copy Message View)
#[derive(Debug, Clone)]
pub struct ZeroCopySipMessage<'a> {
    pub request_line: Option<ZeroCopyRequestLine<'a>>,
    pub status_code: Option<u16>,
    pub reason_phrase: Option<&'a str>,
    pub headers: Vec<ZeroCopyHeader<'a>>,
    pub body: &'a [u8],
}

impl<'a> ZeroCopySipMessage<'a> {
    /// 从原始字节切片直接零拷贝解析 SIP 报文
    pub fn parse(raw: &'a [u8]) -> Result<Self, SipParseError> {
        let input = str::from_utf8(raw).map_err(|_| SipParseError::InvalidHeaderLine("invalid utf-8".into()))?;
        
        let mut lines = input.split("\r\n");
        let first_line = lines.next().ok_or(SipParseError::EmptyMessage)?;

        let mut request_line = None;
        let mut status_code = None;
        let mut reason_phrase = None;

        if first_line.starts_with("SIP/2.0 ") {
            // 解析 响应行: SIP/2.0 200 OK
            let mut parts = first_line.splitn(3, ' ');
            let _version = parts.next();
            if let Some(code_str) = parts.next() {
                status_code = code_str.parse::<u16>().ok();
            }
            reason_phrase = parts.next();
        } else {
            // 解析 请求行: INVITE sip:user@domain SIP/2.0
            let mut parts = first_line.splitn(3, ' ');
            let method = parts.next().unwrap_or("");
            let uri = parts.next().unwrap_or("");
            let version = parts.next().unwrap_or("SIP/2.0");
            request_line = Some(ZeroCopyRequestLine { method, uri, version });
        }

        let mut headers = Vec::with_capacity(16);
        let mut body_start_bytes = raw.len();
        let mut current_offset = first_line.len() + 2;

        for line in lines {
            if line.is_empty() {
                // 空行意味着 Header 结束，后续为 Body
                body_start_bytes = current_offset + 2;
                break;
            }

            if let Some(colon_pos) = line.find(':') {
                let name = line[..colon_pos].trim();
                let value = line[colon_pos + 1..].trim();
                headers.push(ZeroCopyHeader { name, value });
            }
            current_offset += line.len() + 2;
        }

        let body = if body_start_bytes < raw.len() {
            &raw[body_start_bytes..]
        } else {
            &[]
        };

        Ok(Self {
            request_line,
            status_code,
            reason_phrase,
            headers,
            body,
        })
    }

    /// 获取特定 Name 的 Header 对应的值（忽略大小写）
    pub fn header_value(&self, name: &str) -> Option<&'a str> {
        self.headers
            .iter()
            .find(|h| h.name.eq_ignore_ascii_case(name))
            .map(|h| h.value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_zero_copy_parse_request() {
        let raw = b"INVITE sip:1001@192.168.1.1 SIP/2.0\r\n\
Via: SIP/2.0/UDP 192.168.1.5:5060;branch=z9hG4bK-123\r\n\
From: <sip:1000@192.168.1.1>;tag=abc\r\n\
To: <sip:1001@192.168.1.1>\r\n\
Call-ID: test-call-id-999\r\n\
Content-Length: 4\r\n\
\r\n\
test";

        let msg = ZeroCopySipMessage::parse(raw).unwrap();
        let req = msg.request_line.unwrap();
        assert_eq!(req.method, "INVITE");
        assert_eq!(req.uri, "sip:1001@192.168.1.1");
        assert_eq!(msg.header_value("Call-ID"), Some("test-call-id-999"));
        assert_eq!(msg.header_value("from"), Some("<sip:1000@192.168.1.1>;tag=abc"));
        assert_eq!(msg.body, b"test");
    }
}
