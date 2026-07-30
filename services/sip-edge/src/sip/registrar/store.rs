use cdr_core::PostgresCdrStore;
use sip_core::{SipRequest, SipUri};
use std::{
    collections::HashMap,
    net::SocketAddr,
    time::{Duration, SystemTime},
};
use time::OffsetDateTime;

use super::error::RegisterError;
use super::helpers::{
    address_of_record, canonical_aor, parse_contact, remaining_seconds, request_expires,
    ContactUpdate,
};
use super::{
    RegisterOutcome, RegistrationContact, RegistrationInvalidateMsg, DEFAULT_EXPIRES_SECONDS,
    MAX_EXPIRES_SECONDS,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RegistrationBinding {
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
        nats_client: Option<&async_nats::Client>,
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
            self.apply_contact(&aor, &contact, request, peer, now, db_store, nats_client)
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
                    for (uri, received_from, _user_agent, expires_at, path) in rows {
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

    #[allow(clippy::too_many_arguments)]
    async fn apply_contact(
        &mut self,
        aor: &str,
        raw_contact: &str,
        request: &SipRequest,
        peer: SocketAddr,
        now: SystemTime,
        db_store: Option<&PostgresCdrStore>,
        nats_client: Option<&async_nats::Client>,
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
                let msg = RegistrationInvalidateMsg {
                    aor: aor.to_string(),
                    action: "unregister".to_string(),
                    timestamp: SystemTime::now()
                        .duration_since(SystemTime::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs(),
                };
                if let Some(nats) = nats_client {
                    let _ = nats
                        .publish(
                            "vos_rs.cluster.registration.invalidate",
                            serde_json::to_string(&msg).unwrap_or_default().into(),
                        )
                        .await;
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
                    let msg = RegistrationInvalidateMsg {
                        aor: aor.to_string(),
                        action: "unregister".to_string(),
                        timestamp: SystemTime::now()
                            .duration_since(SystemTime::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs(),
                    };
                    if let Some(nats) = nats_client {
                        let _ = nats
                            .publish(
                                "vos_rs.cluster.registration.invalidate",
                                serde_json::to_string(&msg).unwrap_or_default().into(),
                            )
                            .await;
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
                let user_agent = request
                    .headers
                    .get("user-agent")
                    .map(|v| v.as_str().to_string());
                let expires_at = now + Duration::from_secs(u64::from(expires));
                let binding = RegistrationBinding {
                    uri: uri.clone(),
                    received_from: peer,
                    expires_at,
                    path: path.clone(),
                };
                self.bindings
                    .entry(aor.to_string())
                    .or_default()
                    .insert(uri.clone(), binding);

                if let Some(db) = db_store {
                    let since_epoch = expires_at
                        .duration_since(SystemTime::UNIX_EPOCH)
                        .unwrap_or_default();
                    let offset_dt =
                        OffsetDateTime::from_unix_timestamp_nanos(since_epoch.as_nanos() as i128)
                            .unwrap_or(OffsetDateTime::UNIX_EPOCH);
                    let _ = db
                        .upsert_registration(
                            aor,
                            &uri,
                            &peer.to_string(),
                            user_agent.as_deref(),
                            offset_dt,
                            &path,
                        )
                        .await;
                }
                let msg = RegistrationInvalidateMsg {
                    aor: aor.to_string(),
                    action: "register".to_string(),
                    timestamp: SystemTime::now()
                        .duration_since(SystemTime::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs(),
                };
                if let Some(nats) = nats_client {
                    let _ = nats
                        .publish(
                            "vos_rs.cluster.registration.invalidate",
                            serde_json::to_string(&msg).unwrap_or_default().into(),
                        )
                        .await;
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
