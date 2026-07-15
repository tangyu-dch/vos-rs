use std::net::SocketAddr;
use std::time::SystemTime;

use sip_core::SipRequest;

use crate::cluster::{flow_key, FlowRecord};
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

    let response = {
        let mut registrar_guard = edge_state.registrar.write().await;
        match registrar_guard
            .handle_register(&request, peer, SystemTime::now(), None)
            .await
        {
            Ok(outcome) => {
                edge_state.invalidate_registration_lookup(&outcome.aor);
                // 将注册信息同步至 Redis（用于集群模式下的跨节点状态共享）
                let max_expires = outcome
                    .contacts
                    .iter()
                    .map(|c| c.expires)
                    .max()
                    .unwrap_or(0);
                let aor_key = outcome.aor.clone();
                let contacts_clone = outcome.contacts.clone();
                let redis_connection = edge_state.redis_connection();
                let flow_record = registration_transport(&request).and_then(|transport| {
                    edge_config.cluster.enabled.then(|| FlowRecord {
                        owner_node_id: edge_config.cluster.node_id.clone(),
                        transport: transport.to_string(),
                    })
                });
                let connection_flow_key = flow_key(peer);

                // 异步在后台执行 Redis 写入，防止阻塞 SIP 消息处理链路
                tokio::spawn(async move {
                    if let Some(mut conn) = redis_connection {
                        let redis_key = format!("vos_rs:reg:{}", aor_key);

                        if max_expires > 0 {
                            if let Ok(json_val) = serde_json::to_string(&contacts_clone) {
                                let _: Result<(), redis::RedisError> = redis::cmd("SET")
                                    .arg(&redis_key)
                                    .arg(json_val)
                                    .arg("EX")
                                    .arg(max_expires as u64)
                                    .query_async(&mut conn)
                                    .await;
                            }
                            if let Some(flow_record) = flow_record {
                                if let Ok(value) = serde_json::to_string(&flow_record) {
                                    let _: Result<(), redis::RedisError> = redis::cmd("SET")
                                        .arg(&connection_flow_key)
                                        .arg(value)
                                        .arg("EX")
                                        .arg(max_expires as u64)
                                        .query_async(&mut conn)
                                        .await;
                                }
                            }
                        } else {
                            // 注销，从 Redis 清除
                            let _: Result<(), redis::RedisError> = redis::cmd("DEL")
                                .arg(&redis_key)
                                .query_async(&mut conn)
                                .await;
                            let _: Result<(), redis::RedisError> = redis::cmd("DEL")
                                .arg(&connection_flow_key)
                                .query_async(&mut conn)
                                .await;
                        }
                    }
                });

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
