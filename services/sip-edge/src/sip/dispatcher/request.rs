use sip_core::{Method, SipRequest};
use std::net::SocketAddr;
use tracing::{debug, info};

use crate::config::EdgeConfig;
use crate::edge_state::{EdgeState, PendingDatagram};
use crate::sip::handlers::handle_request;
use crate::sip::{transaction, RequestTransactionKey};

pub(crate) async fn dispatch_request(
    request: SipRequest,
    peer: SocketAddr,
    edge_state: &EdgeState,
    edge_config: &EdgeConfig,
) -> Vec<PendingDatagram> {
    info!(method = %request.method, uri = %request.uri, "received SIP request");

    let transaction_key = RequestTransactionKey::from_request(&request, peer);
    let has_socket = edge_state.get_socket().is_some();
    if !has_socket {
        if let Some(ref key) = transaction_key {
            if let Some(cached) = edge_state.test_request_cache.get(key) {
                debug!(%peer, method = %request.method, "replaying cached test response");
                return cached.clone();
            }
        }
    } else if let Some(ref key) = transaction_key {
        if let Some(tx) = edge_state.get_server_transaction(key) {
            debug!(%peer, method = %request.method, "feeding duplicate request to active Server Transaction");
            let _ = tx
                .send(transaction::ServerTransactionEvent::Request(
                    request.clone(),
                ))
                .await;
            return Vec::new();
        }
    }

    let is_ack = matches!(&request.method, Method::Ack);
    if is_ack {
        let ack_branch = request
            .headers
            .get("via")
            .and_then(|v| transaction::branch_param(v.as_str()));
        let ack_call_id = request
            .headers
            .get("call-id")
            .map(|v| v.as_str().to_string());
        let ack_cseq_num = request
            .headers
            .get("cseq")
            .and_then(|v| v.as_str().split_whitespace().next().map(|s| s.to_string()));
        let invite_key = RequestTransactionKey::new_manual(
            peer.to_string(),
            "INVITE".to_string(),
            ack_branch,
            ack_call_id,
            ack_cseq_num.map(|num| format!("{} INVITE", num)),
        );
        if let Some(tx) = edge_state.get_server_transaction(&invite_key) {
            let _ = tx
                .send(transaction::ServerTransactionEvent::Ack(request.clone()))
                .await;
        }
    }

    let datagrams = handle_request(request.clone(), peer, edge_state, edge_config).await;

    if is_ack {
        return datagrams;
    }

    if !has_socket {
        if let Some(ref key) = transaction_key {
            let peer_str = peer.to_string();
            let peer_resps: Vec<PendingDatagram> = datagrams
                .iter()
                .filter(|d| d.target == peer_str)
                .cloned()
                .collect();
            if !peer_resps.is_empty() {
                edge_state
                    .test_request_cache
                    .insert(key.clone(), peer_resps);
            }
        }
        return datagrams;
    }

    let mut final_datagrams = Vec::new();
    if let (true, Some(key)) = (has_socket, transaction_key) {
        let peer_str = peer.to_string();
        let mut peer_resps = Vec::new();

        for datagram in datagrams {
            if datagram.target == peer_str {
                peer_resps.push(datagram.bytes);
            } else {
                final_datagrams.push(datagram);
            }
        }

        if !peer_resps.is_empty() {
            let is_invite = request.method == Method::Invite;
            if is_invite {
                let has_2xx = peer_resps.iter().any(|resp| resp.starts_with(b"SIP/2.0 2"));
                if has_2xx {
                    for resp in peer_resps {
                        final_datagrams.push(PendingDatagram::new(peer_str.clone(), resp));
                    }
                } else {
                    let (tx, rx) = tokio::sync::mpsc::channel(4);
                    edge_state.register_server_transaction(key.clone(), tx.clone());
                    transaction::spawn_invite_server_transaction(
                        key,
                        request,
                        peer,
                        edge_state.get_socket(),
                        rx,
                    );
                    for resp in peer_resps {
                        let _ = tx
                            .send(transaction::ServerTransactionEvent::Response(resp))
                            .await;
                    }
                }
            } else {
                let (tx, rx) = tokio::sync::mpsc::channel(16);
                edge_state.register_server_transaction(key.clone(), tx.clone());
                transaction::spawn_non_invite_server_transaction(
                    key,
                    request,
                    peer,
                    edge_state.get_socket(),
                    rx,
                );
                for resp in peer_resps {
                    let _ = tx
                        .send(transaction::ServerTransactionEvent::Response(resp))
                        .await;
                }
            }
        }
    } else {
        final_datagrams = datagrams;
    }

    final_datagrams
}
