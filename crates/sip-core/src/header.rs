use crate::{SipParseError, SipResult};
use std::borrow::Cow;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HeaderName(String);

impl HeaderName {
    pub fn new(raw: &str) -> SipResult<Self> {
        let value = raw.trim();
        if value.is_empty() || value.bytes().any(|byte| byte.is_ascii_control()) {
            return Err(SipParseError::InvalidHeaderLine(raw.to_string()));
        }

        Ok(Self(canonical_header_name_owned(value)))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for HeaderName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HeaderValue(String);

impl HeaderValue {
    pub fn new(raw: &str) -> Self {
        Self(raw.trim().to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for HeaderValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HeaderMap {
    entries: Vec<(HeaderName, HeaderValue)>,
}

impl HeaderMap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, name: HeaderName, value: HeaderValue) {
        self.entries.push((name, value));
    }

    /// Replace the first occurrence of `name` with `value`.
    /// If no existing entry exists, the new entry is appended.
    pub fn replace(&mut self, name: HeaderName, value: HeaderValue) {
        let needle = canonical_header_name(name.as_str());
        if let Some(pos) = self
            .entries
            .iter()
            .position(|(n, _)| n.as_str() == needle.as_ref())
        {
            self.entries[pos] = (name, value);
        } else {
            self.entries.push((name, value));
        }
    }

    pub fn get(&self, name: &str) -> Option<&HeaderValue> {
        let needle = canonical_header_name(name);
        self.entries
            .iter()
            .find(|(header_name, _)| header_name.as_str() == needle.as_ref())
            .map(|(_, value)| value)
    }

    pub fn get_all<'a>(&'a self, name: &str) -> impl Iterator<Item = &'a HeaderValue> {
        let needle = canonical_header_name(name);
        self.entries.iter().filter_map(move |(header_name, value)| {
            (header_name.as_str() == needle.as_ref()).then_some(value)
        })
    }

    pub fn iter(&self) -> impl Iterator<Item = &(HeaderName, HeaderValue)> {
        self.entries.iter()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

fn canonical_header_name(raw: &str) -> Cow<'static, str> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "b" => Cow::Borrowed("referred-by"),
        "c" => Cow::Borrowed("content-type"),
        "e" => Cow::Borrowed("content-encoding"),
        "f" => Cow::Borrowed("from"),
        "i" => Cow::Borrowed("call-id"),
        "k" => Cow::Borrowed("supported"),
        "l" => Cow::Borrowed("content-length"),
        "m" => Cow::Borrowed("contact"),
        "o" => Cow::Borrowed("event"),
        "r" => Cow::Borrowed("refer-to"),
        "s" => Cow::Borrowed("subject"),
        "t" => Cow::Borrowed("to"),
        "u" => Cow::Borrowed("allow-events"),
        "v" => Cow::Borrowed("via"),
        "x" => Cow::Borrowed("session-expires"),
        value => Cow::Owned(value.to_string()),
    }
}

fn canonical_header_name_owned(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "b" => "referred-by".to_string(),
        "c" => "content-type".to_string(),
        "e" => "content-encoding".to_string(),
        "f" => "from".to_string(),
        "i" => "call-id".to_string(),
        "k" => "supported".to_string(),
        "l" => "content-length".to_string(),
        "m" => "contact".to_string(),
        "o" => "event".to_string(),
        "r" => "refer-to".to_string(),
        "s" => "subject".to_string(),
        "t" => "to".to_string(),
        "u" => "allow-events".to_string(),
        "v" => "via".to_string(),
        "x" => "session-expires".to_string(),
        value => value.to_string(),
    }
}
