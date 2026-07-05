use crate::{SipParseError, SipResult};
use std::borrow::Cow;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HeaderName(Cow<'static, str>);

impl HeaderName {
    pub fn new(raw: &str) -> SipResult<Self> {
        let value = raw.trim();
        if value.is_empty() || value.bytes().any(|byte| byte.is_ascii_control()) {
            return Err(SipParseError::InvalidHeaderLine(raw.to_string()));
        }

        Ok(Self(canonical_header_name(value)))
    }

    pub fn as_str(&self) -> &str {
        match &self.0 {
            Cow::Borrowed(s) => s,
            Cow::Owned(s) => s,
        }
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

    /// Append text to the last header value (for header folding).
    pub fn fold_last(&mut self, continuation: &str) {
        if let Some((_, value)) = self.entries.last_mut() {
            let old = value.as_str();
            let mut new_val = String::with_capacity(old.len() + 1 + continuation.len());
            new_val.push_str(old);
            new_val.push(' ');
            new_val.push_str(continuation);
            *value = HeaderValue(new_val);
        }
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
        // Compact single-letter headers
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
        // Full common SIP headers (zero-copy from static strings)
        "via" => Cow::Borrowed("via"),
        "from" => Cow::Borrowed("from"),
        "to" => Cow::Borrowed("to"),
        "call-id" => Cow::Borrowed("call-id"),
        "contact" => Cow::Borrowed("contact"),
        "content-type" => Cow::Borrowed("content-type"),
        "content-length" => Cow::Borrowed("content-length"),
        "cseq" => Cow::Borrowed("cseq"),
        "max-forwards" => Cow::Borrowed("max-forwards"),
        "expires" => Cow::Borrowed("expires"),
        "authorization" => Cow::Borrowed("authorization"),
        "www-authenticate" => Cow::Borrowed("www-authenticate"),
        "proxy-authorization" => Cow::Borrowed("proxy-authorization"),
        "proxy-authenticate" => Cow::Borrowed("proxy-authenticate"),
        "record-route" => Cow::Borrowed("record-route"),
        "route" => Cow::Borrowed("route"),
        "service-route" => Cow::Borrowed("service-route"),
        "session-expires" => Cow::Borrowed("session-expires"),
        "min-se" => Cow::Borrowed("min-se"),
        "require" => Cow::Borrowed("require"),
        "supported" => Cow::Borrowed("supported"),
        "allow" => Cow::Borrowed("allow"),
        "user-agent" => Cow::Borrowed("user-agent"),
        "server" => Cow::Borrowed("server"),
        "subject" => Cow::Borrowed("subject"),
        "content-encoding" => Cow::Borrowed("content-encoding"),
        "referred-by" => Cow::Borrowed("referred-by"),
        "refer-to" => Cow::Borrowed("refer-to"),
        "event" => Cow::Borrowed("event"),
        "allow-events" => Cow::Borrowed("allow-events"),
        value => Cow::Owned(value.to_string()),
    }
}
