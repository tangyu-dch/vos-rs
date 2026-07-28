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

use std::sync::atomic::AtomicU64;

mod config;
mod decision;
mod digest;

pub use config::AuthConfig;
pub use decision::AuthDecision;
pub(crate) use digest::{digest_response, parse_digest_authorization};

pub(super) const DEFAULT_REALM: &str = "vos-rs";
pub(super) const DEFAULT_NONCE: &str = "vos-rs-dev-nonce";
pub(super) static NONCE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[cfg(test)]
mod tests {
    use super::{digest_response, AuthConfig, AuthDecision};
    use sip_core::{parse_message, SipRequest};
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
            AuthDecision::ChallengeWithFailure
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

        let sip_core::SipMessageBorrow::Request(request) = parse_message(raw.as_bytes()).unwrap()
        else {
            panic!("expected request");
        };
        request.into_owned()
    }
}
