//! # InboundTransaction 对话校验
//!
//! 本模块扩展 [`InboundTransaction`][super::InboundTransaction]，提供入站 in-dialog 请求的
//! 对话匹配与 CSeq 校验。

use sip_core::{Method, SipRequest};
use std::net::SocketAddr;

use crate::edge_state::models::{DialogLeg, InboundTransaction};
use crate::sip::dialog::{cseq_number, tag_param, DialogValidationError};

impl InboundTransaction {
    /// 校验入站 in-dialog 请求所属的对话腿，并返回 CSeq 序号（ACK/CANCEL 不返回）。
    ///
    /// 匹配顺序：
    /// 1. 按 Call-ID 命中 caller/gateway/transfer 三腿
    /// 2. 否则按 peer 地址精确匹配
    /// 3. 最后回退到 IP 级别匹配（应对 NAT 端口变化）
    pub(crate) fn validate_in_dialog_request(
        &self,
        request: &SipRequest,
        peer: SocketAddr,
    ) -> Result<(DialogLeg, Option<u32>), DialogValidationError> {
        let request_call_id = request
            .headers
            .get("call-id")
            .map(|value| value.as_str())
            .unwrap_or_default();
        let request_from_tag = request
            .headers
            .get("from")
            .and_then(|value| tag_param(value.as_str()))
            .ok_or(DialogValidationError::MissingFromTag)?;

        let leg = if request_call_id == self.dialogs.caller.call_id {
            DialogLeg::Caller
        } else if request_call_id == self.dialogs.gateway.call_id {
            DialogLeg::Gateway
        } else if self
            .transfer_dialog
            .as_ref()
            .is_some_and(|transfer| request_call_id == transfer.dialog.call_id)
        {
            DialogLeg::Transfer
        } else {
            self.dialog_leg_for_peer(peer)
                .ok_or(DialogValidationError::PeerMismatch)?
        };
        let dialog = match leg {
            DialogLeg::Caller => &self.dialogs.caller,
            DialogLeg::Gateway => &self.dialogs.gateway,
            DialogLeg::Transfer => {
                &self
                    .transfer_dialog
                    .as_ref()
                    .ok_or(DialogValidationError::PeerMismatch)?
                    .dialog
            }
        };

        if dialog.remote_tag.as_deref() != Some(request_from_tag.as_str()) {
            return Err(DialogValidationError::FromTagMismatch);
        }

        if !matches!(&request.method, Method::Cancel) {
            let request_to_tag = request
                .headers
                .get("to")
                .and_then(|value| tag_param(value.as_str()));
            if request_to_tag.as_deref() != Some(dialog.local_tag.as_str()) {
                return Err(DialogValidationError::ToTagMismatch);
            }
        }

        if !matches!(&request.method, Method::Ack | Method::Cancel) {
            let cseq = request
                .headers
                .get("cseq")
                .ok_or(DialogValidationError::MissingCSeq)
                .and_then(|value| {
                    cseq_number(value.as_str()).ok_or(DialogValidationError::InvalidCSeq)
                })?;
            let last_cseq = dialog.remote_cseq;
            if let Some(last_cseq) = last_cseq {
                if cseq <= last_cseq {
                    return Err(DialogValidationError::CSeqOutOfOrder {
                        received: cseq,
                        last: last_cseq,
                    });
                }
            }
            return Ok((leg, Some(cseq)));
        }

        Ok((leg, None))
    }

    /// 按 peer 地址定位对话腿。
    ///
    /// 优先精确匹配 `host:port`，失败时回退到 IP 级匹配，应对 NAT 端口漂移场景。
    pub(crate) fn dialog_leg_for_peer(&self, peer: SocketAddr) -> Option<DialogLeg> {
        let peer_str = peer.to_string();
        if self.dialogs.caller.peer.as_deref() == Some(peer_str.as_str()) {
            return Some(DialogLeg::Caller);
        }
        if self.dialogs.gateway.peer.as_deref() == Some(peer_str.as_str()) {
            return Some(DialogLeg::Gateway);
        }
        if self
            .transfer_dialog
            .as_ref()
            .and_then(|transfer| transfer.dialog.peer.as_deref())
            == Some(peer_str.as_str())
        {
            return Some(DialogLeg::Transfer);
        }
        // Fallback: IP-level matching in case client port changed or behind NAT
        let peer_ip = peer.ip().to_string();
        if let Some(caller_peer) = self.dialogs.caller.peer.as_deref() {
            if let Ok(caller_addr) = caller_peer.parse::<SocketAddr>() {
                if caller_addr.ip().to_string() == peer_ip {
                    return Some(DialogLeg::Caller);
                }
            }
        }
        if let Some(ref out_peer) = self.dialogs.gateway.peer {
            if let Ok(out_addr) = out_peer.parse::<SocketAddr>() {
                if out_addr.ip().to_string() == peer_ip {
                    return Some(DialogLeg::Gateway);
                }
            }
        }
        if let Some(transfer_peer) = self
            .transfer_dialog
            .as_ref()
            .and_then(|transfer| transfer.dialog.peer.as_deref())
        {
            if let Ok(transfer_addr) = transfer_peer.parse::<SocketAddr>() {
                if transfer_addr.ip().to_string() == peer_ip {
                    return Some(DialogLeg::Transfer);
                }
            }
        }
        None
    }
}
