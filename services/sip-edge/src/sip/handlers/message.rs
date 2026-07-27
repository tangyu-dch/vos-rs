use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;

use sip_core::{SipRequest, SipUri};
use tracing::info;

use crate::config::EdgeConfig;
use crate::edge_state::{
    extract_uri_from_contact, B2buaDialogPair, DialogLegState, EdgeState, InboundTransaction,
    PendingDatagram,
};
use crate::sip::{outbound, response};

pub(crate) async fn handle_out_of_dialog_message(
    request: SipRequest,
    peer: SocketAddr,
    edge_state: &EdgeState,
    edge_config: &EdgeConfig,
) -> Vec<PendingDatagram> {
    let call_id = match request
        .headers
        .get("call-id")
        .map(|v| v.as_str().to_string())
    {
        Some(cid) => cid,
        None => {
            return vec![PendingDatagram::new(
                peer.to_string(),
                response::build_response_with_owned_headers(&request, 400, "Bad Request", &[], ""),
            )];
        }
    };

    let target_contact = edge_state.lookup_contact(&request.uri).await;

    let outbound_uri = if let Some(ref contact) = target_contact {
        SipUri::from_str(&contact.uri).ok()
    } else {
        edge_state
            .call_manager
            .routes()
            .select(&request.uri)
            .ok()
            .map(|sr| sr.outbound_uri)
    };

    let Some(outbound_uri) = outbound_uri else {
        info!(call_id = %call_id, to = %request.uri, "destination for MESSAGE not found");
        let route_error = call_core::CallError::NoRouteForDestination(request.uri.to_string());
        return vec![PendingDatagram::new(
            peer.to_string(),
            response::error_for_call_error(&request, &route_error),
        )];
    };

    let caller_route_set = request
        .headers
        .get_all("record-route")
        .map(|v| v.as_str().to_string())
        .collect::<Vec<_>>();

    let target_addr = if let Some(ref contact) = target_contact {
        contact.received_from.clone()
    } else {
        outbound::target_addr_for(&outbound_uri)
    };

    let session_id = uuid::Uuid::new_v4().to_string();
    let gateway_call_id = uuid::Uuid::new_v4().to_string();
    let caller_remote_uri = request
        .headers
        .get("from")
        .and_then(|value| extract_uri_from_contact(value.as_str()))
        .unwrap_or_else(|| request.uri.clone());
    let caller_local_uri = request
        .headers
        .get("to")
        .and_then(|value| extract_uri_from_contact(value.as_str()))
        .unwrap_or_else(|| request.uri.clone());
    let caller_remote_tag = request
        .headers
        .get("from")
        .and_then(|value| crate::sip::dialog::tag_param(value.as_str()));
    let caller_remote_cseq = request
        .headers
        .get("cseq")
        .and_then(|value| crate::sip::dialog::cseq_number(value.as_str()));
    let caller_remote_target = request
        .headers
        .get("contact")
        .and_then(|value| extract_uri_from_contact(value.as_str()))
        .unwrap_or_else(|| caller_remote_uri.clone());
    let dialogs = B2buaDialogPair {
        caller: DialogLegState {
            call_id: call_id.clone(),
            local_uri: caller_local_uri,
            remote_uri: caller_remote_uri.clone(),
            local_tag: format!("vosrs-a-{}", uuid::Uuid::new_v4().simple()),
            remote_tag: caller_remote_tag.clone(),
            local_cseq: 0,
            remote_cseq: caller_remote_cseq,
            route_set: caller_route_set.clone(),
            remote_target: caller_remote_target,
            peer: Some(peer.to_string()),
        },
        gateway: DialogLegState {
            call_id: gateway_call_id.clone(),
            local_uri: caller_remote_uri,
            remote_uri: outbound_uri.clone(),
            local_tag: format!("vosrs-b-{}", uuid::Uuid::new_v4().simple()),
            remote_tag: None,
            local_cseq: caller_remote_cseq.unwrap_or(1),
            remote_cseq: None,
            route_set: Vec::new(),
            remote_target: outbound_uri.clone(),
            peer: Some(target_addr.clone()),
        },
    };

    {
        edge_state.inbound_transactions.insert(
            call_id.clone(),
            InboundTransaction {
                session_id,
                dialogs: dialogs.clone(),
                peer: peer.to_string(),
                outbound_peer: target_contact.as_ref().map(|c| c.received_from.clone()),
                outbound_uri: outbound_uri.clone(),
                inbound_from_tag: request
                    .headers
                    .get("from")
                    .and_then(|v| crate::sip::dialog::tag_param(v.as_str())),
                inbound_to_tag: None,
                last_inbound_cseq: request
                    .headers
                    .get("cseq")
                    .and_then(|v| crate::sip::dialog::cseq_number(v.as_str())),
                last_outbound_cseq: None,
                caller_rtp: None,
                gateway_relay_rtp: None,
                gateway_rtp: None,
                caller_relay_rtp: None,
                original_request: Some(Arc::new(request.clone())),
                inbound_route_set: caller_route_set,
                outbound_route_set: Vec::new(),
                caller_contact: None,
                callee_contact: None,
                session_expires: None,
                session_refresher: None,
                last_session_refresh: None,
                prack_rseq: 0,
                gateway_100rel: false,
                refer_subscription: None,
                transfer_dialog: None,
                callee_behind_nat: target_contact.is_some(),
                fork_dialogs: Default::default(),
                max_duration_secs: None,
                established_at: None,
                invite_response_order: Arc::new(tokio::sync::Mutex::new(
                    crate::edge_state::InviteResponseOrder::default(),
                )),
            },
        );
    }

    let outbound_bytes = outbound::build_b2bua_in_dialog_request(
        &request,
        &dialogs.gateway.remote_target,
        &edge_config.advertised_addr,
        &dialogs.gateway.route_set,
        &dialogs.gateway.call_id,
        &dialogs.gateway.local_uri,
        &dialogs.gateway.local_tag,
        &dialogs.gateway.remote_uri,
        None,
        dialogs.gateway.local_cseq,
        &request.body,
    );

    vec![PendingDatagram::new(target_addr, outbound_bytes)]
}
