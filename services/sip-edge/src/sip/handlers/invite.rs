use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::atomic::Ordering;
use std::time::SystemTime;

use sip_core::{SipRequest, SipUri};
use tracing::{debug, info, warn};

use super::{
    prepare_rewritten_sdp, proxy_unauthorized_for_request, register_relay_target,
    response_for_media_error,
};
use crate::config::EdgeConfig;
use crate::edge_state::{EdgeState, PendingDatagram};
use crate::sip::{outbound, response, AuthDecision};

pub(crate) async fn handle_invite_request(
    request: SipRequest,
    peer: SocketAddr,
    edge_state: &EdgeState,
    edge_config: &EdgeConfig,
) -> Vec<PendingDatagram> {
    if edge_state.draining.load(Ordering::Relaxed) {
        info!(
            call_id = %request.headers.get("call-id").map(|v| v.as_str()).unwrap_or(""),
            "rejecting new INVITE with 503 during drain"
        );
        return vec![PendingDatagram::new(
            peer.to_string(),
            response::response_503_service_unavailable(&request),
        )];
    }

    {
        let rules = edge_state
            .anti_fraud_rules
            .read()
            .unwrap_or_else(|e| e.into_inner());
        let caller = EdgeState::username_from_request(&request).unwrap_or_default();
        let callee = request
            .headers
            .get("to")
            .and_then(|v| {
                let s = v.as_str();
                let start = s.find("sip:").map(|i| i + 4)?;
                let end = s[start..].find('@')?;
                Some(s[start..start + end].to_string())
            })
            .unwrap_or_default();
        let client_ip = peer.ip().to_string();

        for rule in rules.iter() {
            if !rule.enabled {
                continue;
            }
            match rule.rule_type.as_str() {
                "callee_blacklist" | "caller_blacklist" | "blacklist" => {
                    if (!rule.target_value.is_empty() && callee.starts_with(&rule.target_value))
                        || (!rule.target_value.is_empty() && caller.starts_with(&rule.target_value))
                    {
                        warn!(%caller, %callee, target = %rule.target_value, "呼叫被防盗打黑名单拦截");
                        return vec![PendingDatagram::new(
                            peer.to_string(),
                            response::build_response_with_owned_headers(
                                &request,
                                403,
                                "Forbidden - Anti-Fraud Blacklist Match",
                                &[(
                                    "X-VOS-RS-Error".to_string(),
                                    "Callee number is blacklisted".to_string(),
                                )],
                                "",
                            ),
                        )];
                    }
                }
                "user_concurrency" | "limit_concurrent" | "ip_concurrency" => {
                    let limit = rule.limit_number.unwrap_or(0) as u32;
                    if rule.target_value == client_ip {
                        let current_ip_concurrency = edge_state
                            .inbound_transactions
                            .iter()
                            .filter(|entry| entry.value().peer.contains(&client_ip))
                            .count() as u32;
                        if current_ip_concurrency >= limit {
                            warn!(%client_ip, current_ip_concurrency, limit, "防盗打限制：IP 并发超限被拦截");
                            return vec![PendingDatagram::new(
                                peer.to_string(),
                                response::build_response_with_owned_headers(
                                    &request,
                                    503,
                                    "Service Unavailable - IP Concurrency Limit Exceeded",
                                    &[(
                                        "X-VOS-RS-Error".to_string(),
                                        "IP concurrent call limit exceeded".to_string(),
                                    )],
                                    "",
                                ),
                            )];
                        }
                    } else if rule.target_value == caller && !caller.is_empty() {
                        let active_count = edge_state.user_concurrent_count(&caller);
                        if active_count >= limit {
                            warn!(%caller, active_count, limit, "防盗打限制：用户并发超限被拦截");
                            return vec![PendingDatagram::new(
                                peer.to_string(),
                                response::build_response_with_owned_headers(
                                    &request,
                                    503,
                                    "Service Unavailable - User Concurrency Limit Exceeded",
                                    &[(
                                        "X-VOS-RS-Error".to_string(),
                                        "User concurrent call limit exceeded".to_string(),
                                    )],
                                    "",
                                ),
                            )];
                        }
                    }
                }
                _ => {}
            }
        }
    }

    if let Some(username) = request.headers.get("from").and_then(|v| {
        let s = v.as_str();
        let start = s.find("sip:")?;
        let end = s[start..].find('@')?;
        Some(s[start + 4..start + end].to_string())
    }) {
        // O(1) 并发数查询，替代原来 O(n) inbound_transactions.iter() 扫描
        let active_count = edge_state.user_concurrent_count(&username);

        if active_count >= edge_config.sbc_max_concurrency {
            warn!(%username, active_count, limit = edge_config.sbc_max_concurrency, "rejecting INVITE due to user concurrency limit exceeded");
            return vec![PendingDatagram::new(
                peer.to_string(),
                response::build_response_with_owned_headers(
                    &request,
                    486,
                    "Busy Here - Concurrency Limit Exceeded",
                    &[],
                    "",
                ),
            )];
        }
    }

    let from_gw = edge_state.is_peer_gateway(peer).await;
    if !from_gw {
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
                proxy_unauthorized_for_request(&request, &edge_config.auth),
            )];
        }
    }

    let caller_domain = EdgeState::domain_from_request(&request);
    let callee_domain = request.uri.host.clone();

    if let Some(ref caller_dom) = caller_domain {
        if callee_domain != *caller_dom {
            let registered_contact = edge_state
                .registrar
                .read()
                .await
                .lookup_contact(
                    &request.uri,
                    SystemTime::now(),
                    edge_state.db_store.as_ref(),
                )
                .await;
            if registered_contact.is_some() {
                warn!(
                    caller = %request.headers.get("from").map(|v| v.as_str()).unwrap_or(""),
                    callee = %request.uri,
                    "cross-tenant call forbidden by domain isolation"
                );
                return vec![PendingDatagram::new(
                    peer.to_string(),
                    response::build_response_with_owned_headers(
                        &request,
                        403,
                        "Forbidden - Cross-Tenant Calling Disabled",
                        &[(
                            "X-VOS-RS-Error".to_string(),
                            "Cross-tenant calling is disabled".to_string(),
                        )],
                        "",
                    ),
                )];
            }
        }
    }

    let registered_contact = edge_state
        .registrar
        .read()
        .await
        .lookup_contact(
            &request.uri,
            SystemTime::now(),
            edge_state.db_store.as_ref(),
        )
        .await;

    let response::RequestHandling {
        response,
        mut outbound_invite,
    } = if let Some(ref contact) = registered_contact {
        if let Ok(outbound_uri) = SipUri::from_str(&contact.uri) {
            response::response_for_invite_to_uri(&request, &edge_state.call_manager, outbound_uri)
        } else {
            let mut health = edge_state
                .gateway_health
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            response::response_for_request_with_health(
                &request,
                &edge_state.call_manager,
                Some(&mut health),
            )
        }
    } else {
        let mut health = edge_state
            .gateway_health
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        response::response_for_request_with_health(
            &request,
            &edge_state.call_manager,
            Some(&mut health),
        )
    };

    if let Some(ref mut plan) = outbound_invite {
        if registered_contact.is_none() && !plan.gateway_id.is_empty() {
            if let Some(ref caller_dom) = caller_domain {
                if plan.gateway_id.contains('.') && !plan.gateway_id.contains(caller_dom) {
                    warn!(
                        gateway_id = %plan.gateway_id,
                        caller_domain = %caller_dom,
                        "tenant domain mismatch for outbound gateway"
                    );
                    return vec![PendingDatagram::new(
                        peer.to_string(),
                        response::build_response_with_owned_headers(
                            &request,
                            403,
                            "Forbidden - Gateway Domain Mismatch",
                            &[(
                                "X-VOS-RS-Error".to_string(),
                                "Gateway is not allowed for this tenant domain".to_string(),
                            )],
                            "",
                        ),
                    )];
                }
            }
        }
    }

    if let Some(ref contact) = registered_contact {
        if let Some(ref mut plan) = outbound_invite {
            plan.target_override_addr = Some(contact.received_from.clone());
        }
    }

    let mut calculated_max_duration: Option<u32> = request
        .headers
        .get("x-test-max-duration")
        .and_then(|v| v.as_str().trim().parse::<u32>().ok());

    // Pre-call balance check: reject if caller has no balance.
    if let Some(ref _plan) = outbound_invite {
        if let Some(ref db) = edge_state.db_store {
            let caller_user =
                crate::edge_state::EdgeState::username_from_request(&request).unwrap_or_default();
            let callee = request.uri.user.as_deref().unwrap_or("");
            if !caller_user.is_empty() {
                match db.check_balance(&caller_user, callee).await {
                    Ok((has_balance, balance, rate)) => {
                        if !has_balance {
                            warn!(caller = %caller_user, balance, rate, "pre-call balance check failed: insufficient balance");
                            return vec![PendingDatagram::new(
                                peer.to_string(),
                                response::error_for_call_error(
                                    &request,
                                    &call_core::CallError::GatewayUnavailable(
                                        "余额不足".to_string(),
                                    ),
                                ),
                            )];
                        }
                        if rate > 0.0 {
                            calculated_max_duration = Some(((balance / rate) * 60.0) as u32);
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, "pre-call balance check error, allowing call");
                    }
                }
            }
        }
    }

    if let Some(outbound_invite) = outbound_invite.as_ref() {
        let rewritten_sdp = match prepare_rewritten_sdp(
            &request.headers,
            &request.body,
            &edge_state.media_relay,
            &edge_config.media,
            "inbound INVITE offer",
        ) {
            Ok(rewritten_sdp) => rewritten_sdp,
            Err(error) => {
                warn!(%error, "rejecting INVITE after media negotiation failure");
                return vec![PendingDatagram::new(
                    peer.to_string(),
                    response_for_media_error(&request, &error),
                )];
            }
        };
        if let Some(rewritten_sdp) = &rewritten_sdp {
            if let Some(caller_rtp) = &rewritten_sdp.original_endpoint {
                register_relay_target(
                    &edge_state.media_relay,
                    &rewritten_sdp.relay_endpoint,
                    caller_rtp,
                    "gateway-to-caller RTP",
                );
            }
        }

        let internal_call_id = request
            .headers
            .get("call-id")
            .map(|v| v.as_str().to_string())
            .unwrap_or_default();

        let mut candidates = Vec::new();
        if let Some(call) = edge_state
            .call_manager
            .get(&call_core::CallId::new(internal_call_id.clone()))
        {
            candidates = call.candidates.clone();
        }

        edge_state.remember_inbound_invite(
            &request,
            peer,
            outbound_invite.outbound_uri.clone(),
            rewritten_sdp
                .as_ref()
                .and_then(|sdp| sdp.original_endpoint.clone()),
            rewritten_sdp.as_ref().map(|sdp| sdp.relay_endpoint.clone()),
            outbound_invite.target_override_addr.is_some(),
            calculated_max_duration,
        );

        let mut datagrams = vec![PendingDatagram::new(peer.to_string(), response)];
        let path = if let Some(ref contact) = registered_contact {
            contact.path.as_slice()
        } else {
            &[]
        };

        let nonce_input = format!(
            "{}-{}",
            internal_call_id,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        let md5_hex = format!("{:x}", md5::compute(nonce_input.as_bytes()));

        let forking_enabled = request
            .headers
            .get("x-forking-enabled")
            .map(|v| v.as_str().trim().to_lowercase() == "true")
            .unwrap_or(false)
            || request
                .headers
                .get("x-call-forking")
                .map(|v| v.as_str().trim().to_lowercase() == "true")
                .unwrap_or(false);

        if forking_enabled && candidates.len() > 1 {
            let fork_candidates = candidates.iter().take(3).cloned().collect::<Vec<_>>();
            let mut forks_to_save = Vec::new();
            for (i, candidate) in fork_candidates.iter().enumerate() {
                let external_call_id = format!(
                    "{}-f{}@{}",
                    &md5_hex[..12],
                    i,
                    edge_config
                        .advertised_addr
                        .split(':')
                        .next()
                        .unwrap_or("vos-rs")
                );
                edge_state.register_call_id_mapping(&internal_call_id, &external_call_id);
                forks_to_save.push((
                    external_call_id.clone(),
                    candidate.target.gateway_id.as_str().to_string(),
                ));

                let target = outbound::target_addr_for(&candidate.outbound_uri);
                let bytes = outbound::build_outbound_invite_with_session_timer_and_call_id(
                    &request,
                    &candidate.outbound_uri,
                    &edge_config.advertised_addr,
                    rewritten_sdp
                        .as_ref()
                        .map(|sdp| sdp.body.as_slice())
                        .unwrap_or(request.body.as_slice()),
                    edge_config.session_expires_gateway,
                    path,
                    &external_call_id,
                );
                datagrams.push(PendingDatagram::new(target, bytes));

                let gw_id = candidate.target.gateway_id.as_str().to_string();
                if !gw_id.is_empty() {
                    let status = {
                        let mut health = edge_state
                            .gateway_health
                            .lock()
                            .unwrap_or_else(|e| e.into_inner());
                        health.increment_active(&gw_id);
                        health.get_gateway_status(&gw_id)
                    };
                    crate::timers::persist_gateway_health(edge_state, gw_id.clone(), status);
                }
            }

            if let Some(mut t_mut) = edge_state.inbound_transactions.get_mut(&internal_call_id) {
                t_mut.active_forks = forks_to_save;
            }
        } else {
            let external_call_id = format!(
                "{}@{}",
                &md5_hex[..16],
                edge_config
                    .advertised_addr
                    .split(':')
                    .next()
                    .unwrap_or("vos-rs")
            );
            edge_state.register_call_id_mapping(&internal_call_id, &external_call_id);
            debug!(
                internal_call_id,
                external_call_id, "topology hiding: registered Call-ID mapping"
            );

            let target = if let Some(ref override_addr) = outbound_invite.target_override_addr {
                override_addr.clone()
            } else {
                outbound::target_addr_for(&outbound_invite.outbound_uri)
            };

            let bytes = outbound::build_outbound_invite_with_session_timer_and_call_id(
                &request,
                &outbound_invite.outbound_uri,
                &edge_config.advertised_addr,
                rewritten_sdp
                    .as_ref()
                    .map(|sdp| sdp.body.as_slice())
                    .unwrap_or(request.body.as_slice()),
                edge_config.session_expires_gateway,
                path,
                &external_call_id,
            );
            datagrams.push(PendingDatagram::new(target, bytes));

            if !outbound_invite.gateway_id.is_empty() {
                let status = {
                    let mut health = edge_state
                        .gateway_health
                        .lock()
                        .unwrap_or_else(|e| e.into_inner());
                    health.increment_active(&outbound_invite.gateway_id);
                    health.get_gateway_status(&outbound_invite.gateway_id)
                };
                crate::timers::persist_gateway_health(
                    edge_state,
                    outbound_invite.gateway_id.clone(),
                    status,
                );
            }
        }

        return datagrams;
    }

    vec![PendingDatagram::new(peer.to_string(), response)]
}
