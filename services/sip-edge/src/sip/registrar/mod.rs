//! # SIP REGISTER 注册处理
//!
//! 本模块实现了 SIP REGISTER 请求的处理，包括：
//!
//! - **注册处理**：处理 REGISTER 请求，绑定 Contact 地址
//! - **注销处理**：Expires=0 时移除 Contact 绑定
//! - **查询处理**：不带 Contact 的 REGISTER 查询当前绑定
//! - **过期管理**：自动清理过期的注册绑定
//!
//! ## 注册流程
//!
//! ```text
//! REGISTER → 验证 Digest Auth → 存储 Contact 绑定 → 返回 200 OK
//! ```
//!
//! ## Contact 绑定
//!
//! 每个 AOR（Address of Record）可以有多个 Contact 绑定，
//! 用于支持多设备注册和故障转移。
//!
//! ## 配置
//!
//! | 环境变量 | 说明 | 默认值 |
//! |---------|------|--------|
//! | `sip_edge.auth.users` | 认证用户列表 | 空 |

mod error;
mod helpers;
mod store;

// `RegisterError` is the return error type of `RegistrationStore::handle_register`
// and therefore part of this module's public API, but no caller names it
// explicitly. Re-export it so external code *can* name it if needed.
#[allow(unused_imports)]
pub use error::RegisterError;
pub use store::RegistrationStore;

pub(crate) use helpers::{canonical_aor, parse_uri_from_header};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RegistrationInvalidateMsg {
    pub aor: String,
    pub action: String,
    pub timestamp: u64,
}

const DEFAULT_EXPIRES_SECONDS: u32 = 3600;
const MAX_EXPIRES_SECONDS: u32 = 86_400;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RegistrationContact {
    pub uri: String,
    pub expires: u32,
    pub received_from: String,
    pub path: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisterOutcome {
    pub aor: String,
    pub contacts: Vec<RegistrationContact>,
}

#[cfg(test)]
mod tests {
    use super::{RegisterError, RegistrationStore};
    use sip_core::{parse_message, SipRequest};
    use std::{
        net::SocketAddr,
        time::{Duration, SystemTime},
    };

    #[tokio::test]
    async fn registers_contact_and_returns_active_binding() {
        let mut store = RegistrationStore::new();
        let request = request(concat!(
            "REGISTER sip:example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-reg\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:1001@example.com>\r\n",
            "Call-ID: reg-1@example.com\r\n",
            "CSeq: 1 REGISTER\r\n",
            "Contact: <sip:1001@192.0.2.10:5070;transport=udp>;expires=60\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        ));

        let outcome = store
            .handle_register(
                &request,
                "192.0.2.10:5060".parse().unwrap(),
                SystemTime::UNIX_EPOCH,
                None,
                None,
            )
            .await
            .unwrap();

        assert_eq!(outcome.aor, "sip:1001@example.com");
        assert_eq!(outcome.contacts.len(), 1);
        assert_eq!(
            outcome.contacts[0].uri,
            "sip:1001@192.0.2.10:5070;transport=udp"
        );
        assert_eq!(outcome.contacts[0].expires, 60);
        assert_eq!(store.binding_count(), 1);
    }

    #[tokio::test]
    async fn query_without_contact_returns_current_bindings() {
        let mut store = RegistrationStore::new();
        let register = request(concat!(
            "REGISTER sip:example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-reg\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:1001@example.com>\r\n",
            "Call-ID: reg-2@example.com\r\n",
            "CSeq: 1 REGISTER\r\n",
            "Contact: <sip:1001@192.0.2.10:5070>;expires=60\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        ));
        store
            .handle_register(
                &register,
                "192.0.2.10:5060".parse().unwrap(),
                SystemTime::UNIX_EPOCH,
                None,
                None,
            )
            .await
            .unwrap();

        let query = request(concat!(
            "REGISTER sip:example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-query\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:1001@example.com>\r\n",
            "Call-ID: reg-query@example.com\r\n",
            "CSeq: 2 REGISTER\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        ));
        let outcome = store
            .handle_register(
                &query,
                "192.0.2.10:5060".parse().unwrap(),
                SystemTime::UNIX_EPOCH + Duration::from_secs(10),
                None,
                None,
            )
            .await
            .unwrap();

        assert_eq!(outcome.contacts.len(), 1);
        assert_eq!(outcome.contacts[0].expires, 50);
    }

    #[tokio::test]
    async fn expires_zero_removes_contact() {
        let mut store = RegistrationStore::new();
        let register = request(concat!(
            "REGISTER sip:example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-reg\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:1001@example.com>\r\n",
            "Call-ID: reg-3@example.com\r\n",
            "CSeq: 1 REGISTER\r\n",
            "Contact: <sip:1001@192.0.2.10:5070>\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        ));
        store
            .handle_register(
                &register,
                "192.0.2.10:5060".parse().unwrap(),
                SystemTime::UNIX_EPOCH,
                None,
                None,
            )
            .await
            .unwrap();

        let unregister = request(concat!(
            "REGISTER sip:example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-unreg\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:1001@example.com>\r\n",
            "Call-ID: reg-3@example.com\r\n",
            "CSeq: 2 REGISTER\r\n",
            "Contact: <sip:1001@192.0.2.10:5070>;expires=0\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        ));
        let outcome = store
            .handle_register(
                &unregister,
                "192.0.2.10:5060".parse().unwrap(),
                SystemTime::UNIX_EPOCH,
                None,
                None,
            )
            .await
            .unwrap();

        assert!(outcome.contacts.is_empty());
        assert_eq!(store.binding_count(), 0);
    }

    #[tokio::test]
    async fn wildcard_contact_requires_expires_zero() {
        let mut store = RegistrationStore::new();
        let request = request(concat!(
            "REGISTER sip:example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-bad\r\n",
            "From: <sip:1001@example.com>;tag=from-tag\r\n",
            "To: <sip:1001@example.com>\r\n",
            "Call-ID: reg-bad@example.com\r\n",
            "CSeq: 1 REGISTER\r\n",
            "Contact: *\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        ));

        let error = store
            .handle_register(
                &request,
                "192.0.2.10:5060".parse().unwrap(),
                SystemTime::UNIX_EPOCH,
                None,
                None,
            )
            .await
            .unwrap_err();

        assert_eq!(error, RegisterError::InvalidContact("*".to_string()));
    }

    #[tokio::test]
    async fn lookup_contact_returns_active_binding_for_destination_uri() {
        let mut store = RegistrationStore::new();
        let register = request(concat!(
            "REGISTER sip:example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.20:5060;branch=z9hG4bK-reg\r\n",
            "From: <sip:1002@example.com>;tag=from-tag\r\n",
            "To: <sip:1002@example.com>\r\n",
            "Call-ID: reg-lookup@example.com\r\n",
            "CSeq: 1 REGISTER\r\n",
            "Contact: <sip:1002@192.0.2.20:5070;transport=udp>;expires=60\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        ));
        store
            .handle_register(
                &register,
                "192.0.2.20:5060".parse().unwrap(),
                SystemTime::UNIX_EPOCH,
                None,
                None,
            )
            .await
            .unwrap();

        let destination = "sip:1002@example.com".parse().unwrap();
        let contact = store
            .lookup_contact(
                &destination,
                SystemTime::UNIX_EPOCH + Duration::from_secs(5),
                None,
            )
            .await
            .expect("registered contact should be found");

        assert_eq!(contact.uri, "sip:1002@192.0.2.20:5070;transport=udp");
        assert_eq!(contact.expires, 55);
    }

    fn request(raw: &str) -> SipRequest {
        let sip_core::SipMessageBorrow::Request(request) = parse_message(raw.as_bytes()).unwrap()
        else {
            panic!("expected request");
        };
        request.into_owned()
    }

    #[allow(dead_code)]
    fn peer() -> SocketAddr {
        "192.0.2.10:5060".parse().unwrap()
    }
}
