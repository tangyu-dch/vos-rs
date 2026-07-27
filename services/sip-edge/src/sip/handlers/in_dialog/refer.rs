use std::net::SocketAddr;
use std::str::FromStr;

use sip_core::{SipRequest, SipUri};
use tracing::warn;

use super::super::percent_decode;
use crate::config::EdgeConfig;
use crate::edge_state::{
    extract_uri_from_contact, DialogLeg, DialogLegState, EdgeState, InboundTransaction,
    PendingDatagram, TransferDialogState,
};
use crate::sip::{outbound, response};

/// Handle in-dialog REFER requests per RFC 3515 (Blind Transfer B2BUA).
///
/// Flow:
/// 1. Respond 202 Accepted immediately.
/// 2. Send an initial NOTIFY carrying `SIP/2.0 100 Trying`.
/// 3. Resolve the transfer target (registered contact or routing table).
/// 4. Allocate a media relay endpoint for the transfer target leg.
/// 5. Determine the transferee leg (the opposite of `source_leg`) and pair its relay
///    port with the target relay port.
/// 6. Build the transfer dialog (`TransferDialogState`), persist it on the session,
///    and index the new transfer Call-ID.
/// 7. Emit the transfer INVITE toward the target.
///
/// REFER is fully handled here; the orchestrator returns the produced datagrams
/// immediately without cross-leg forwarding.
pub(super) async fn handle_refer(
    request: &SipRequest,
    peer: SocketAddr,
    edge_state: &EdgeState,
    edge_config: &EdgeConfig,
    transaction: &InboundTransaction,
    call_id: &str,
    source_leg: DialogLeg,
) -> Vec<PendingDatagram> {
    let mut datagrams = Vec::new();
    let refer_to_str = request.headers.get("refer-to").map(|v| v.as_str());
    let target_uri = refer_to_str.and_then(extract_uri_from_contact);

    datagrams.push(PendingDatagram::new(
        peer.to_string(),
        response::accepted_202_for_request(request),
    ));

    let Some(target_uri) = target_uri else {
        warn!(call_id, "missing or invalid Refer-To header in REFER");
        let notify_400 = outbound::build_notify_sipfrag_with_state(
            call_id,
            request
                .headers
                .get("from")
                .map(|v| v.as_str())
                .unwrap_or(""),
            request.headers.get("to").map(|v| v.as_str()).unwrap_or(""),
            transaction.dialogs.caller.local_cseq + 50,
            &edge_config.advertised_addr,
            "SIP/2.0 400 Bad Request\r\n",
            "terminated;reason=noresource",
        );
        datagrams.push(PendingDatagram::new(peer.to_string(), notify_400));
        return datagrams;
    };

    let local_cseq = transaction.dialogs.caller.local_cseq + 50;

    let notify_body = "SIP/2.0 100 Trying\r\n";
    let notify = outbound::build_notify_sipfrag(
        call_id,
        request
            .headers
            .get("from")
            .map(|v| v.as_str())
            .unwrap_or(""),
        request.headers.get("to").map(|v| v.as_str()).unwrap_or(""),
        local_cseq,
        &edge_config.advertised_addr,
        notify_body,
    );
    datagrams.push(PendingDatagram::new(peer.to_string(), notify));

    let outbound_uri = if let Some(contact) = edge_state.lookup_contact(&target_uri).await {
        SipUri::from_str(&contact.uri).ok()
    } else {
        edge_state
            .call_manager
            .routes()
            .select(&target_uri)
            .ok()
            .map(|sr| sr.outbound_uri)
    };

    let outbound_uri = match outbound_uri {
        Some(uri) => uri,
        None => {
            let notify_404 = outbound::build_notify_sipfrag_with_state(
                call_id,
                request
                    .headers
                    .get("from")
                    .map(|v| v.as_str())
                    .unwrap_or(""),
                request.headers.get("to").map(|v| v.as_str()).unwrap_or(""),
                local_cseq + 1,
                &edge_config.advertised_addr,
                "SIP/2.0 404 Not Found\r\n",
                "terminated;reason=noresource",
            );
            datagrams.push(PendingDatagram::new(peer.to_string(), notify_404));
            return datagrams;
        }
    };

    let target_relay_rtp = match edge_state
        .media_relay
        .allocate_endpoint_for_call(&edge_config.media, &transaction.session_id)
    {
        Ok(ep) => ep,
        Err(error) => {
            warn!(%error, "failed to allocate media relay endpoint for transfer target");
            let notify_503 = outbound::build_notify_sipfrag_with_state(
                call_id,
                request
                    .headers
                    .get("from")
                    .map(|v| v.as_str())
                    .unwrap_or(""),
                request.headers.get("to").map(|v| v.as_str()).unwrap_or(""),
                local_cseq + 1,
                &edge_config.advertised_addr,
                "SIP/2.0 503 Service Unavailable\r\n",
                "terminated;reason=noresource",
            );
            datagrams.push(PendingDatagram::new(peer.to_string(), notify_503));
            return datagrams;
        }
    };

    let transferee_leg = match source_leg {
        DialogLeg::Caller => DialogLeg::Gateway,
        DialogLeg::Gateway => DialogLeg::Caller,
        DialogLeg::Transfer => transaction
            .transfer_dialog
            .as_ref()
            .map(|transfer| transfer.transferee_leg)
            .unwrap_or(DialogLeg::Caller),
    };
    let transferee_relay_rtp = match transferee_leg {
        DialogLeg::Caller => transaction.caller_relay_rtp.clone(),
        DialogLeg::Gateway => transaction.gateway_relay_rtp.clone(),
        DialogLeg::Transfer => None,
    };

    if let Some(transferee_relay) = &transferee_relay_rtp {
        edge_state
            .media_relay
            .pair_ports(target_relay_rtp.port, transferee_relay.port);
    }

    let transfer_call_id = format!("vosrs-transfer-{}", uuid::Uuid::new_v4().simple());
    let transferee_dialog = match transferee_leg {
        DialogLeg::Caller => &transaction.dialogs.caller,
        DialogLeg::Gateway => &transaction.dialogs.gateway,
        DialogLeg::Transfer => {
            warn!(call_id = %call_id, "invalid transfer-to-transfer dialog linkage");
            return datagrams;
        }
    };
    let target_addr = outbound::target_addr_for(&outbound_uri);
    let transfer_dialog = TransferDialogState {
        dialog: DialogLegState {
            call_id: transfer_call_id.clone(),
            local_uri: transferee_dialog.remote_uri.clone(),
            remote_uri: target_uri.clone(),
            local_tag: format!("vosrs-t-{}", uuid::Uuid::new_v4().simple()),
            remote_tag: None,
            local_cseq: 1,
            remote_cseq: None,
            route_set: Vec::new(),
            remote_target: outbound_uri.clone(),
            peer: Some(target_addr.clone()),
        },
        transferee_leg,
    };

    let refer_sub = crate::edge_state::ReferSubscription {
        from_header: request
            .headers
            .get("from")
            .map(|v| v.as_str().to_string())
            .unwrap_or_default(),
        to_header: request
            .headers
            .get("to")
            .map(|v| v.as_str().to_string())
            .unwrap_or_default(),
        notify_cseq: local_cseq,
        referrer_peer: peer.to_string(),
        target_relay_port: Some(target_relay_rtp.port),
    };

    {
        if let Some(mut t_mut) = edge_state
            .inbound_transactions
            .get_mut(&transaction.session_id)
        {
            t_mut.refer_subscription = Some(refer_sub);
            t_mut.transfer_dialog = Some(transfer_dialog.clone());
        }
    }
    edge_state
        .inbound_transactions
        .index_dialog(&transaction.session_id, &transfer_call_id);

    let sdp_body = format!(
        "v=0\r\no=- 0 0 IN IP4 {addr}\r\ns=-\r\nc=IN IP4 {addr}\r\nt=0 0\r\nm=audio {port} RTP/AVP 0 8 101\r\na=rtpmap:0 PCMU/8000\r\na=rtpmap:8 PCMA/8000\r\na=rtpmap:101 telephone-event/8000\r\na=fmtp:101 0-16\r\n",
        addr = edge_config.advertised_addr,
        port = target_relay_rtp.port,
    );

    let replaces_header_val = if let Some(refer_to_val) = refer_to_str {
        if let Some(idx) = refer_to_val.find("?Replaces=") {
            let part = &refer_to_val[idx + "?Replaces=".len()..];
            let end_idx = part.find('>').unwrap_or(part.len());
            let encoded = &part[..end_idx];
            Some(percent_decode(encoded))
        } else if let Some(idx) = refer_to_val.find("&Replaces=") {
            let part = &refer_to_val[idx + "&Replaces=".len()..];
            let end_idx = part.find('>').unwrap_or(part.len());
            let encoded = &part[..end_idx];
            Some(percent_decode(encoded))
        } else {
            None
        }
    } else {
        None
    };

    let invite_bytes = outbound::build_transfer_invite(
        &transfer_dialog.dialog,
        &edge_config.advertised_addr,
        sdp_body.as_bytes(),
        replaces_header_val.as_deref(),
    );

    datagrams.push(PendingDatagram::new(target_addr, invite_bytes));
    datagrams
}
