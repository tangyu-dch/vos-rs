use sip_core::{SipRequest, SipUri};
use std::{str::FromStr, time::SystemTime};

use super::error::RegisterError;

pub(super) enum ContactUpdate {
    Wildcard,
    Contact {
        uri: String,
        contact_expires: Option<u32>,
    },
}

pub(super) fn address_of_record(request: &SipRequest) -> Result<String, RegisterError> {
    let request_uri;
    let raw = if let Some(value) = request.headers.get("to") {
        value.as_str()
    } else {
        request_uri = request.uri.to_string();
        request_uri.as_str()
    };
    let uri = parse_uri_from_header(raw)
        .ok_or_else(|| RegisterError::InvalidAddressOfRecord(raw.trim().to_string()))?;
    canonical_aor(&uri)
}

pub(crate) fn canonical_aor(uri: &SipUri) -> Result<String, RegisterError> {
    let Some(user) = &uri.user else {
        return Err(RegisterError::InvalidAddressOfRecord(uri.to_string()));
    };

    if let Some(port) = uri.port {
        Ok(format!("sip:{user}@{}:{port}", uri.host))
    } else {
        Ok(format!("sip:{user}@{}", uri.host))
    }
}

pub(super) fn parse_contact(raw: &str) -> Result<ContactUpdate, RegisterError> {
    let value = raw.trim();
    if value == "*" {
        return Ok(ContactUpdate::Wildcard);
    }

    let (uri_raw, params) = split_contact_uri_and_params(value)
        .ok_or_else(|| RegisterError::InvalidContact(raw.to_string()))?;
    let uri = SipUri::from_str(uri_raw)
        .map_err(|_| RegisterError::InvalidContact(raw.to_string()))?
        .to_string();
    let contact_expires = contact_param(params, "expires")
        .map(parse_expires)
        .transpose()?;

    Ok(ContactUpdate::Contact {
        uri,
        contact_expires,
    })
}

pub(super) fn split_contact_uri_and_params(raw: &str) -> Option<(&str, &str)> {
    if let Some(start) = raw.find('<') {
        let end = raw[start + 1..].find('>')? + start + 1;
        return Some((&raw[start + 1..end], raw[end + 1..].trim()));
    }

    match raw.split_once(';') {
        Some((uri, params)) => Some((uri.trim(), params)),
        None => Some((raw.trim(), "")),
    }
}

pub(crate) fn parse_uri_from_header(raw: &str) -> Option<SipUri> {
    let value = raw.trim();
    let uri_raw = if let Some(start) = value.find('<') {
        let end = value[start + 1..].find('>')? + start + 1;
        &value[start + 1..end]
    } else {
        value.split(';').next().unwrap_or(value).trim()
    };

    SipUri::from_str(uri_raw).ok()
}

pub(super) fn request_expires(request: &SipRequest) -> Result<Option<u32>, RegisterError> {
    request
        .headers
        .get("expires")
        .map(|value| parse_expires(value.as_str()))
        .transpose()
}

pub(super) fn contact_param<'a>(params: &'a str, name: &str) -> Option<&'a str> {
    params
        .split(';')
        .filter_map(|param| param.trim().split_once('='))
        .find_map(|(key, value)| key.eq_ignore_ascii_case(name).then_some(value.trim()))
}

pub(super) fn parse_expires(raw: &str) -> Result<u32, RegisterError> {
    raw.trim()
        .parse::<u32>()
        .map_err(|_| RegisterError::InvalidExpires(raw.trim().to_string()))
}

pub(super) fn remaining_seconds(expires_at: SystemTime, now: SystemTime) -> Option<u32> {
    let duration = expires_at.duration_since(now).ok()?;
    let seconds = duration.as_secs().min(u64::from(u32::MAX));
    u32::try_from(seconds).ok()
}
