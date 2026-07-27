//! # B2BUA 会话生命周期
//!
//! 本模块扩展 [`EdgeState`][super::EdgeState]，负责 B2BUA 会话的创建、绑定与回收：
//!
//! - `remember_inbound_invite`: 创建 A-leg 并写入 `inbound_transactions`，
//!   同时初始化 B-leg 占位 dialog 与 caller 并发计数。
//! - `bind_gateway_dialog`: 收到 B-leg INVITE 响应后，将 gateway Call-ID 绑定到 session_id。
//! - `teardown_call_transaction`: 原子移除会话并释放所有媒体资源。
//! - DID 路由（`replace_did_destinations` / `resolve_number_destination` / `did_destination`）。
//!
//! ## B2BUA 模型
//!
//! ```text
//! A-leg Call-ID ─┐
//!                ├─> session_id ─> media_session
//! B-leg Call-ID ─┘
//! ```
//!
//! A-leg Call-ID 由入站 INVITE 携带，B-leg Call-ID 由出站 INVITE 在
//! [`outbound::invite::build_b2bua_outbound_invite`] 中生成；二者通过
//! [`CallSessionStore::index_dialog`][super::models::CallSessionStore::index_dialog]
//! 统一索引到 `session_id`。

use std::net::SocketAddr;
use std::sync::Arc;

use sdp_core::RtpEndpoint;
use sip_core::{SipRequest, SipUri};

use crate::sip::dialog;
use crate::sip::response::EDGE_TAG;

use super::models::{B2buaDialogPair, DialogLegState, InboundTransaction, InviteResponseOrder};
use super::uri_utils::{extract_uri_from_contact, sip_uri_from_peer};
use super::EdgeState;

impl EdgeState {
    /// 刷新 DID 路由表（来自数据库快照）。
    pub(crate) fn replace_did_destinations(
        &self,
        dids: std::collections::HashMap<String, cdr_core::DidDestination>,
    ) {
        if let Ok(mut current) = self.did_destinations.write() {
            *current = dids;
        } else {
            tracing::error!("DID 目标路由缓存锁已损坏，忽略本次刷新");
        }
    }

    /// 按号码查表，将 URI 的 user 部分替换为 DID 目标 extension ID。
    ///
    /// 仅当 DID 启用且 `target_type == "extension"` 时改写，否则原样返回。
    pub(crate) fn resolve_number_destination(&self, uri: &SipUri) -> SipUri {
        let Some(number) = uri.user.as_deref() else {
            return uri.clone();
        };
        let target_id = self.did_destinations.read().ok().and_then(|dids| {
            dids.get(number)
                .filter(|did| did.enabled && did.target_type == "extension")
                .map(|did| did.target_id.clone())
        });
        let Some(target_id) = target_id else {
            return uri.clone();
        };
        let mut resolved = uri.clone();
        resolved.user = Some(target_id.into());
        resolved
    }

    /// Returns the enabled DID rule for a real number.
    pub(crate) fn did_destination(&self, number: &str) -> Option<cdr_core::DidDestination> {
        self.did_destinations
            .read()
            .ok()
            .and_then(|destinations| destinations.get(number).filter(|did| did.enabled).cloned())
    }

    /// 将 B-leg Call-ID 绑定到已有 session_id（用于 B2BUA 拓扑隐藏）。
    ///
    /// 在出站 INVITE 发送后立即调用，使后续 B-leg 的 in-dialog 请求能命中 session。
    pub(crate) fn bind_gateway_dialog(&self, session_id: &str, gateway_call_id: &str) {
        if let Some(mut transaction) = self.inbound_transactions.get_mut(session_id) {
            transaction.dialogs.gateway.call_id = gateway_call_id.to_string();
            let session_id = transaction.session_id.clone();
            drop(transaction);
            self.inbound_transactions
                .index_dialog(&session_id, gateway_call_id);
        }
    }

    /// 记录入站 INVITE，创建 B2BUA 会话。
    ///
    /// 此函数是 B2BUA 的"会话诞生点"：
    ///
    /// 1. 从入站 INVITE 解析 A-leg dialog 字段（Call-ID / From / To / CSeq / Route）
    /// 2. 为 B-leg 预占 dialog（local_tag 由本机生成，gateway Call-ID 暂为空，
    ///    由 `bind_gateway_dialog` 后续填入）
    /// 3. 写入 [`CallSessionStore`][super::models::CallSessionStore] 并索引 A-leg Call-ID
    /// 4. 递增 caller 用户并发计数
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn remember_inbound_invite(
        &self,
        session_id: String,
        request: &SipRequest,
        peer: SocketAddr,
        outbound_uri: SipUri,
        caller_rtp: Option<RtpEndpoint>,
        gateway_relay_rtp: Option<RtpEndpoint>,
        max_duration_secs: Option<u32>,
    ) {
        let Some(call_id) = request.headers.get("call-id") else {
            return;
        };

        let inbound_route_set = request
            .headers
            .get_all("record-route")
            .map(|value| value.as_str().to_string())
            .collect::<Vec<_>>();
        let caller_contact = request
            .headers
            .get("contact")
            .and_then(|value| extract_uri_from_contact(value.as_str()))
            .map(|mut uri| {
                if uri.port.is_none() {
                    uri.port = Some(peer.port());
                }
                uri
            });
        let caller_call_id = call_id.as_str().to_string();
        let caller_remote_uri = request
            .headers
            .get("from")
            .and_then(|value| extract_uri_from_contact(value.as_str()))
            .unwrap_or_else(|| sip_uri_from_peer(&peer.to_string()));
        let caller_local_uri = request
            .headers
            .get("to")
            .and_then(|value| extract_uri_from_contact(value.as_str()))
            .unwrap_or_else(|| outbound_uri.clone());
        let caller_remote_tag = request
            .headers
            .get("from")
            .and_then(|value| dialog::tag_param(value.as_str()));
        let caller_remote_cseq = request
            .headers
            .get("cseq")
            .and_then(|value| dialog::cseq_number(value.as_str()));
        let caller_remote_target = caller_contact
            .clone()
            .unwrap_or_else(|| sip_uri_from_peer(&peer.to_string()));
        let dialogs = B2buaDialogPair {
            caller: DialogLegState {
                call_id: caller_call_id.clone(),
                local_uri: caller_local_uri,
                remote_uri: caller_remote_uri.clone(),
                // caller leg 的 local_tag 与 `response::append_to_header` 自动填充的 EDGE_TAG
                // 保持一致，确保 in-dialog 请求的 To tag 校验不会因 tag 不匹配而拒绝。
                local_tag: EDGE_TAG.to_string(),
                remote_tag: caller_remote_tag.clone(),
                local_cseq: 0,
                remote_cseq: caller_remote_cseq,
                route_set: inbound_route_set.clone(),
                remote_target: caller_remote_target,
                peer: Some(peer.to_string()),
            },
            gateway: DialogLegState {
                call_id: String::new(),
                local_uri: caller_remote_uri,
                remote_uri: outbound_uri.clone(),
                local_tag: format!("vosrs-b-{}", uuid::Uuid::new_v4().simple()),
                remote_tag: None,
                local_cseq: caller_remote_cseq.unwrap_or(1),
                remote_cseq: None,
                route_set: Vec::new(),
                remote_target: outbound_uri.clone(),
                peer: None,
            },
        };

        self.inbound_transactions.insert(InboundTransaction {
            session_id,
            dialogs,
            original_request: Some(Arc::new(request.clone())),
            caller_rtp,
            gateway_relay_rtp,
            gateway_rtp: None,
            caller_relay_rtp: None,
            session_expires: None,
            session_refresher: None,
            last_session_refresh: None,
            prack_rseq: 0,
            gateway_100rel: false,
            refer_subscription: None,
            transfer_dialog: None,
            fork_dialogs: Default::default(),
            max_duration_secs,
            established_at: None,
            invite_response_order: Arc::new(
                tokio::sync::Mutex::new(InviteResponseOrder::default()),
            ),
        });

        // 记录该用户新增一路活跃并发通话
        if let Some(username) = Self::username_from_request(request) {
            self.increment_user_concurrency(&username);
        }
    }

    /// 原子移除 SIP 会话事务并释放其拥有的全部媒体资源。
    pub(crate) fn teardown_call_transaction(&self, call_id: &str) -> Option<InboundTransaction> {
        let (_, transaction) = self.inbound_transactions.remove(call_id)?;
        self.media_relay.clear_dtmf_digits(&transaction.session_id);
        self.media_relay.clear_dtmf_events(&transaction.session_id);
        self.clear_media_targets(&transaction);
        Some(transaction)
    }
}

/// 防止 `Method` 在某些 feature 组合下未被引用。
#[allow(unused_imports)]
use sip_core::Method as _MethodMarker;
