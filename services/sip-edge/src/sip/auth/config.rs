use super::decision::AuthDecision;
use super::digest::{parse_digest_authorization, DigestExpectation};
use super::{DEFAULT_NONCE, DEFAULT_REALM, NONCE_COUNTER};
use dashmap::DashMap;
use sip_core::SipRequest;
use std::collections::HashMap;
use std::sync::atomic::Ordering;

#[cfg(test)]
use cdr_core::PostgresCdrStore;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AuthConfig {
    #[serde(default)]
    pub enabled: Option<bool>,
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
            enabled: Some(false),
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
            enabled: None,
            realm: realm.into(),
            nonce: nonce.into(),
            users,
            secret_key: "test-secret-key".to_string(),
        }
    }

    pub fn is_enabled(&self) -> bool {
        if self.enabled == Some(false) {
            return false;
        }
        !self.users.is_empty()
    }

    /// 生成带指定 realm 的 WWW-Authenticate / Proxy-Authenticate 头。
    ///
    /// `realm_override` 为 Some 时使用传入的 realm（如租户域名），
    /// 为 None 时回退到配置的默认 realm。
    pub fn challenge_header_with_nonce_and_realm(
        &self,
        nonce: &str,
        realm_override: Option<&str>,
    ) -> String {
        let realm = realm_override.unwrap_or(&self.realm);
        format!(
            "Digest realm=\"{}\", nonce=\"{}\", algorithm=MD5, qop=\"auth\"",
            realm, nonce
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

    #[cfg(test)]
    pub async fn verify_request(
        &self,
        request: &SipRequest,
        db_store: Option<&PostgresCdrStore>,
        replay_cache: Option<&DashMap<String, u64>>,
    ) -> AuthDecision {
        let username = self.authorization_username(request);
        let password = if let (Some(db), Some(username)) = (db_store, username.as_deref()) {
            match db.get_user_password(username).await {
                Ok(Some(password)) => Some(password),
                _ => self.configured_password(username),
            }
        } else {
            username
                .as_deref()
                .and_then(|username| self.configured_password(username))
        };
        self.verify_request_with_password(
            request,
            password,
            self.is_enabled() || db_store.is_some(),
            replay_cache,
            None,
        )
    }

    /// 提取 Digest Authorization 中的用户名。
    pub(crate) fn authorization_username(&self, request: &SipRequest) -> Option<String> {
        let authorization = request
            .headers
            .get("authorization")
            .or_else(|| request.headers.get("proxy-authorization"));
        if let Some(auth_hdr) = authorization {
            if let Some(username) = parse_digest_authorization(auth_hdr.as_str())
                .and_then(|params| params.get("username").cloned())
            {
                return Some(username);
            }
        }
        // 当首个 REGISTER 不包含 Authorization 头部时，从 From/To 头部提取用户名用于识别 Trunk 或用户
        let header_val = request
            .headers
            .get("from")
            .or_else(|| request.headers.get("to"))?;
        let uri = crate::sip::registrar::parse_uri_from_header(header_val.as_str())?;
        uri.user.as_deref().map(|u| u.to_string())
    }

    /// 查找 config.yaml 中的静态鉴权凭据。
    pub(crate) fn configured_password(&self, username: &str) -> Option<String> {
        self.users.get(username).cloned()
    }

    /// 使用已从 Redis 取得的凭据验证 Digest，不访问数据库。
    ///
    /// `realm_override` 为 Some 时使用传入的 realm 进行验证（如租户域名），
    /// 为 None 时回退到配置的默认 realm。
    pub(crate) fn verify_request_with_password(
        &self,
        request: &SipRequest,
        password: Option<String>,
        auth_required: bool,
        replay_cache: Option<&DashMap<String, u64>>,
        realm_override: Option<&str>,
    ) -> AuthDecision {
        if !auth_required {
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
            return AuthDecision::ChallengeWithFailure;
        };

        let Some(nonce) = params.get("nonce") else {
            tracing::debug!("missing nonce in digest authorization");
            return AuthDecision::ChallengeWithFailure;
        };

        if !self.verify_dynamic_nonce(nonce, 300) {
            tracing::warn!(nonce = %nonce, secret_key_len = self.secret_key.len(), "nonce verification failed");
            return AuthDecision::ChallengeWithFailure;
        }

        // Check if nonce is in replay cache (already used)
        if let Some(cache) = replay_cache {
            let Some(cnonce) = params.get("cnonce") else {
                return AuthDecision::ChallengeWithFailure;
            };
            let Some(nc) = params.get("nc") else {
                return AuthDecision::ChallengeWithFailure;
            };
            let key = format!("{}:{}:{}", nonce, cnonce, nc);
            if cache.contains_key(&key) {
                tracing::warn!(%key, "replay attack detected");
                return AuthDecision::ChallengeWithFailure;
            }
        }

        if let Some(cache) = replay_cache {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs();

            let Some(cnonce) = params.get("cnonce") else {
                return AuthDecision::ChallengeWithFailure;
            };
            let Some(nc) = params.get("nc") else {
                return AuthDecision::ChallengeWithFailure;
            };
            let key = format!("{}:{}:{}", nonce, cnonce, nc);
            if cache.contains_key(&key) {
                tracing::warn!(%key, "replay attack detected");
                return AuthDecision::ChallengeWithFailure;
            }
            cache.insert(key, now + 300);
        }

        let Some(username) = params.get("username") else {
            return AuthDecision::ChallengeWithFailure;
        };

        let Some(password) = password else {
            return AuthDecision::ChallengeWithFailure;
        };

        let effective_realm = realm_override.unwrap_or(&self.realm);
        if let Some(req_realm) = params.get("realm") {
            if req_realm.trim() != effective_realm.trim() {
                tracing::warn!(req_realm = %req_realm, expected_realm = %effective_realm, "realm mismatch in digest header");
                return AuthDecision::ChallengeWithFailure;
            }
        }

        let expected = DigestExpectation {
            username,
            password: &password,
            realm: effective_realm,
            nonce,
            method: request.method.as_str(),
        };

        if expected.matches(&params) {
            AuthDecision::Authorized {
                username: username.clone(),
            }
        } else {
            AuthDecision::ChallengeWithFailure
        }
    }
}
