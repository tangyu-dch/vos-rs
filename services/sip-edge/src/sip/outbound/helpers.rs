//! 出站消息构建辅助函数：地址解析、branch 生成、header 构建与校验。

use call_core::CallerIdentity;
use sip_core::{HeaderMap, Method, SipRequest, SipUri};
use std::str::FromStr;

pub(crate) const DEFAULT_SIP_PORT: u16 = 5060;

pub fn target_addr_for(uri: &SipUri) -> String {
    format!("{}:{}", uri.host, uri.port.unwrap_or(DEFAULT_SIP_PORT))
}

/// Derives a socket target address from a raw SIP URI string (e.g. "sip:gw@10.0.0.1:5060").
/// Falls back to port 5060 if missing.
pub fn target_addr_for_str(raw_uri: &str) -> String {
    if let Ok(uri) = SipUri::from_str(raw_uri) {
        target_addr_for(&uri)
    } else {
        // Best-effort: strip the "sip:" prefix and use as-is
        let host = raw_uri
            .trim_start_matches("sip:")
            .trim_start_matches("sips:");
        if host.contains(':') {
            host.to_string()
        } else {
            format!("{host}:{DEFAULT_SIP_PORT}")
        }
    }
}

pub(crate) fn branch_for(request: &SipRequest) -> String {
    let call_id = request
        .headers
        .get("call-id")
        .map(|value| value.as_str())
        .unwrap_or("missing-call-id");
    let cseq = request
        .headers
        .get("cseq")
        .map(|value| value.as_str())
        .unwrap_or("missing-cseq");

    format!(
        "z9hG4bK-vosrs-{}-{}",
        token_fragment(call_id),
        token_fragment(cseq)
    )
}

pub(crate) fn token_fragment(value: &str) -> String {
    let token = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string();

    if token.is_empty() {
        "empty".to_string()
    } else {
        token
    }
}

pub(crate) fn header_uri(header: &str) -> Option<String> {
    let trimmed = header.trim();
    if let Some(start) = trimmed.find('<') {
        let rest = &trimmed[start + 1..];
        let end = rest.find('>')?;
        let uri = rest[..end].trim();
        if uri.starts_with("sip:") || uri.starts_with("sips:") {
            return Some(uri.to_string());
        }
    }

    let first = trimmed.split(';').next()?.trim();
    if first.starts_with("sip:") || first.starts_with("sips:") {
        Some(first.to_string())
    } else {
        None
    }
}

pub(crate) fn next_max_forwards(headers: &HeaderMap) -> u32 {
    headers
        .get("max-forwards")
        .and_then(|value| value.as_str().parse::<u32>().ok())
        .map(|value| value.saturating_sub(1))
        .unwrap_or(69)
}

pub fn is_forwardable_in_dialog_method(method: &Method) -> bool {
    matches!(
        method,
        Method::Bye | Method::Cancel | Method::Info | Method::Refer | Method::Update
    )
}

pub(crate) fn append_single_header(
    request: &mut String,
    headers: &HeaderMap,
    lookup_name: &str,
    output_name: &str,
) {
    if let Some(value) = headers.get(lookup_name) {
        request.push_str(output_name);
        request.push_str(": ");
        request.push_str(value.as_str());
        request.push_str("\r\n");
    }
}

/// Rebuilds identity headers from trusted structured data instead of forwarding raw input.
pub fn caller_identity_headers(
    inbound: &SipRequest,
    advertised_addr: &str,
    identity: &CallerIdentity,
) -> Option<String> {
    if advertised_addr.contains(['\r', '\n']) || !valid_caller_number(&identity.presented_number) {
        return None;
    }
    let tag = inbound
        .headers
        .get("from")
        .and_then(|value| header_parameter(value.as_str(), "tag"))
        .filter(|value| value.bytes().all(valid_token_byte));
    let tag = tag.map(|tag| format!(";tag={tag}")).unwrap_or_default();
    Some(format!(
        "From: <sip:{number}@{domain}>{tag}\r\nP-Asserted-Identity: <sip:{number}@{domain}>\r\n",
        number = identity.presented_number,
        domain = advertised_addr,
    ))
}

pub(crate) fn append_caller_identity_headers(
    request: &mut String,
    inbound: &SipRequest,
    advertised_addr: &str,
    identity: &CallerIdentity,
    local_from_tag: Option<&str>,
) {
    let headers = if let Some(local_tag) = local_from_tag {
        valid_caller_number(&identity.presented_number).then(|| {
            format!(
                "From: <sip:{number}@{domain}>;tag={local_tag}\r\nP-Asserted-Identity: <sip:{number}@{domain}>\r\n",
                number = identity.presented_number,
                domain = advertised_addr,
            )
        })
    } else {
        caller_identity_headers(inbound, advertised_addr, identity)
    };
    if let Some(headers) = headers {
        request.push_str(&headers);
    }
}

pub(crate) fn append_header_with_tag(
    request: &mut String,
    headers: &HeaderMap,
    lookup_name: &str,
    output_name: &str,
    tag: &str,
) {
    let Some(value) = headers.get(lookup_name) else {
        return;
    };
    let value = value.as_str();
    let lower = value.to_ascii_lowercase();
    let without_tag = if let Some(start) = lower.find(";tag=") {
        let end = value[start + 1..]
            .find(';')
            .map(|offset| start + 1 + offset)
            .unwrap_or(value.len());
        format!("{}{}", &value[..start], &value[end..])
    } else {
        value.to_string()
    };
    request.push_str(output_name);
    request.push_str(": ");
    request.push_str(&without_tag);
    request.push_str(";tag=");
    request.push_str(tag);
    request.push_str("\r\n");
}

pub(crate) fn valid_caller_number(number: &str) -> bool {
    let digits = number.strip_prefix('+').unwrap_or(number);
    !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
}

pub(crate) fn header_parameter<'a>(header: &'a str, name: &str) -> Option<&'a str> {
    header.split(';').skip(1).find_map(|parameter| {
        let (key, value) = parameter.trim().split_once('=')?;
        key.eq_ignore_ascii_case(name)
            .then_some(value.trim().trim_end_matches('>'))
    })
}

pub(crate) fn valid_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'+')
}
