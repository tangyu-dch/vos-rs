use std::collections::HashMap;

pub(super) struct DigestExpectation<'a> {
    pub(super) username: &'a str,
    pub(super) password: &'a str,
    pub(super) realm: &'a str,
    pub(super) nonce: &'a str,
    pub(super) method: &'a str,
}

impl DigestExpectation<'_> {
    pub(super) fn matches(&self, params: &HashMap<String, String>) -> bool {
        let Some(realm) = params.get("realm") else {
            return false;
        };
        let Some(_nonce) = params.get("nonce") else {
            return false;
        };
        let Some(uri) = params.get("uri") else {
            return false;
        };
        let Some(response) = params.get("response") else {
            return false;
        };

        if realm != self.realm {
            tracing::warn!(expected = %self.realm, got = %realm, "SIP Auth Realm 不匹配");
            return false;
        }

        let is_ha1 =
            self.password.len() == 32 && self.password.chars().all(|c| c.is_ascii_hexdigit());
        let ha1 = if is_ha1 {
            self.password.to_string()
        } else {
            md5_hex(&format!(
                "{}:{}:{}",
                self.username, self.realm, self.password
            ))
        };
        let ha2 = md5_hex(&format!("{}:{}", self.method, uri));
        let expected = match params.get("qop") {
            Some(qop) => {
                if qop != "auth" {
                    tracing::warn!(got = %qop, "unsupported qop");
                    return false;
                }
                let Some(nc) = params.get("nc") else {
                    return false;
                };
                let Some(cnonce) = params.get("cnonce") else {
                    return false;
                };
                md5_hex(&format!("{ha1}:{}:{nc}:{cnonce}:{qop}:{ha2}", self.nonce))
            }
            None => md5_hex(&format!("{ha1}:{}:{ha2}", self.nonce)),
        };

        let result = response.eq_ignore_ascii_case(&expected);
        if !result {
            tracing::warn!(
                username = %self.username,
                expected = %expected,
                got = %response,
                method = %self.method,
                uri = %uri,
                ha1 = %ha1,
                ha2 = %ha2,
                "SIP Digest Auth 哈希计算结果不匹配 (密码或 Realm 错误)"
            );
        }
        result
    }
}

pub fn digest_response(
    username: &str,
    password: &str,
    realm: &str,
    nonce: &str,
    method: &str,
    uri: &str,
    qop: Option<(&str, &str, &str)>,
) -> String {
    let ha1 = md5_hex(&format!("{username}:{realm}:{password}"));
    let ha2 = md5_hex(&format!("{method}:{uri}"));

    match qop {
        Some((qop, nc, cnonce)) => md5_hex(&format!("{ha1}:{nonce}:{nc}:{cnonce}:{qop}:{ha2}")),
        None => md5_hex(&format!("{ha1}:{nonce}:{ha2}")),
    }
}

fn md5_hex(value: &str) -> String {
    format!("{:x}", md5::compute(value.as_bytes()))
}

pub(crate) fn parse_digest_authorization(raw: &str) -> Option<HashMap<String, String>> {
    let raw = raw.trim();
    let params = raw.strip_prefix("Digest ")?;
    Some(parse_auth_params(params))
}

fn parse_auth_params(raw: &str) -> HashMap<String, String> {
    let mut params = HashMap::new();
    let mut cursor = raw.trim();

    while !cursor.is_empty() {
        let Some((key, rest)) = cursor.split_once('=') else {
            break;
        };
        let key = key.trim().to_ascii_lowercase();
        let rest = rest.trim_start();

        let (value, remaining) = if let Some(rest) = rest.strip_prefix('"') {
            parse_quoted_value(rest)
        } else {
            parse_token_value(rest)
        };

        if !key.is_empty() {
            params.insert(key, value);
        }

        cursor = remaining
            .trim_start()
            .strip_prefix(',')
            .unwrap_or(remaining)
            .trim_start();
    }

    params
}

fn parse_quoted_value(raw: &str) -> (String, &str) {
    let mut value = String::new();
    let mut escaped = false;

    for (index, ch) in raw.char_indices() {
        if escaped {
            value.push(ch);
            escaped = false;
            continue;
        }

        match ch {
            '\\' => escaped = true,
            '"' => return (value, &raw[index + 1..]),
            _ => value.push(ch),
        }
    }

    (value, "")
}

fn parse_token_value(raw: &str) -> (String, &str) {
    match raw.find(',') {
        Some(index) => (raw[..index].trim().to_string(), &raw[index..]),
        None => (raw.trim().to_string(), ""),
    }
}
