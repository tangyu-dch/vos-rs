use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;

use sip_core::{SipRequest, SipUri};
use tracing::info;

use crate::config::EdgeConfig;
use crate::edge_state::{EdgeState, InboundTransaction, PendingDatagram};
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

    let vias = request
        .headers
        .get_all("via")
        .map(|v| v.as_str().to_string())
        .collect::<Vec<_>>();
    let inbound_route_set = request
        .headers
        .get_all("record-route")
        .map(|v| v.as_str().to_string())
        .collect::<Vec<_>>();

    let target_addr = if let Some(ref contact) = target_contact {
        contact.received_from.clone()
    } else {
        outbound::target_addr_for(&outbound_uri)
    };

    {
        edge_state.inbound_transactions.insert(
            call_id.clone(),
            InboundTransaction {
                peer: peer.to_string(),
                outbound_peer: target_contact.as_ref().map(|c| c.received_from.clone()),
                vias,
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
                inbound_route_set,
                outbound_route_set: Vec::new(),
                caller_contact: None,
                callee_contact: None,
                session_expires: None,
                session_refresher: None,
                last_session_refresh: None,
                prack_rseq: 0,
                gateway_100rel: false,
                refer_subscription: None,
                transfer_from_header: None,
                transfer_to_header: None,
                transfer_call_id: None,
                transfer_contact: None,
                transfer_peer: None,
                transferee_is_caller: false,
                callee_behind_nat: target_contact.is_some(),
                active_forks: Vec::new(),
                max_duration_secs: None,
                established_at: None,
                invite_response_order: Arc::new(tokio::sync::Mutex::new(
                    crate::edge_state::InviteResponseOrder::default(),
                )),
            },
        );
    }

    let outbound_bytes =
        outbound::build_outbound_message(&request, &outbound_uri, &edge_config.advertised_addr);

    vec![PendingDatagram::new(target_addr, outbound_bytes)]
}
