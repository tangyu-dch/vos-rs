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

use cdr_core::PostgresCdrStore;
use sip_core::{SipRequest, SipUri};
use std::{
    collections::HashMap,
    error::Error,
    fmt,
    net::SocketAddr,
    str::FromStr,
    time::{Duration, SystemTime},
};
use time::OffsetDateTime;

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

#[derive(Debug, Clone, PartialEq, Eq)]
struct RegistrationBinding {
    uri: String,
    received_from: SocketAddr,
    expires_at: SystemTime,
    path: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct RegistrationStore {
    bindings: HashMap<String, HashMap<String, RegistrationBinding>>,
}

impl RegistrationStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn handle_register(
        &mut self,
        request: &SipRequest,
        peer: SocketAddr,
        now: SystemTime,
        db_store: Option<&PostgresCdrStore>,
    ) -> Result<RegisterOutcome, RegisterError> {
        self.prune_expired(now, db_store).await;

        let aor = address_of_record(request)?;
        let contacts = request
            .headers
            .get_all("contact")
            .map(|value| value.as_str().to_string())
            .collect::<Vec<_>>();

        if contacts.is_empty() {
            return Ok(RegisterOutcome {
                contacts: self.active_contacts(&aor, now, db_store).await,
                aor,
            });
        }

        for contact in contacts {
            self.apply_contact(&aor, &contact, request, peer, now, db_store)
                .await?;
        }

        Ok(RegisterOutcome {
            contacts: self.active_contacts(&aor, now, db_store).await,
            aor,
        })
    }

    #[cfg(test)]
    pub fn binding_count(&self) -> usize {
        self.bindings.values().map(HashMap::len).sum()
    }

    pub async fn active_contacts(
        &self,
        aor: &str,
        now: SystemTime,
        db_store: Option<&PostgresCdrStore>,
    ) -> Vec<RegistrationContact> {
        if let Some(db) = db_store {
            match db.get_registrations(aor).await {
                Ok(rows) => {
                    let mut contacts = Vec::new();
                    for (uri, received_from, expires_at, path) in rows {
                        let nanos = expires_at.unix_timestamp_nanos();
                        let sys_expires_at = if nanos > 0 {
                            SystemTime::UNIX_EPOCH + Duration::from_nanos(nanos as u64)
                        } else {
                            SystemTime::UNIX_EPOCH
                        };
                        if let Some(expires) = remaining_seconds(sys_expires_at, now) {
                            contacts.push(RegistrationContact {
                                uri,
                                expires,
                                received_from,
                                path,
                            });
                        }
                    }
                    contacts
                }
                Err(_) => Vec::new(),
            }
        } else {
            self.bindings
                .get(aor)
                .into_iter()
                .flat_map(|bindings| bindings.values())
                .filter_map(|binding| {
                    let expires = remaining_seconds(binding.expires_at, now)?;
                    Some(RegistrationContact {
                        uri: binding.uri.clone(),
                        expires,
                        received_from: binding.received_from.to_string(),
                        path: binding.path.clone(),
                    })
                })
                .collect()
        }
    }

    pub async fn lookup_contact(
        &self,
        destination_uri: &SipUri,
        now: SystemTime,
        db_store: Option<&PostgresCdrStore>,
    ) -> Option<RegistrationContact> {
        let aor = canonical_aor(destination_uri).ok()?;
        self.active_contacts(&aor, now, db_store)
            .await
            .into_iter()
            .next()
    }

    pub async fn get_all_active_received_from(
        &self,
        now: SystemTime,
        db_store: Option<&PostgresCdrStore>,
    ) -> Vec<String> {
        if let Some(db) = db_store {
            db.get_all_active_received_from().await.unwrap_or_default()
        } else {
            let mut list = Vec::new();
            for aor_bindings in self.bindings.values() {
                for binding in aor_bindings.values() {
                    if binding.expires_at > now {
                        list.push(binding.received_from.to_string());
                    }
                }
            }
            list
        }
    }

    async fn apply_contact(
        &mut self,
        aor: &str,
        raw_contact: &str,
        request: &SipRequest,
        peer: SocketAddr,
        now: SystemTime,
        db_store: Option<&PostgresCdrStore>,
    ) -> Result<(), RegisterError> {
        match parse_contact(raw_contact)? {
            ContactUpdate::Wildcard => {
                let expires = request_expires(request)?.unwrap_or(DEFAULT_EXPIRES_SECONDS);
                if expires != 0 {
                    return Err(RegisterError::InvalidContact(raw_contact.to_string()));
                }
                if let Some(db) = db_store {
                    let _ = db.delete_all_registrations(aor).await;
                } else {
                    self.bindings.remove(aor);
                }
                Ok(())
            }
            ContactUpdate::Contact {
                uri,
                contact_expires,
            } => {
                let expires = contact_expires
                    .or(request_expires(request)?)
                    .unwrap_or(DEFAULT_EXPIRES_SECONDS)
                    .min(MAX_EXPIRES_SECONDS);
                if expires == 0 {
                    if let Some(db) = db_store {
                        let _ = db.delete_registration(aor, &uri).await;
                    } else if let Some(bindings) = self.bindings.get_mut(aor) {
                        bindings.remove(&uri);
                        if bindings.is_empty() {
                            self.bindings.remove(aor);
                        }
                    }
                    return Ok(());
                }

                let path = request
                    .headers
                    .get_all("path")
                    .flat_map(|v| v.as_str().split(','))
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<String>>();

                let expires_at = now + Duration::from_secs(u64::from(expires));
                if let Some(db) = db_store {
                    let since_epoch = expires_at
                        .duration_since(SystemTime::UNIX_EPOCH)
                        .unwrap_or_default();
                    let offset_dt =
                        OffsetDateTime::from_unix_timestamp_nanos(since_epoch.as_nanos() as i128)
                            .unwrap_or(OffsetDateTime::UNIX_EPOCH);
                    let _ = db
                        .upsert_registration(aor, &uri, &peer.to_string(), offset_dt, &path)
                        .await;
                } else {
                    let binding = RegistrationBinding {
                        uri: uri.clone(),
                        received_from: peer,
                        expires_at,
                        path,
                    };
                    self.bindings
                        .entry(aor.to_string())
                        .or_default()
                        .insert(uri, binding);
                }
                Ok(())
            }
        }
    }

    async fn prune_expired(&mut self, now: SystemTime, db_store: Option<&PostgresCdrStore>) {
        if let Some(db) = db_store {
            let _ = db.prune_expired_registrations().await;
        } else {
            self.bindings.retain(|_, bindings| {
                bindings.retain(|_, binding| binding.expires_at > now);
                !bindings.is_empty()
            });
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)]
pub enum RegisterError {
    InvalidAddressOfRecord(String),
    InvalidContact(String),
    InvalidExpires(String),
}

impl fmt::Display for RegisterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidAddressOfRecord(value) => {
                write!(f, "invalid REGISTER address-of-record: {value}")
            }
            Self::InvalidContact(value) => write!(f, "invalid REGISTER Contact: {value}"),
            Self::InvalidExpires(value) => write!(f, "invalid REGISTER Expires: {value}"),
        }
    }
}

impl Error for RegisterError {}

enum ContactUpdate {
    Wildcard,
    Contact {
        uri: String,
        contact_expires: Option<u32>,
    },
}

fn address_of_record(request: &SipRequest) -> Result<String, RegisterError> {
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

fn parse_contact(raw: &str) -> Result<ContactUpdate, RegisterError> {
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

fn split_contact_uri_and_params(raw: &str) -> Option<(&str, &str)> {
    if let Some(start) = raw.find('<') {
        let end = raw[start + 1..].find('>')? + start + 1;
        return Some((&raw[start + 1..end], raw[end + 1..].trim()));
    }

    match raw.split_once(';') {
        Some((uri, params)) => Some((uri.trim(), params)),
        None => Some((raw.trim(), "")),
    }
}

fn parse_uri_from_header(raw: &str) -> Option<SipUri> {
    let value = raw.trim();
    let uri_raw = if let Some(start) = value.find('<') {
        let end = value[start + 1..].find('>')? + start + 1;
        &value[start + 1..end]
    } else {
        value.split(';').next().unwrap_or(value).trim()
    };

    SipUri::from_str(uri_raw).ok()
}

fn request_expires(request: &SipRequest) -> Result<Option<u32>, RegisterError> {
    request
        .headers
        .get("expires")
        .map(|value| parse_expires(value.as_str()))
        .transpose()
}

fn contact_param<'a>(params: &'a str, name: &str) -> Option<&'a str> {
    params
        .split(';')
        .filter_map(|param| param.trim().split_once('='))
        .find_map(|(key, value)| key.eq_ignore_ascii_case(name).then_some(value.trim()))
}

fn parse_expires(raw: &str) -> Result<u32, RegisterError> {
    raw.trim()
        .parse::<u32>()
        .map_err(|_| RegisterError::InvalidExpires(raw.trim().to_string()))
}

fn remaining_seconds(expires_at: SystemTime, now: SystemTime) -> Option<u32> {
    let duration = expires_at.duration_since(now).ok()?;
    let seconds = duration.as_secs().min(u64::from(u32::MAX));
    u32::try_from(seconds).ok()
}

#[cfg(test)]
mod tests {
    use super::{RegisterError, RegistrationStore};
    use sip_core::{parse_message, SipMessage, SipRequest};
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
        let SipMessage::Request(request) = parse_message(raw.as_bytes()).unwrap() else {
            panic!("expected request");
        };
        request
    }

    #[allow(dead_code)]
    fn peer() -> SocketAddr {
        "192.0.2.10:5060".parse().unwrap()
    }
}
