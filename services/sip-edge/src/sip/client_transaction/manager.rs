use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use sip_core::{parse_message, SipMessageBorrow, SipResponse};
use tracing::{debug, warn};

use super::state::ClientTransactionControl;
use crate::sip::transaction::{branch_param, ClientTransactionKey};

struct ClientTransactionHandle {
    control: Arc<ClientTransactionControl>,
}

#[derive(Clone, Copy)]
struct PendingResponse {
    status_code: u16,
    seen_at: Instant,
}

pub(super) struct ClientTransactionRegistration {
    pub(super) control: Arc<ClientTransactionControl>,
}

#[derive(Default)]
pub(crate) struct ClientTransactionManager {
    active: DashMap<ClientTransactionKey, ClientTransactionHandle>,
    pending_by_branch: DashMap<String, PendingResponse>,
}

const PENDING_RESPONSE_LIMIT: usize = 4_096;
const PENDING_RESPONSE_TTL: Duration = Duration::from_secs(64);

impl ClientTransactionManager {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(super) fn register(
        &self,
        key: ClientTransactionKey,
    ) -> Option<ClientTransactionRegistration> {
        let control = Arc::new(ClientTransactionControl::new(&key.method));
        match self.active.entry(key.clone()) {
            dashmap::mapref::entry::Entry::Occupied(_) => return None,
            dashmap::mapref::entry::Entry::Vacant(entry) => entry.insert(ClientTransactionHandle {
                control: Arc::clone(&control),
            }),
        };

        if let Some((_, pending)) = self.pending_by_branch.remove(&key.branch) {
            if pending.seen_at.elapsed() <= PENDING_RESPONSE_TTL {
                control.on_response(pending.status_code);
            }
        }

        Some(ClientTransactionRegistration { control })
    }

    #[cfg(test)]
    pub(crate) fn contains_key(&self, key: &ClientTransactionKey) -> bool {
        self.active.contains_key(key)
    }

    pub(crate) fn observe_response(&self, response: &SipResponse) -> usize {
        let Some(call_id) = response
            .headers
            .get("call-id")
            .map(|value| value.as_str().to_string())
        else {
            return 0;
        };
        let Some((cseq, method)) = response.headers.get("cseq").and_then(|value| {
            let mut parts = value.as_str().split_whitespace();
            Some((parts.next()?.to_string(), parts.next()?.to_string()))
        }) else {
            return 0;
        };

        let branches = response
            .headers
            .get_all("via")
            .filter_map(|via| branch_param(via.as_str()))
            .collect::<HashSet<_>>();
        // The Via branch is the RFC 3261 transaction identifier. Keep a short-lived
        // response ledger independently of the active index so the retransmission
        // runner can make a final synchronous check immediately before socket I/O.
        self.remember_pending_response(&branches, response.status_code);
        let branch_matches = self
            .active
            .iter()
            .filter(|entry| {
                entry.key().method.eq_ignore_ascii_case(&method)
                    && branches.contains(&entry.key().branch)
            })
            .map(|entry| entry.key().clone())
            .collect::<HashSet<_>>();
        let mut matching_keys = branch_matches.clone();
        let mut matched_by_identity = false;

        if matching_keys.is_empty() {
            let identity_matches = self
                .active
                .iter()
                .filter(|entry| {
                    let key = entry.key();
                    key.call_id == call_id
                        && key.cseq == cseq
                        && key.method.eq_ignore_ascii_case(&method)
                })
                .map(|entry| entry.key().clone())
                .collect::<Vec<_>>();
            if identity_matches.len() == 1 {
                matching_keys.insert(identity_matches[0].clone());
                matched_by_identity = true;
            }
        }

        let mut delivered = 0;
        for key in matching_keys {
            if let Some(handle) = self.active.get(&key) {
                let action = handle.control.on_response(response.status_code);
                debug!(
                    status = response.status_code,
                    ?action,
                    state = ?handle.control.state(),
                    match_mode = if matched_by_identity { "call-id+cseq" } else { "via-branch" },
                    ?key,
                    "client transaction response applied synchronously"
                );
                delivered += 1;
            }
        }

        if delivered == 0 {
            if method.eq_ignore_ascii_case("INVITE") && response.status_code < 200 {
                warn!(
                    status = response.status_code,
                    %call_id,
                    %cseq,
                    %method,
                    ?branches,
                    active_transactions = self.active.len(),
                    "INVITE provisional response missed active transaction index; branch ledger will suppress retransmission"
                );
            } else {
                debug!(
                    status = response.status_code,
                    %call_id,
                    %cseq,
                    %method,
                    ?branches,
                    active_transactions = self.active.len(),
                    "SIP response did not match an active client transaction"
                );
            }
        }
        delivered
    }

    /// Applies a SIP response at the transport ingress before application dispatch.
    pub(crate) fn observe_packet(&self, packet: &[u8]) -> usize {
        let Ok(SipMessageBorrow::Response(response)) = parse_message(packet) else {
            return 0;
        };
        self.observe_response(&response.into_owned())
    }

    pub(super) fn apply_observed_branch_response(
        &self,
        key: &ClientTransactionKey,
        control: &ClientTransactionControl,
    ) -> bool {
        let Some(pending) = self.pending_by_branch.get(&key.branch) else {
            return false;
        };
        if pending.seen_at.elapsed() > PENDING_RESPONSE_TTL {
            drop(pending);
            self.pending_by_branch.remove(&key.branch);
            return false;
        }
        let status_code = pending.status_code;
        drop(pending);
        control.on_response(status_code);
        true
    }

    fn remember_pending_response(&self, branches: &HashSet<String>, status_code: u16) {
        if self.pending_by_branch.len() >= PENDING_RESPONSE_LIMIT {
            let now = Instant::now();
            self.pending_by_branch
                .retain(|_, pending| now.duration_since(pending.seen_at) <= PENDING_RESPONSE_TTL);
        }
        if self.pending_by_branch.len() >= PENDING_RESPONSE_LIMIT {
            return;
        }

        for branch in branches {
            if self.pending_by_branch.len() >= PENDING_RESPONSE_LIMIT {
                break;
            }
            self.pending_by_branch.insert(
                branch.clone(),
                PendingResponse {
                    status_code,
                    seen_at: Instant::now(),
                },
            );
        }
    }

    pub(crate) fn cancel(&self, key: &ClientTransactionKey) -> bool {
        let Some(handle) = self.active.get(key) else {
            return false;
        };
        handle.control.cancel();
        true
    }

    pub(super) fn finish(
        &self,
        key: &ClientTransactionKey,
        control: &Arc<ClientTransactionControl>,
    ) {
        let mut removed_current = false;
        match self.active.entry(key.clone()) {
            dashmap::mapref::entry::Entry::Occupied(entry)
                if Arc::ptr_eq(&entry.get().control, control) =>
            {
                entry.remove();
                removed_current = true;
            }
            _ => {}
        }
        if removed_current {
            self.pending_by_branch.remove(&key.branch);
        }
    }

    #[cfg(test)]
    pub(super) fn active_len(&self) -> usize {
        self.active.len()
    }
}
