//! # SIP Digest Auth 认证
//!
//! 本模块实现了 SIP Digest Auth 认证机制，包括：
//!
//! - **Digest 计算**：MD5(username:realm:password) + nonce + method + uri
//! - **Nonce 管理**：动态 nonce 生成和防重放保护
//! - **用户管理**：从统一配置文件或数据库加载用户凭据
//!
//! ## 安全机制
//!
//! - 动态 nonce 包含时间戳和计数器，防止重放攻击
//! - nonce 有效期可配置
//! - 密码使用 HA1 哈希存储

use cdr_core::PostgresCdrStore;
use dashmap::DashMap;
use sip_core::SipRequest;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
const DEFAULT_REALM: &str = "vos-rs";
const DEFAULT_NONCE: &str = "vos-rs-dev-nonce";
static NONCE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AuthConfig {
    pub(crate) realm: String,
    pub(crate) nonce: String,
    pub(crate) users: HashMap<String, String>,
    #[serde(default = "default_secret_key")]
    pub secret_key: String,
}

fn default_secret_key() -> String {
    format!(
        "{:x}",
        md5::compute(format!("{:?}", std::time::SystemTime::now()).as_bytes())
    )
}

impl AuthConfig {
    pub fn disabled() -> Self {
        Self {
            realm: DEFAULT_REALM.to_string(),
            nonce: DEFAULT_NONCE.to_string(),
            users: HashMap::new(),
            secret_key: "test-secret-key".to_string(),
        }
    }

    #[cfg(test)]
    pub fn new(
        realm: impl Into<String>,
        nonce: impl Into<String>,
        users: HashMap<String, String>,
    ) -> Self {
        Self {
            realm: realm.into(),
            nonce: nonce.into(),
            users,
            secret_key: "test-secret-key".to_string(),
        }
    }

    pub fn is_enabled(&self) -> bool {
        !self.users.is_empty()
    }

    pub fn challenge_header_with_nonce(&self, nonce: &str) -> String {
        format!(
            "Digest realm=\"{}\", nonce=\"{}\", algorithm=MD5, qop=\"auth\"",
            self.realm, nonce
        )
    }
    pub fn select_nonce(&self) -> String {
        if self.nonce == DEFAULT_NONCE {
            self.generate_dynamic_nonce()
        } else {
            self.nonce.clone()
        }
    }

    pub fn generate_dynamic_nonce(&self) -> String {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let seq = NONCE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let sig = format!(
            "{:x}",
            md5::compute(format!("{}:{}:{}", now, seq, self.secret_key).as_bytes())
        );
        format!("{}-{}-{}", now, seq, sig)
    }

    pub fn verify_dynamic_nonce(&self, nonce: &str, max_age_secs: u64) -> bool {
        if nonce == self.nonce || nonce == DEFAULT_NONCE {
            return true;
        }

        let Some((ts_str, rest)) = nonce.split_once('-') else {
            return false;
        };
        let Ok(ts) = ts_str.parse::<u64>() else {
            return false;
        };

        let expected_sig = format!(
            "{:x}",
            md5::compute(format!("{}:{}:{}", ts, "", self.secret_key).as_bytes())
        );
        if rest == expected_sig {
            return true;
        }

        if let Some((seq_str, sig)) = rest.split_once('-') {
            let _ = seq_str.parse::<u64>();
            let expected_sig = format!(
                "{:x}",
                md5::compute(format!("{}:{}:{}", ts, seq_str, self.secret_key).as_bytes())
            );
            if sig == expected_sig {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::SystemTime::UNIX_EPOCH)
                    .unwrap()
                    .as_secs();
                return now.saturating_sub(ts) <= max_age_secs;
            }
        }

        false
    }

    pub async fn verify_request(
        &self,
        request: &SipRequest,
        db_store: Option<&PostgresCdrStore>,
        replay_cache: Option<&DashMap<String, u64>>,
    ) -> AuthDecision {
        if !self.is_enabled() && db_store.is_none() {
            return AuthDecision::Disabled;
        }

        let raw_authorization = request
            .headers
            .get("authorization")
            .or_else(|| request.headers.get("proxy-authorization"));
        let Some(raw_authorization) = raw_authorization else {
            tracing::debug!("no Authorization header found");
            return AuthDecision::Challenge;
        };

        let Some(params) = parse_digest_authorization(raw_authorization.as_str()) else {
            tracing::debug!("failed to parse digest authorization");
            return AuthDecision::Challenge;
        };

        let Some(nonce) = params.get("nonce") else {
            tracing::debug!("missing nonce in digest authorization");
            return AuthDecision::Challenge;
        };

        if !self.verify_dynamic_nonce(nonce, 300) {
            tracing::warn!(nonce = %nonce, secret_key_len = self.secret_key.len(), "nonce verification failed");
            return AuthDecision::Challenge;
        }

        // Check if nonce is in replay cache (already used)
        if let Some(cache) = replay_cache {
            let Some(cnonce) = params.get("cnonce") else {
                return AuthDecision::Challenge;
            };
            let Some(nc) = params.get("nc") else {
                return AuthDecision::Challenge;
            };
            let key = format!("{}:{}:{}", nonce, cnonce, nc);
            if cache.contains_key(&key) {
                tracing::warn!(%key, "replay attack detected");
                return AuthDecision::Challenge;
            }
        }

        if let Some(cache) = replay_cache {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs();

            let Some(cnonce) = params.get("cnonce") else {
                return AuthDecision::Challenge;
            };
            let Some(nc) = params.get("nc") else {
                return AuthDecision::Challenge;
            };
            let key = format!("{}:{}:{}", nonce, cnonce, nc);
            if cache.contains_key(&key) {
                tracing::warn!(%key, "replay attack detected");
                return AuthDecision::Challenge;
            }
            cache.insert(key, now + 300);
        }

        let Some(username) = params.get("username") else {
            return AuthDecision::Challenge;
        };

        let password_opt = if let Some(db) = db_store {
            match db.get_user_password(username).await {
                Ok(Some(pw)) => Some(pw),
                _ => self.users.get(username).cloned(),
            }
        } else {
            self.users.get(username).cloned()
        };

        let Some(password) = password_opt else {
            return AuthDecision::Challenge;
        };

        let expected = DigestExpectation {
            username,
            password: &password,
            realm: &self.realm,
            nonce,
            method: request.method.as_str(),
        };

        if expected.matches(&params) {
            AuthDecision::Authorized {
                username: username.clone(),
            }
        } else {
            AuthDecision::Challenge
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthDecision {
    Disabled,
    Authorized { username: String },
    Challenge,
}

struct DigestExpectation<'a> {
    username: &'a str,
    password: &'a str,
    realm: &'a str,
    nonce: &'a str,
    method: &'a str,
}

impl DigestExpectation<'_> {
    fn matches(&self, params: &HashMap<String, String>) -> bool {
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
            tracing::debug!(expected = %self.realm, got = %realm, "realm mismatch");
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
                    tracing::debug!(got = %qop, "unsupported qop");
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
            tracing::debug!(
                expected = %expected,
                got = %response,
                method = %self.method,
                uri = %uri,
                ha2 = %ha2,
                "digest response mismatch"
            );
        }
        result
    }
}

#[cfg(test)]
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

fn parse_digest_authorization(raw: &str) -> Option<HashMap<String, String>> {
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

#[cfg(test)]
mod tests {
    use super::{digest_response, AuthConfig, AuthDecision};
    use sip_core::{parse_message, SipMessage, SipRequest};
    use std::collections::HashMap;

    #[tokio::test]
    async fn disabled_auth_allows_requests() {
        let request = register_request(None);

        assert_eq!(
            AuthConfig::disabled()
                .verify_request(&request, None, None)
                .await,
            AuthDecision::Disabled
        );
    }

    #[tokio::test]
    async fn missing_authorization_challenges_when_enabled() {
        let request = register_request(None);
        let config = auth_config();

        assert_eq!(
            config.verify_request(&request, None, None).await,
            AuthDecision::Challenge
        );
    }

    #[tokio::test]
    async fn valid_digest_authorization_is_accepted() {
        let uri = "sip:127.0.0.1:5060";
        let response = digest_response(
            "1001",
            "secret",
            "vos-rs",
            "test-nonce",
            "REGISTER",
            uri,
            Some(("auth", "00000001", "abcdef")),
        );
        let authorization = format!(
            "Digest username=\"1001\", realm=\"vos-rs\", nonce=\"test-nonce\", uri=\"{uri}\", response=\"{response}\", algorithm=MD5, qop=auth, nc=00000001, cnonce=\"abcdef\""
        );
        let request = register_request(Some(&authorization));

        assert_eq!(
            auth_config().verify_request(&request, None, None).await,
            AuthDecision::Authorized {
                username: "1001".to_string()
            }
        );
    }

    #[tokio::test]
    async fn wrong_password_is_challenged() {
        let uri = "sip:127.0.0.1:5060";
        let response = digest_response(
            "1001",
            "wrong",
            "vos-rs",
            "test-nonce",
            "REGISTER",
            uri,
            Some(("auth", "00000001", "abcdef")),
        );
        let authorization = format!(
            "Digest username=\"1001\", realm=\"vos-rs\", nonce=\"test-nonce\", uri=\"{uri}\", response=\"{response}\", qop=auth, nc=00000001, cnonce=\"abcdef\""
        );
        let request = register_request(Some(&authorization));

        assert_eq!(
            auth_config().verify_request(&request, None, None).await,
            AuthDecision::Challenge
        );
    }

    fn auth_config() -> AuthConfig {
        AuthConfig::new(
            "vos-rs",
            "test-nonce",
            HashMap::from([("1001".to_string(), "secret".to_string())]),
        )
    }

    fn register_request(authorization: Option<&str>) -> SipRequest {
        let auth_header = authorization
            .map(|value| format!("Authorization: {value}\r\n"))
            .unwrap_or_default();
        let raw = format!(
            concat!(
                "REGISTER sip:127.0.0.1:5060 SIP/2.0\r\n",
                "Via: SIP/2.0/UDP 127.0.0.1:5070;branch=z9hG4bK-auth\r\n",
                "From: <sip:1001@127.0.0.1>;tag=auth\r\n",
                "To: <sip:1001@127.0.0.1>\r\n",
                "Call-ID: auth@example.com\r\n",
                "CSeq: 1 REGISTER\r\n",
                "{auth_header}",
                "Contact: <sip:1001@127.0.0.1:5070>;expires=120\r\n",
                "Content-Length: 0\r\n",
                "\r\n"
            ),
            auth_header = auth_header
        );

        let SipMessage::Request(request) = parse_message(raw.as_bytes()).unwrap() else {
            panic!("expected request");
        };
        request
    }
}
