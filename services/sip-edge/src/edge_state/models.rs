use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use sdp_core::RtpEndpoint;
use sip_core::{parse_message, SipMessageBorrow, SipRequest, SipUri};
use tokio::sync::Mutex;

#[derive(Debug, Clone)]
pub(crate) struct InviteResponseMetadata {
    pub(crate) order: Arc<Mutex<InviteResponseOrder>>,
    pub(crate) cseq: Option<u32>,
    pub(crate) status_code: u16,
}

#[derive(Debug, Clone)]
pub struct PendingDatagram {
    pub target: String,
    pub bytes: Vec<u8>,
    kind: SipDatagramKind,
    pub(crate) invite_response: Option<InviteResponseMetadata>,
}

/// The protocol role carried by a queued SIP datagram.
///
/// This role is derived from the SIP start line when the datagram enters the
/// transport queue. Network addresses are deliberately excluded: several SIP
/// accounts may legitimately share one NAT endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SipDatagramKind {
    Request,
    Response,
    Keepalive,
    Invalid,
}

impl PendingDatagram {
    pub fn new(target: impl Into<String>, bytes: Vec<u8>) -> Self {
        let kind = SipDatagramKind::classify(&bytes);
        Self {
            target: target.into(),
            bytes,
            kind,
            invite_response: None,
        }
    }

    pub(crate) fn is_request(&self) -> bool {
        self.kind == SipDatagramKind::Request
    }

    pub(crate) fn is_response(&self) -> bool {
        self.kind == SipDatagramKind::Response
    }

    pub fn with_invite_response_order(
        mut self,
        order: Arc<Mutex<InviteResponseOrder>>,
        cseq: Option<u32>,
        status_code: u16,
    ) -> Self {
        self.invite_response = Some(InviteResponseMetadata {
            order,
            cseq,
            status_code,
        });
        self
    }
}

impl SipDatagramKind {
    fn classify(bytes: &[u8]) -> Self {
        if bytes.iter().all(|byte| matches!(byte, b'\r' | b'\n')) {
            return Self::Keepalive;
        }

        match parse_message(bytes) {
            Ok(SipMessageBorrow::Request(_)) => Self::Request,
            Ok(SipMessageBorrow::Response(_)) => Self::Response,
            Err(_) => Self::Invalid,
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct InviteResponseOrder {
    pub(crate) cseq: Option<u32>,
    pub(crate) final_response_seen: bool,
    pub(crate) final_response_send_started: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct DialogLegState {
    pub(crate) call_id: String,
    pub(crate) local_uri: SipUri,
    pub(crate) remote_uri: SipUri,
    pub(crate) local_tag: String,
    pub(crate) remote_tag: Option<String>,
    pub(crate) local_cseq: u32,
    pub(crate) remote_cseq: Option<u32>,
    pub(crate) route_set: Vec<String>,
    pub(crate) remote_target: SipUri,
    pub(crate) peer: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct B2buaDialogPair {
    pub(crate) caller: DialogLegState,
    pub(crate) gateway: DialogLegState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DialogLeg {
    Caller,
    Gateway,
    Transfer,
}

#[derive(Debug, Clone)]
pub(crate) struct TransferDialogState {
    pub(crate) dialog: DialogLegState,
    pub(crate) transferee_leg: DialogLeg,
}

#[derive(Debug, Clone)]
pub(crate) struct ForkDialogState {
    pub(crate) dialog: DialogLegState,
    pub(crate) gateway_id: String,
}

impl B2buaDialogPair {
    pub(crate) fn placeholder(
        caller_call_id: impl Into<String>,
        outbound_uri: SipUri,
        caller_peer: impl Into<String>,
    ) -> Self {
        let caller_call_id = caller_call_id.into();
        let caller_peer = caller_peer.into();
        let caller_tag = format!("vosrs-a-{}", uuid::Uuid::new_v4().simple());
        let gateway_tag = format!("vosrs-b-{}", uuid::Uuid::new_v4().simple());
        Self {
            caller: DialogLegState {
                call_id: caller_call_id,
                local_uri: outbound_uri.clone(),
                remote_uri: outbound_uri.clone(),
                local_tag: caller_tag,
                remote_tag: None,
                local_cseq: 0,
                remote_cseq: None,
                route_set: Vec::new(),
                remote_target: outbound_uri.clone(),
                peer: Some(caller_peer),
            },
            gateway: DialogLegState {
                call_id: String::new(),
                local_uri: outbound_uri.clone(),
                remote_uri: outbound_uri.clone(),
                local_tag: gateway_tag,
                remote_tag: None,
                local_cseq: 1,
                remote_cseq: None,
                route_set: Vec::new(),
                remote_target: outbound_uri,
                peer: None,
            },
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct InboundTransaction {
    pub(crate) session_id: String,
    pub(crate) dialogs: B2buaDialogPair,
    /// 原始 INVITE 请求模板，用于后续响应构建与 CDR 记录。
    pub(crate) original_request: Option<Arc<SipRequest>>,
    pub(crate) caller_rtp: Option<RtpEndpoint>,
    pub(crate) caller_relay_rtp: Option<RtpEndpoint>,
    pub(crate) gateway_relay_rtp: Option<RtpEndpoint>,
    pub(crate) gateway_rtp: Option<RtpEndpoint>,
    pub(crate) session_expires: Option<u32>,
    pub(crate) session_refresher: Option<String>,
    pub(crate) last_session_refresh: Option<Instant>,
    pub(crate) gateway_100rel: bool,
    pub(crate) prack_rseq: u32,
    pub(crate) refer_subscription: Option<crate::edge_state::ReferSubscription>,
    pub(crate) transfer_dialog: Option<TransferDialogState>,
    pub(crate) fork_dialogs: HashMap<String, ForkDialogState>,
    pub(crate) max_duration_secs: Option<u32>,
    pub(crate) established_at: Option<Instant>,
    pub(crate) invite_response_order: Arc<Mutex<InviteResponseOrder>>,
    /// 多租户上下文：在 INVITE 入站时解析得到，贯穿呼叫生命周期。
    /// None 表示未启用多租户隔离（tenant_enabled=false）或未关联租户。
    pub(crate) tenant: Option<crate::tenant::TenantContext>,
}

/// Stores B2BUA sessions by an internal session ID and resolves either dialog Call-ID to it.
#[derive(Debug, Default)]
pub(crate) struct CallSessionStore {
    sessions: dashmap::DashMap<String, InboundTransaction>,
    dialog_index: dashmap::DashMap<String, String>,
}

impl CallSessionStore {
    pub(crate) fn insert(&self, transaction: InboundTransaction) -> Option<InboundTransaction> {
        let session_id = transaction.session_id.clone();
        self.index_dialog(&session_id, &transaction.dialogs.caller.call_id);
        self.index_dialog(&session_id, &transaction.dialogs.gateway.call_id);
        if let Some(transfer) = &transaction.transfer_dialog {
            self.index_dialog(&session_id, &transfer.dialog.call_id);
        }
        for fork_call_id in transaction.fork_dialogs.keys() {
            self.index_dialog(&session_id, fork_call_id);
        }
        self.sessions.insert(session_id, transaction)
    }

    pub(crate) fn insert_fork_dialog(&self, session_id: &str, fork: ForkDialogState) -> bool {
        let call_id = fork.dialog.call_id.clone();
        if call_id.is_empty() {
            return false;
        }
        let Some(mut transaction) = self.get_mut(session_id) else {
            return false;
        };
        transaction.fork_dialogs.insert(call_id.clone(), fork);
        let canonical_session_id = transaction.session_id.clone();
        drop(transaction);
        self.index_dialog(&canonical_session_id, &call_id);
        true
    }

    pub(crate) fn index_dialog(&self, session_id: &str, call_id: &str) {
        if !call_id.is_empty() {
            self.dialog_index
                .insert(call_id.to_string(), session_id.to_string());
        }
    }

    pub(crate) fn session_id_for_dialog(&self, call_id: &str) -> Option<String> {
        if self.sessions.contains_key(call_id) {
            return Some(call_id.to_string());
        }
        self.dialog_index.get(call_id).map(|entry| entry.clone())
    }

    pub(crate) fn get(
        &self,
        call_id: &str,
    ) -> Option<dashmap::mapref::one::Ref<'_, String, InboundTransaction>> {
        let session_id = self.session_id_for_dialog(call_id)?;
        self.sessions.get(&session_id)
    }

    pub(crate) fn get_mut(
        &self,
        call_id: &str,
    ) -> Option<dashmap::mapref::one::RefMut<'_, String, InboundTransaction>> {
        let session_id = self.session_id_for_dialog(call_id)?;
        self.sessions.get_mut(&session_id)
    }

    pub(crate) fn contains_key(&self, call_id: &str) -> bool {
        self.session_id_for_dialog(call_id)
            .is_some_and(|session_id| self.sessions.contains_key(&session_id))
    }

    pub(crate) fn remove(&self, call_id: &str) -> Option<(String, InboundTransaction)> {
        let session_id = self.session_id_for_dialog(call_id)?;
        let (_, transaction) = self.sessions.remove(&session_id)?;
        self.dialog_index
            .retain(|_, indexed_session| indexed_session != &session_id);
        Some((session_id, transaction))
    }

    pub(crate) fn iter(
        &self,
    ) -> impl Iterator<Item = dashmap::mapref::multiple::RefMulti<'_, String, InboundTransaction>>
    {
        self.sessions.iter()
    }

    pub(crate) fn iter_mut(
        &self,
    ) -> impl Iterator<Item = dashmap::mapref::multiple::RefMutMulti<'_, String, InboundTransaction>>
    {
        self.sessions.iter_mut()
    }
}
