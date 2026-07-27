use sip_core::{Method, SipRequest};
use std::net::SocketAddr;
use tracing::{debug, info, warn};

use crate::config::EdgeConfig;
use crate::edge_state::{EdgeState, PendingDatagram};
use crate::sip::handlers::handle_request;
use crate::sip::{transaction, InviteAckKey, RequestTransactionKey};

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
        if let Some(ack_key) = InviteAckKey::from_request(&request) {
            if let Some(tx) = edge_state.take_invite_ack_transaction(&ack_key) {
                debug!(?ack_key, "ACK matched INVITE server transaction");
                let _ = tx.send(transaction::ServerTransactionEvent::Ack).await;
            } else {
                warn!(%peer, ?ack_key, "received ACK without a matching INVITE server transaction");
            }
        }
        // Each B2BUA dialog leg owns its ACK independently. The outbound leg
        // ACK is generated when its 2xx response arrives; forwarding this
        // inbound ACK would acknowledge the outbound leg a second time and is
        // ambiguous when both UAs share the same transport address.
        return Vec::new();
    }

    let datagrams = handle_request(request.clone(), peer, edge_state, edge_config).await;

    if !has_socket {
        if let Some(ref key) = transaction_key {
            let peer_str = peer.to_string();
            let peer_resps: Vec<PendingDatagram> = datagrams
                .iter()
                .filter(|datagram| is_response_for_peer(datagram, &peer_str))
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
        let (peer_resps, outbound_datagrams) = partition_server_responses(datagrams, &peer_str);
        final_datagrams.extend(outbound_datagrams);

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
                            .send(transaction::ServerTransactionEvent::send_response(resp))
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
                        .send(transaction::ServerTransactionEvent::send_response(resp))
                        .await;
                }
            }
        }
    } else {
        final_datagrams = datagrams;
    }

    final_datagrams
}

fn is_response_for_peer(datagram: &PendingDatagram, peer: &str) -> bool {
    datagram.target == peer && datagram.is_response()
}

fn partition_server_responses(
    datagrams: Vec<PendingDatagram>,
    peer: &str,
) -> (Vec<Vec<u8>>, Vec<PendingDatagram>) {
    let mut responses = Vec::new();
    let mut outbound = Vec::new();
    for datagram in datagrams {
        if is_response_for_peer(&datagram, peer) {
            responses.push(datagram.bytes);
        } else {
            outbound.push(datagram);
        }
    }
    (responses, outbound)
}

#[cfg(test)]
mod tests {
    use super::{dispatch_request, is_response_for_peer, partition_server_responses};
    use crate::config::EdgeConfig;
    use crate::edge_state::{EdgeState, PendingDatagram};
    use crate::sip::{transaction::ServerTransactionEvent, RequestTransactionKey};
    use call_core::{CallManager, RouteTable};
    use sip_core::{parse_message, SipMessageBorrow};
    use std::time::Duration;

    const SHARED_PEER: &str = "127.0.0.1:59936";

    #[test]
    fn request_to_shared_peer_is_not_classified_as_server_response() {
        let invite = PendingDatagram::new(
            SHARED_PEER.to_string(),
            concat!(
                "INVITE sip:1001@127.0.0.1:59936 SIP/2.0\r\n",
                "Via: SIP/2.0/UDP 127.0.0.1:5060;branch=z9hG4bK-shared\r\n",
                "Call-ID: outbound-leg\r\n",
                "CSeq: 1 INVITE\r\n",
                "Content-Length: 0\r\n\r\n"
            )
            .as_bytes()
            .to_vec(),
        );

        assert!(!is_response_for_peer(&invite, SHARED_PEER));
    }

    #[test]
    fn response_to_shared_peer_is_classified_as_server_response() {
        let response = PendingDatagram::new(
            SHARED_PEER.to_string(),
            concat!(
                "SIP/2.0 100 Trying\r\n",
                "Via: SIP/2.0/UDP 127.0.0.1:59936;branch=z9hG4bK-inbound\r\n",
                "Call-ID: inbound-leg\r\n",
                "CSeq: 1 INVITE\r\n",
                "Content-Length: 0\r\n\r\n"
            )
            .as_bytes()
            .to_vec(),
        );

        assert!(is_response_for_peer(&response, SHARED_PEER));
    }

    #[test]
    fn shared_peer_partition_keeps_outbound_invite_out_of_server_transaction() {
        let response = PendingDatagram::new(
            SHARED_PEER.to_string(),
            b"SIP/2.0 100 Trying\r\nCall-ID: inbound\r\nCSeq: 1 INVITE\r\nContent-Length: 0\r\n\r\n".to_vec(),
        );
        let invite = PendingDatagram::new(
            SHARED_PEER.to_string(),
            b"INVITE sip:1001@127.0.0.1 SIP/2.0\r\nCall-ID: outbound\r\nCSeq: 1 INVITE\r\nContent-Length: 0\r\n\r\n".to_vec(),
        );

        let (responses, outbound) = partition_server_responses(vec![response, invite], SHARED_PEER);

        assert_eq!(responses.len(), 1);
        assert_eq!(outbound.len(), 1);
        assert!(outbound[0].is_request());
        assert!(outbound[0].bytes.starts_with(b"INVITE "));
    }

    #[tokio::test]
    async fn ack_with_separate_branch_and_rebound_port_terminates_invite_transaction() {
        let invite_peer = SHARED_PEER.parse().unwrap();
        let ack_peer = "127.0.0.1:60000".parse().unwrap();
        let invite = parse_request(concat!(
            "INVITE sip:1001@vos-rs SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 127.0.0.1:59936;branch=z9hG4bK-invite\r\n",
            "From: <sip:1002@vos-rs>;tag=caller\r\n",
            "To: <sip:1001@vos-rs>\r\n",
            "Call-ID: shared-peer-call\r\n",
            "CSeq: 16695 INVITE\r\n",
            "Content-Length: 0\r\n\r\n"
        ));
        let ack = parse_request(concat!(
            "ACK sip:1001@vos-rs SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 127.0.0.1:59936;branch=z9hG4bK-separate-ack\r\n",
            "From: <sip:1002@vos-rs>;tag=caller\r\n",
            "To: <sip:1001@vos-rs>;tag=callee\r\n",
            "Call-ID: shared-peer-call\r\n",
            "CSeq: 16695 ACK\r\n",
            "Content-Length: 0\r\n\r\n"
        ));
        let (cdr_tx, _cdr_rx) = tokio::sync::mpsc::unbounded_channel();
        let state = EdgeState::new(CallManager::new(RouteTable::default(), cdr_tx));
        state.remember_inbound_invite(
            "shared-peer-session".to_string(),
            &invite,
            invite_peer,
            "sip:1001@127.0.0.1:5090".parse().unwrap(),
            None,
            None,
            None,
        );
        let key = RequestTransactionKey::from_request(&invite, invite_peer).unwrap();
        let (event_tx, mut event_rx) = tokio::sync::mpsc::channel(4);
        state.register_server_transaction(key, event_tx);

        let datagrams = dispatch_request(ack, ack_peer, &state, &EdgeConfig::default()).await;

        let event = tokio::time::timeout(Duration::from_millis(100), event_rx.recv())
            .await
            .expect("ACK event timeout")
            .expect("ACK event channel closed");
        assert!(matches!(event, ServerTransactionEvent::Ack));
        assert!(state.server_transactions.is_empty());
        assert!(
            datagrams.is_empty(),
            "the caller-leg ACK must be consumed by the B2BUA instead of forwarded"
        );
    }

    fn parse_request(raw: &str) -> sip_core::SipRequest {
        let SipMessageBorrow::Request(request) = parse_message(raw.as_bytes()).unwrap() else {
            panic!("expected SIP request");
        };
        request.into_owned()
    }
}
