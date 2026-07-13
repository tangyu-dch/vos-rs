use std::net::SocketAddr;
use std::time::SystemTime;

use sip_core::SipRequest;

use crate::config::EdgeConfig;
use crate::edge_state::{EdgeState, PendingDatagram};
use crate::sip::registrar::RegisterOutcome;
use crate::sip::{response, AuthConfig, AuthDecision};

pub(crate) async fn handle_register_request(
    request: SipRequest,
    peer: SocketAddr,
    edge_state: &EdgeState,
    edge_config: &EdgeConfig,
) -> Vec<PendingDatagram> {
    let db_store = edge_state.db_store.clone();

    if matches!(
        edge_config
            .auth
            .verify_request(
                &request,
                db_store.as_ref(),
                Some(&edge_state.nonce_replay_cache)
            )
            .await,
        AuthDecision::Challenge
    ) {
        return vec![PendingDatagram::new(
            peer.to_string(),
            unauthorized_for_request(&request, &edge_config.auth),
        )];
    }

    let response = {
        let mut registrar_guard = edge_state.registrar.write().await;
        match registrar_guard
            .handle_register(&request, peer, SystemTime::now(), db_store.as_ref())
            .await
        {
            Ok(outcome) => {
                response_for_register_outcome(&request, &outcome, &edge_config.advertised_addr)
            }
            Err(error) => response::build_response_with_owned_headers(
                &request,
                400,
                "Bad Request",
                &[("X-VOS-RS-Error".to_string(), error.to_string())],
                "",
            ),
        }
    };

    vec![PendingDatagram::new(peer.to_string(), response)]
}

fn unauthorized_for_request(request: &SipRequest, auth_config: &AuthConfig) -> Vec<u8> {
    let nonce = auth_config.select_nonce();
    let challenge = auth_config.challenge_header_with_nonce(&nonce);
    response::build_response_with_owned_headers(
        request,
        401,
        "Unauthorized",
        &[("WWW-Authenticate".to_string(), challenge)],
        "",
    )
}

fn response_for_register_outcome(
    request: &SipRequest,
    outcome: &RegisterOutcome,
    advertised_addr: &str,
) -> Vec<u8> {
    let mut headers = Vec::with_capacity(outcome.contacts.len() + 1);
    headers.push(("X-VOS-RS-AOR".to_string(), outcome.aor.clone()));
    headers.extend(outcome.contacts.iter().map(|contact| {
        (
            "Contact".to_string(),
            format!("<{}>;expires={}", contact.uri, contact.expires),
        )
    }));

    headers.push((
        "Service-Route".to_string(),
        format!("<sip:{};lr>", advertised_addr),
    ));

    response::build_response_with_owned_headers(request, 200, "OK", &headers, "")
}
