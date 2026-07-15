use std::net::SocketAddr;
use std::time::SystemTime;

use sip_core::SipRequest;

use crate::cluster::{flow_key, FlowRecord, RegistrationSyncCommand};
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
    let auth_res = edge_state
        .verify_sip_auth(&edge_config.auth, &request)
        .await;
    match auth_res {
        AuthDecision::Challenge => {
            return vec![PendingDatagram::new(
                peer.to_string(),
                unauthorized_for_request(&request, &edge_config.auth),
            )];
        }
        AuthDecision::ChallengeWithFailure => {
            edge_state.sbc_engine.register_auth_failure(peer.ip());
            return vec![PendingDatagram::new(
                peer.to_string(),
                unauthorized_for_request(&request, &edge_config.auth),
            )];
        }
        _ => {}
    }

    let outcome = {
        let mut registrar_guard = edge_state.registrar.write().await;
        registrar_guard
            .handle_register(&request, peer, SystemTime::now(), None)
            .await
    };

    let response = match outcome {
        Ok(outcome) => {
            edge_state.invalidate_registration_lookup(&outcome.aor);
            if let Some(sync) = edge_state.registration_sync() {
                let command = registration_sync_command(&request, peer, edge_config, &outcome);
                if let Some(command) = command {
                    if let Err(error) = sync.send(command).await {
                        tracing::error!(%error, aor = %outcome.aor, "REGISTER Redis 同步队列已关闭");
                        return vec![PendingDatagram::new(
                            peer.to_string(),
                            response::build_response_with_owned_headers(
                                &request,
                                503,
                                "Service Unavailable",
                                &[],
                                "",
                            ),
                        )];
                    }
                }
            }
            response_for_register_outcome(&request, &outcome, &edge_config.advertised_addr)
        }
        Err(error) => response::build_response_with_owned_headers(
            &request,
            400,
            "Bad Request",
            &[("X-VOS-RS-Error".to_string(), error.to_string())],
            "",
        ),
    };

    vec![PendingDatagram::new(peer.to_string(), response)]
}

fn registration_sync_command(
    request: &SipRequest,
    peer: SocketAddr,
    edge_config: &EdgeConfig,
    outcome: &RegisterOutcome,
) -> Option<RegistrationSyncCommand> {
    let registration_key = format!("vos_rs:reg:{}", outcome.aor);
    let flow_key = flow_key(peer);
    let ttl_secs = outcome
        .contacts
        .iter()
        .map(|contact| u64::from(contact.expires))
        .max()
        .unwrap_or(0);
    if ttl_secs == 0 {
        return Some(RegistrationSyncCommand::Delete {
            registration_key,
            flow_key,
        });
    };

    let contacts_json = serde_json::to_string(&outcome.contacts).ok()?;
    let flow = registration_transport(request)
        .and_then(|transport| {
            edge_config.cluster.enabled.then(|| FlowRecord {
                owner_node_id: edge_config.cluster.node_id.clone(),
                transport: transport.to_string(),
            })
        })
        .and_then(|record| serde_json::to_string(&record).ok())
        .map(|flow_json| (flow_key, flow_json));
    Some(RegistrationSyncCommand::Upsert {
        registration_key,
        contacts_json,
        flow,
        ttl_secs,
    })
}

fn registration_transport(request: &SipRequest) -> Option<&'static str> {
    let via = request.headers.get("via")?.as_str().to_ascii_uppercase();
    if via.contains("SIP/2.0/WSS") {
        Some("wss")
    } else if via.contains("SIP/2.0/WS") {
        Some("ws")
    } else if via.contains("SIP/2.0/TLS") {
        Some("tls")
    } else if via.contains("SIP/2.0/TCP") {
        Some("tcp")
    } else {
        None
    }
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
