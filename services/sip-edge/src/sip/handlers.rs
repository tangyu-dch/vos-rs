use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Instant, SystemTime};

use call_core::CallError;
use sdp_core::{RtpEndpoint, SessionDescription};
use sip_core::{HeaderMap, HeaderName, HeaderValue, Method, SipRequest, SipUri};
use tracing::{debug, info, warn};

use crate::config::EdgeConfig;
use crate::edge_state::{
    extract_uri_from_contact, parse_target_addr_from_route, sip_uri_from_peer, DialogLeg,
    EdgeState, InboundTransaction, PendingDatagram,
};
use crate::media::{self, MediaConfig, MediaRelayState};
use crate::sip::registrar::RegisterOutcome;
use crate::sip::{outbound, response, AuthConfig, AuthDecision, DialogValidationError};
use crate::timers::calculate_mos_for_legs;

pub(crate) async fn handle_request(
    request: SipRequest,
    peer: SocketAddr,
    edge_state: &EdgeState,
    edge_config: &EdgeConfig,
) -> Vec<PendingDatagram> {
    if matches!(&request.method, Method::Register) {
        return handle_register_request(request, peer, edge_state, edge_config).await;
    }

    if matches!(&request.method, Method::Message) {
        let to_tag = request
            .headers
            .get("to")
            .and_then(|v| crate::sip::dialog::tag_param(v.as_str()));
        if to_tag.is_some() {
            return handle_in_dialog_request(request, peer, edge_state, edge_config).await;
        } else {
            return handle_out_of_dialog_message(request, peer, edge_state, edge_config).await;
        }
    }

    if outbound::is_forwardable_in_dialog_method(&request.method) {
        return handle_in_dialog_request(request, peer, edge_state, edge_config).await;
    }

    // Mid-dialog Re-INVITE: To header contains a tag, meaning this is within an established dialog
    if matches!(&request.method, Method::Invite) {
        let to_tag = request
            .headers
            .get("to")
            .and_then(|v| crate::sip::dialog::tag_param(v.as_str()));
        if to_tag.is_some() {
            return handle_in_dialog_request(request, peer, edge_state, edge_config).await;
        }
    }

    // RFC 3262: PRACK is always an in-dialog message (has To-tag) — route to in-dialog handler
    if matches!(&request.method, Method::Prack) {
        return handle_in_dialog_request(request, peer, edge_state, edge_config).await;
    }

    if edge_state.draining.load(Ordering::Relaxed) && matches!(&request.method, Method::Invite) {
        info!(
            call_id = %request.headers.get("call-id").map(|v| v.as_str()).unwrap_or(""),
            "rejecting new INVITE with 503 during drain"
        );
        return vec![PendingDatagram::new(
            peer.to_string(),
            response::response_503_service_unavailable(&request),
        )];
    }

    if matches!(&request.method, Method::Invite) {
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
                            || (!rule.target_value.is_empty()
                                && caller.starts_with(&rule.target_value))
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
                                .count()
                                as u32;
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
    }

    let registered_contact = {
        if matches!(&request.method, Method::Invite) {
            edge_state
                .registrar
                .read()
                .await
                .lookup_contact(
                    &request.uri,
                    SystemTime::now(),
                    edge_state.db_store.as_ref(),
                )
                .await
        } else {
            None
        }
    };
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

    if let Some(ref contact) = registered_contact {
        if let Some(ref mut plan) = outbound_invite {
            plan.target_override_addr = Some(contact.received_from.clone());
        }
    }

    // Pre-call balance check: reject if caller has no balance.
    if let Some(ref _plan) = outbound_invite {
        if let Some(ref db) = edge_state.db_store {
            let caller_user = crate::edge_state::EdgeState::username_from_request(&request)
                .unwrap_or_default();
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
                                    &call_core::CallError::GatewayUnavailable("余额不足".to_string()),
                                ),
                            )];
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
        edge_state.remember_inbound_invite(
            &request,
            peer,
            outbound_invite.outbound_uri.clone(),
            rewritten_sdp
                .as_ref()
                .and_then(|sdp| sdp.original_endpoint.clone()),
            rewritten_sdp.as_ref().map(|sdp| sdp.relay_endpoint.clone()),
            outbound_invite.target_override_addr.is_some(),
        );

        let mut datagrams = vec![PendingDatagram::new(peer.to_string(), response)];
        let target = if let Some(ref override_addr) = outbound_invite.target_override_addr {
            override_addr.clone()
        } else {
            outbound::target_addr_for(&outbound_invite.outbound_uri)
        };
        let path = if let Some(ref contact) = registered_contact {
            contact.path.as_slice()
        } else {
            &[]
        };

        // Topology Hiding: generate a fresh Call-ID for the outbound (gateway) leg.
        // The inbound Call-ID is retained internally; the gateway sees only the external one.
        let internal_call_id = request
            .headers
            .get("call-id")
            .map(|v| v.as_str().to_string())
            .unwrap_or_default();
        let nonce_input = format!(
            "{}-{}",
            internal_call_id,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        );
        let md5_hex = format!("{:x}", md5::compute(nonce_input.as_bytes()));
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

        // Increment active call count for the selected gateway.
        if !outbound_invite.gateway_id.is_empty() {
            let status = {
                let mut health = edge_state
                    .gateway_health
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                health.increment_active(&outbound_invite.gateway_id);
                health.get_gateway_status(&outbound_invite.gateway_id)
            };
            crate::timers::persist_gateway_health(edge_state, outbound_invite.gateway_id.clone(), status);
        }

        return datagrams;
    }

    vec![PendingDatagram::new(peer.to_string(), response)]
}

async fn handle_out_of_dialog_message(
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

    let target_contact = {
        let registrar = edge_state.registrar.read().await;
        registrar
            .lookup_contact(
                &request.uri,
                SystemTime::now(),
                edge_state.db_store.as_ref(),
            )
            .await
    };

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
            },
        );
    }

    let outbound_bytes =
        outbound::build_outbound_message(&request, &outbound_uri, &edge_config.advertised_addr);

    vec![PendingDatagram::new(target_addr, outbound_bytes)]
}

async fn handle_register_request(
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

fn proxy_unauthorized_for_request(request: &SipRequest, auth_config: &AuthConfig) -> Vec<u8> {
    let nonce = auth_config.select_nonce();
    let challenge = auth_config.challenge_header_with_nonce(&nonce);
    response::build_response_with_owned_headers(
        request,
        407,
        "Proxy Authentication Required",
        &[("Proxy-Authenticate".to_string(), challenge)],
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

pub(crate) async fn handle_in_dialog_request(
    request: SipRequest,
    peer: SocketAddr,
    edge_state: &EdgeState,
    edge_config: &EdgeConfig,
) -> Vec<PendingDatagram> {
    let Some(call_id) = request
        .headers
        .get("call-id")
        .map(|v| v.as_str().to_string())
    else {
        if matches!(&request.method, Method::Ack) {
            return Vec::new();
        }

        let error = CallError::MissingRequiredHeader("Call-ID");
        return vec![PendingDatagram::new(
            peer.to_string(),
            response::error_for_call_error(&request, &error),
        )];
    };

    let mut mutable_request = request;

    let (transaction, source_leg, is_target) = {
        let lookup_cid = edge_state
            .bridged_transfers
            .get(call_id.as_str())
            .map(|r| r.clone());
        let actual_cid = lookup_cid.as_deref().unwrap_or(call_id.as_str());

        let Some(mut t) = edge_state.inbound_transactions.get_mut(actual_cid) else {
            if matches!(&mutable_request.method, Method::Ack) {
                return Vec::new();
            }

            let error = call_error_for_unknown_request(&mutable_request);
            return vec![PendingDatagram::new(
                peer.to_string(),
                response::error_for_call_error(&mutable_request, &error),
            )];
        };

        let is_target = t.transfer_call_id.as_deref() == Some(call_id.as_str());

        let source_leg = if is_target {
            let leg = if t.transferee_is_caller {
                DialogLeg::Gateway
            } else {
                DialogLeg::Caller
            };

            // Rewrite Call-ID, From, To for original leg B
            mutable_request.headers.replace(
                HeaderName::new("call-id").unwrap(),
                HeaderValue::new(actual_cid),
            );
            if let Some(orig_req) = &t.original_request {
                let (from_val, to_val) = if t.transferee_is_caller {
                    (
                        orig_req.headers.get("to").cloned(),
                        orig_req.headers.get("from").cloned(),
                    )
                } else {
                    (
                        orig_req.headers.get("from").cloned(),
                        orig_req.headers.get("to").cloned(),
                    )
                };
                if let Some(f) = from_val {
                    mutable_request.headers.replace(
                        HeaderName::new("from").unwrap(),
                        HeaderValue::new(f.as_str()),
                    );
                }
                if let Some(o) = to_val {
                    mutable_request
                        .headers
                        .replace(HeaderName::new("to").unwrap(), HeaderValue::new(o.as_str()));
                }
            }

            leg
        } else {
            let is_bridged = t.transfer_call_id.is_some();

            let (leg, cseq_update) = match t.validate_in_dialog_request(&mutable_request, peer) {
                Ok(result) => result,
                Err(error) => {
                    return vec![PendingDatagram::new(
                        peer.to_string(),
                        response_for_dialog_validation_error(&mutable_request, &error),
                    )];
                }
            };

            // Update cseq outside of validation
            if let Some(cseq) = cseq_update {
                match leg {
                    DialogLeg::Caller => t.last_inbound_cseq = Some(cseq),
                    DialogLeg::Gateway => t.last_outbound_cseq = Some(cseq),
                }
            }

            if is_bridged {
                // Rewrite Call-ID, From, To for target leg C
                if let Some(ref tf_cid) = t.transfer_call_id {
                    mutable_request.headers.insert(
                        HeaderName::new("call-id").unwrap(),
                        HeaderValue::new(tf_cid),
                    );
                }
                if let Some(ref tf_from) = t.transfer_from_header {
                    mutable_request
                        .headers
                        .insert(HeaderName::new("from").unwrap(), HeaderValue::new(tf_from));
                }
                if let Some(ref tf_to) = t.transfer_to_header {
                    mutable_request
                        .headers
                        .insert(HeaderName::new("to").unwrap(), HeaderValue::new(tf_to));
                }
            }

            leg
        };

        (t.clone(), source_leg, is_target)
    };

    let mut datagrams = Vec::new();
    match &mutable_request.method {
        Method::Bye | Method::Cancel => {
            let mut caller_rtcp = None;
            let mut gateway_rtcp = None;

            if let Some(endpoint) = &transaction.caller_relay_rtp {
                caller_rtcp = Some(
                    edge_state
                        .media_relay
                        .metrics_for_port(endpoint.port)
                        .rtcp_quality,
                );
            }
            if let Some(endpoint) = &transaction.gateway_relay_rtp {
                gateway_rtcp = Some(
                    edge_state
                        .media_relay
                        .metrics_for_port(endpoint.port)
                        .rtcp_quality,
                );
            }

            let metrics = if caller_rtcp.is_some() || gateway_rtcp.is_some() {
                Some(calculate_mos_for_legs(
                    caller_rtcp.as_ref(),
                    gateway_rtcp.as_ref(),
                ))
            } else {
                None
            };

            let cid = mutable_request
                .headers
                .get("call-id")
                .map(|val| val.as_str())
                .unwrap_or("missing-call-id");
            let dtmf_digits = edge_state.media_relay.get_dtmf_digits(cid);
            if let Some(digits) = &dtmf_digits {
                info!(call_id = cid, digits = %digits, "collected DTMF digits for call");
            }
            edge_state.media_relay.clear_dtmf_digits(cid);

            // Collect DTMF audit events for persistence to the detail table.
            let dtmf_events = edge_state.media_relay.take_dtmf_events(cid);
            if !dtmf_events.is_empty() {
                info!(
                    call_id = cid,
                    count = dtmf_events.len(),
                    "collected DTMF audit events for call"
                );
                if let Some(ref db) = edge_state.db_store {
                    if let Err(error) = db.insert_dtmf_events_batch(&dtmf_events).await {
                        warn!(%error, call_id = cid, "failed to persist DTMF audit events");
                    }
                }
            } else {
                edge_state.media_relay.clear_dtmf_events(cid);
            }

            // Clean up bridged mappings
            if transaction.transfer_call_id.is_some() {
                if let Some(ref tf_cid) = transaction.transfer_call_id {
                    edge_state.bridged_transfers.remove(tf_cid);
                }
                let lookup_cid = edge_state
                    .bridged_transfers
                    .get(call_id.as_str())
                    .map(|r| r.clone());
                let actual_cid = lookup_cid.as_deref().unwrap_or(call_id.as_str());
                edge_state.bridged_transfers.remove(actual_cid);
            }

            match edge_state.call_manager.handle_inbound_termination(
                &mutable_request,
                metrics,
                dtmf_digits,
            ) {
                Ok(outcome) => {
                    // Decrement active call count for the gateway.
                    let call_id_str = mutable_request
                        .headers
                        .get("call-id")
                        .map(|v| v.as_str())
                        .unwrap_or("");
                    if let Some(gw_id) = edge_state.call_manager.current_gateway_id(call_id_str) {
                        let status = {
                            let mut health = edge_state
                                .gateway_health
                                .lock()
                                .unwrap_or_else(|e| e.into_inner());
                            health.decrement_active(&gw_id);
                            health.get_gateway_status(&gw_id)
                        };
                        crate::timers::persist_gateway_health(edge_state, gw_id.clone(), status);
                    }

                    // Real-time billing: settle the call.
                    if let Some(ref db) = edge_state.db_store {
                        let call = edge_state.call_manager.get(&outcome.call_id);
                        if let Some(call) = call {
                            let caller_user = call
                                .caller
                                .as_deref()
                                .and_then(|s| {
                                    // Extract user from "sip:user@host" or "<sip:user@host>"
                                    let idx = s.find("sip:")?;
                                    let rest = &s[idx + 4..];
                                    let end = rest.find(['@', ';', '>']).unwrap_or(rest.len());
                                    if end == 0 { None } else { Some(&rest[..end]) }
                                });
                            let callee = call.inbound.remote_uri.user.as_deref().unwrap_or("");
                            let duration_ms = call
                                .ended_at
                                .and_then(|e| e.duration_since(call.started_at).ok())
                                .map(|d| d.as_millis() as i64)
                                .unwrap_or(0);
                            if let Some(user) = caller_user {
                                let cid = outcome.call_id.as_str().to_string();
                                let db = db.clone();
                                let user = user.to_string();
                                let callee = callee.to_string();
                                tokio::spawn(async move {
                                    match db.settle_call(&cid, &user, &callee, duration_ms).await {
                                        Ok(Some(new_bal)) => {
                                            tracing::info!(call_id = %cid, user = %user, new_bal, "实时计费结算完成");
                                        }
                                        Ok(None) => {}
                                        Err(e) => {
                                            tracing::warn!(call_id = %cid, error = %e, "实时计费结算失败");
                                        }
                                    }
                                });
                            }
                        }
                    }

                    edge_state.clear_media_targets(&transaction);
                    if transaction.transfer_call_id.is_some() {
                        let transferee_port = if transaction.transferee_is_caller {
                            transaction.caller_relay_rtp.as_ref().map(|ep| ep.port)
                        } else {
                            transaction.gateway_relay_rtp.as_ref().map(|ep| ep.port)
                        };
                        if let Some(tp) = transferee_port {
                            if let Some(cp) = edge_state.media_relay.peer_port_for(tp) {
                                edge_state.media_relay.clear_target(cp);
                            }
                        }
                    }

                    datagrams.push(PendingDatagram::new(
                        peer.to_string(),
                        response::ok_for_request(&mutable_request),
                    ));
                }
                Err(error) => {
                    datagrams.push(PendingDatagram::new(
                        peer.to_string(),
                        response::error_for_call_error(&mutable_request, &error),
                    ));
                    return datagrams;
                }
            }
        }
        Method::Info => {
            let content_type = mutable_request
                .headers
                .get("content-type")
                .map(|v| v.as_str())
                .unwrap_or("");
            if let Some(digit) = parse_sip_info_dtmf(content_type, &mutable_request.body) {
                let cid = mutable_request
                    .headers
                    .get("call-id")
                    .map(|v| v.as_str())
                    .unwrap_or("");
                if !cid.is_empty() {
                    edge_state.media_relay.register_info_dtmf_digit(cid, digit);
                }
            }

            datagrams.push(PendingDatagram::new(
                peer.to_string(),
                response::ok_for_request(&mutable_request),
            ));
        }
        Method::Prack => {
            let rack_valid = if let Some(rack) = mutable_request.headers.get("rack") {
                let parts = rack.as_str().split_whitespace().collect::<Vec<_>>();
                if parts.len() == 3 {
                    let rseq_ok = parts[0].parse::<u32>().is_ok();
                    let cseq_ok = parts[1].parse::<u32>().is_ok();
                    let method_ok = !parts[2].is_empty();
                    rseq_ok && cseq_ok && method_ok
                } else {
                    false
                }
            } else {
                false
            };

            if !rack_valid {
                warn!("received PRACK with missing or invalid RAck header");
                datagrams.push(PendingDatagram::new(
                    peer.to_string(),
                    response::build_response_with_owned_headers(
                        &mutable_request,
                        400,
                        "Bad Request - Invalid RAck",
                        &[],
                        "",
                    ),
                ));
                return datagrams;
            }

            debug!(
                call_id = mutable_request
                    .headers
                    .get("call-id")
                    .map(|v| v.as_str())
                    .unwrap_or("?"),
                "received PRACK from caller — responding 200 OK (already confirmed to gateway)"
            );
            datagrams.push(PendingDatagram::new(
                peer.to_string(),
                response::ok_for_request(&mutable_request),
            ));
            return datagrams;
        }
        Method::Refer => {
            // RFC 3515 Blind Transfer B2BUA handling
            let refer_to_str = mutable_request.headers.get("refer-to").map(|v| v.as_str());
            let target_uri = refer_to_str.and_then(extract_uri_from_contact);

            datagrams.push(PendingDatagram::new(
                peer.to_string(),
                response::accepted_202_for_request(&mutable_request),
            ));

            if let Some(target_uri) = target_uri {
                let local_cseq = transaction.last_inbound_cseq.unwrap_or(1) + 50;

                let notify_body = "SIP/2.0 100 Trying\r\n";
                let notify = outbound::build_notify_sipfrag(
                    call_id.as_str(),
                    mutable_request
                        .headers
                        .get("from")
                        .map(|v| v.as_str())
                        .unwrap_or(""),
                    mutable_request
                        .headers
                        .get("to")
                        .map(|v| v.as_str())
                        .unwrap_or(""),
                    local_cseq,
                    &edge_config.advertised_addr,
                    notify_body,
                );
                datagrams.push(PendingDatagram::new(peer.to_string(), notify));

                let outbound_uri = {
                    let registrar = edge_state.registrar.read().await;
                    if let Some(contact) = registrar
                        .lookup_contact(
                            &target_uri,
                            SystemTime::now(),
                            edge_state.db_store.as_ref(),
                        )
                        .await
                    {
                        SipUri::from_str(&contact.uri).ok()
                    } else {
                        edge_state
                            .call_manager
                            .routes()
                            .select(&target_uri)
                            .ok()
                            .map(|sr| sr.outbound_uri)
                    }
                };

                if outbound_uri.is_none() {
                    let notify_404 = outbound::build_notify_sipfrag_with_state(
                        call_id.as_str(),
                        mutable_request
                            .headers
                            .get("from")
                            .map(|v| v.as_str())
                            .unwrap_or(""),
                        mutable_request
                            .headers
                            .get("to")
                            .map(|v| v.as_str())
                            .unwrap_or(""),
                        local_cseq + 1,
                        &edge_config.advertised_addr,
                        "SIP/2.0 404 Not Found\r\n",
                        "terminated;reason=noresource",
                    );
                    datagrams.push(PendingDatagram::new(peer.to_string(), notify_404));
                    return datagrams;
                }
                let outbound_uri = outbound_uri.unwrap();

                let target_relay_rtp = match edge_state
                    .media_relay
                    .allocate_endpoint(&edge_config.media)
                {
                    Ok(ep) => ep,
                    Err(error) => {
                        warn!(%error, "failed to allocate media relay endpoint for transfer target");
                        let notify_503 = outbound::build_notify_sipfrag_with_state(
                            call_id.as_str(),
                            mutable_request
                                .headers
                                .get("from")
                                .map(|v| v.as_str())
                                .unwrap_or(""),
                            mutable_request
                                .headers
                                .get("to")
                                .map(|v| v.as_str())
                                .unwrap_or(""),
                            local_cseq + 1,
                            &edge_config.advertised_addr,
                            "SIP/2.0 503 Service Unavailable\r\n",
                            "terminated;reason=noresource",
                        );
                        datagrams.push(PendingDatagram::new(peer.to_string(), notify_503));
                        return datagrams;
                    }
                };

                let transferee_relay_rtp = match source_leg {
                    DialogLeg::Caller => transaction.gateway_relay_rtp.clone(),
                    DialogLeg::Gateway => transaction.caller_relay_rtp.clone(),
                };

                if let Some(transferee_relay) = &transferee_relay_rtp {
                    edge_state
                        .media_relay
                        .pair_ports(target_relay_rtp.port, transferee_relay.port);
                }

                let transfer_call_id = format!("transfer-{}-{}", call_id.as_str(), local_cseq);

                let refer_sub = crate::edge_state::ReferSubscription {
                    refer_to: target_uri.to_string(),
                    from_header: mutable_request
                        .headers
                        .get("from")
                        .map(|v| v.as_str().to_string())
                        .unwrap_or_default(),
                    to_header: mutable_request
                        .headers
                        .get("to")
                        .map(|v| v.as_str().to_string())
                        .unwrap_or_default(),
                    notify_cseq: local_cseq,
                    transfer_call_id: transfer_call_id.clone(),
                    referrer_peer: peer.to_string(),
                    refer_cseq: mutable_request
                        .headers
                        .get("cseq")
                        .and_then(|v| crate::sip::dialog::cseq_number(v.as_str()))
                        .unwrap_or(1),
                    target_relay_port: Some(target_relay_rtp.port),
                    transferee_relay_port: transferee_relay_rtp.as_ref().map(|ep| ep.port),
                };

                {
                    if let Some(mut t_mut) =
                        edge_state.inbound_transactions.get_mut(call_id.as_str())
                    {
                        t_mut.refer_subscription = Some(refer_sub);
                    }
                }

                edge_state
                    .refer_transfers
                    .insert(transfer_call_id.clone(), call_id.as_str().to_string());

                let from_header = match source_leg {
                    DialogLeg::Caller => mutable_request
                        .headers
                        .get("to")
                        .map(|v| v.as_str())
                        .unwrap_or(""),
                    DialogLeg::Gateway => mutable_request
                        .headers
                        .get("from")
                        .map(|v| v.as_str())
                        .unwrap_or(""),
                };
                let to_header = refer_to_str.unwrap_or("");

                let sdp_body = format!(
                    "v=0\r\no=- 0 0 IN IP4 {addr}\r\ns=-\r\nc=IN IP4 {addr}\r\nt=0 0\r\nm=audio {port} RTP/AVP 0 8 101\r\na=rtpmap:0 PCMU/8000\r\na=rtpmap:8 PCMA/8000\r\na=rtpmap:101 telephone-event/8000\r\na=fmtp:101 0-16\r\n",
                    addr = edge_config.advertised_addr,
                    port = target_relay_rtp.port,
                );

                let invite_bytes = outbound::build_transfer_invite(
                    &transfer_call_id,
                    from_header,
                    to_header,
                    1,
                    &edge_config.advertised_addr,
                    &outbound_uri,
                    sdp_body.as_bytes(),
                );

                let target_addr = outbound::target_addr_for(&outbound_uri);
                datagrams.push(PendingDatagram::new(target_addr, invite_bytes));
                return datagrams;
            } else {
                warn!(
                    call_id = call_id.as_str(),
                    "missing or invalid Refer-To header in REFER"
                );
                let notify_400 = outbound::build_notify_sipfrag_with_state(
                    call_id.as_str(),
                    mutable_request
                        .headers
                        .get("from")
                        .map(|v| v.as_str())
                        .unwrap_or(""),
                    mutable_request
                        .headers
                        .get("to")
                        .map(|v| v.as_str())
                        .unwrap_or(""),
                    transaction.last_inbound_cseq.unwrap_or(1) + 50,
                    &edge_config.advertised_addr,
                    "SIP/2.0 400 Bad Request\r\n",
                    "terminated;reason=noresource",
                );
                datagrams.push(PendingDatagram::new(peer.to_string(), notify_400));
                return datagrams;
            }
        }
        _ => {}
    }

    let (request_uri, route_set, target) =
        if transaction.transfer_call_id.is_some() {
            if is_target {
                if transaction.transferee_is_caller {
                    let uri = transaction
                        .caller_contact
                        .clone()
                        .unwrap_or_else(|| sip_uri_from_peer(&transaction.peer));
                    (uri, Vec::new(), transaction.peer.clone())
                } else {
                    let uri = transaction
                        .callee_contact
                        .clone()
                        .unwrap_or_else(|| transaction.outbound_uri.clone());
                    let target = if !transaction.outbound_route_set.is_empty() {
                        parse_target_addr_from_route(&transaction.outbound_route_set[0])
                            .unwrap_or_else(|| {
                                if transaction.callee_behind_nat {
                                    transaction.outbound_peer.clone().unwrap_or_else(|| {
                                        outbound::target_addr_for(&transaction.outbound_uri)
                                    })
                                } else {
                                    outbound::target_addr_for(&transaction.outbound_uri)
                                }
                            })
                    } else if transaction.callee_behind_nat {
                        transaction
                            .outbound_peer
                            .clone()
                            .unwrap_or_else(|| outbound::target_addr_for(&uri))
                    } else {
                        outbound::target_addr_for(&uri)
                    };
                    (uri, transaction.outbound_route_set.clone(), target)
                }
            } else {
                let uri = transaction
                    .transfer_contact
                    .clone()
                    .unwrap_or_else(|| transaction.outbound_uri.clone());
                let target = transaction
                    .transfer_peer
                    .clone()
                    .unwrap_or_else(|| outbound::target_addr_for(&uri));
                (uri, Vec::new(), target)
            }
        } else {
            match source_leg {
                DialogLeg::Caller => {
                    let request_uri = transaction
                        .callee_contact
                        .clone()
                        .unwrap_or_else(|| transaction.outbound_uri.clone());
                    let target = if !transaction.outbound_route_set.is_empty() {
                        parse_target_addr_from_route(&transaction.outbound_route_set[0])
                            .unwrap_or_else(|| {
                                if transaction.callee_behind_nat {
                                    transaction.outbound_peer.clone().unwrap_or_else(|| {
                                        outbound::target_addr_for(&transaction.outbound_uri)
                                    })
                                } else {
                                    outbound::target_addr_for(&transaction.outbound_uri)
                                }
                            })
                    } else if transaction.callee_behind_nat {
                        transaction
                            .outbound_peer
                            .clone()
                            .unwrap_or_else(|| outbound::target_addr_for(&request_uri))
                    } else {
                        outbound::target_addr_for(&request_uri)
                    };
                    (request_uri, transaction.outbound_route_set.clone(), target)
                }
                DialogLeg::Gateway => {
                    let request_uri = transaction
                        .caller_contact
                        .clone()
                        .unwrap_or_else(|| sip_uri_from_peer(&transaction.peer));
                    let target = if !transaction.inbound_route_set.is_empty() {
                        parse_target_addr_from_route(&transaction.inbound_route_set[0])
                            .unwrap_or_else(|| transaction.peer.clone())
                    } else {
                        transaction.peer.clone()
                    };
                    (request_uri, transaction.inbound_route_set.clone(), target)
                }
            }
        };

    let mut rewritten_sdp = None;
    let is_bridged = transaction.transfer_call_id.is_some();
    if !is_bridged && matches!(&mutable_request.method, Method::Invite | Method::Update) {
        {
            if let Some(mut t_mut) = edge_state.inbound_transactions.get_mut(call_id.as_str()) {
                t_mut.last_session_refresh = Some(Instant::now());
                debug!(
                    call_id = call_id.as_str(),
                    "session timer refreshed by Re-INVITE/UPDATE"
                );
            }
        }

        if media::is_sdp_body(&mutable_request.headers, &mutable_request.body) {
            let is_from_caller = peer.to_string() == transaction.peer;
            if is_from_caller {
                if let Some(gw_relay) = &transaction.gateway_relay_rtp {
                    // Single-pass: rewrite SDP + extract original endpoint
                    if let Ok((rewritten, remote_ep)) =
                        media::rewrite_sdp_and_extract_endpoint(&mutable_request.body, gw_relay)
                    {
                        rewritten_sdp = Some(rewritten);
                        register_relay_target(
                            &edge_state.media_relay,
                            gw_relay,
                            &remote_ep,
                            "mid-dialog caller target update",
                        );

                        if let Some(mut t_mut) =
                            edge_state.inbound_transactions.get_mut(call_id.as_str())
                        {
                            t_mut.caller_rtp = Some(remote_ep);
                            t_mut.original_request = Some(Arc::new(mutable_request.clone()));
                        }
                    }
                }
            } else if let Some(caller_relay) = &transaction.caller_relay_rtp {
                // Single-pass: rewrite SDP + extract original endpoint
                if let Ok((rewritten, remote_ep)) =
                    media::rewrite_sdp_and_extract_endpoint(&mutable_request.body, caller_relay)
                {
                    rewritten_sdp = Some(rewritten);
                    register_relay_target(
                        &edge_state.media_relay,
                        caller_relay,
                        &remote_ep,
                        "mid-dialog gateway target update",
                    );

                    if let Some(mut t_mut) =
                        edge_state.inbound_transactions.get_mut(call_id.as_str())
                    {
                        t_mut.gateway_rtp = Some(remote_ep);
                    }
                }
            }
        }
    }

    // Topology Hiding: when forwarding a request from the caller toward the gateway,
    // replace the inbound (internal) Call-ID with the external Call-ID the gateway knows.
    // When forwarding from gateway toward caller, no rewrite is needed — the caller sees
    // the internal Call-ID.
    if matches!(source_leg, DialogLeg::Caller) {
        let internal_cid = mutable_request
            .headers
            .get("call-id")
            .map(|v| v.as_str().to_string())
            .unwrap_or_default();
        if let Some(external_cid) = edge_state.get_external_call_id(&internal_cid) {
            mutable_request.headers.replace(
                HeaderName::new("call-id").unwrap(),
                HeaderValue::new(&external_cid),
            );
            debug!(
                internal_cid,
                external_cid, "topology hiding: rewrote Call-ID for in-dialog request to gateway"
            );
        }
    }

    let bytes = if let Some(body) = &rewritten_sdp {
        outbound::build_outbound_in_dialog_request_with_body(
            &mutable_request,
            &request_uri,
            &edge_config.advertised_addr,
            &route_set,
            body,
        )
    } else {
        outbound::build_outbound_in_dialog_request(
            &mutable_request,
            &request_uri,
            &edge_config.advertised_addr,
            &route_set,
        )
    };
    // BYE/CANCEL 转发后立即清理事务：更新并发计数并从 map 中删除
    if matches!(&mutable_request.method, Method::Bye | Method::Cancel) {
        let username = transaction
            .original_request
            .as_ref()
            .and_then(|req| crate::edge_state::EdgeState::username_from_request(req));
        if let Some(ref uname) = username {
            edge_state.decrement_user_concurrency(uname);
        }
        edge_state.inbound_transactions.remove(call_id.as_str());
    }

    datagrams.push(PendingDatagram::new(target, bytes));
    datagrams
}

fn call_error_for_unknown_request(request: &SipRequest) -> CallError {
    match request.headers.get("call-id") {
        Some(call_id) => CallError::UnknownCall(call_id.as_str().to_string()),
        None => CallError::MissingRequiredHeader("Call-ID"),
    }
}

fn response_for_dialog_validation_error(
    request: &SipRequest,
    error: &DialogValidationError,
) -> Vec<u8> {
    let (status_code, reason_phrase) = error.status();
    response::build_response_with_owned_headers(
        request,
        status_code,
        reason_phrase,
        &[("X-VOS-RS-Error".to_string(), error.to_string())],
        "",
    )
}

fn response_for_media_error(request: &SipRequest, error: &media::MediaError) -> Vec<u8> {
    match error {
        media::MediaError::PortRangeExhausted { .. } => {
            response::service_unavailable_for_request(request, &error.to_string())
        }
        _ => response::not_acceptable_for_request(request, &error.to_string()),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RewrittenSdp {
    pub(crate) original_endpoint: Option<RtpEndpoint>,
    pub(crate) relay_endpoint: RtpEndpoint,
    pub(crate) body: Vec<u8>,
}

pub(crate) fn prepare_rewritten_sdp(
    headers: &HeaderMap,
    body: &[u8],
    media_relay: &MediaRelayState,
    media_config: &MediaConfig,
    direction: &'static str,
) -> Result<Option<RewrittenSdp>, media::MediaError> {
    if !media::is_sdp_body(headers, body) {
        return Ok(None);
    }

    media::validate_media_negotiation(body)?;

    let relay_endpoint = media_relay.allocate_endpoint(media_config)?;
    if let Ok(sdp_text) = std::str::from_utf8(body) {
        if let Ok(session) = SessionDescription::parse(sdp_text) {
            if let Ok(crypto_attributes) = session.first_audio_srtp_crypto() {
                if let Some(crypto) = crypto_attributes.first() {
                    media_relay.register_srtp_offer(
                        relay_endpoint.port,
                        &crypto.suite,
                        &crypto.key_params,
                    );
                }
            }
        }
    }
    match media::rewrite_sdp_and_extract_endpoint(body, &relay_endpoint) {
        Ok((body, original_endpoint)) => Ok(Some(RewrittenSdp {
            original_endpoint: Some(original_endpoint),
            relay_endpoint,
            body,
        })),
        Err(error) => {
            media_relay.clear_target(relay_endpoint.port);
            warn!(%error, direction, "failed to rewrite SDP body for media relay");
            Err(error)
        }
    }
}

pub(crate) fn register_relay_target(
    media_relay: &MediaRelayState,
    relay_endpoint: &RtpEndpoint,
    target_endpoint: &RtpEndpoint,
    direction: &'static str,
) {
    if let Err(error) = media_relay.set_target(relay_endpoint, target_endpoint) {
        warn!(%error, direction, "failed to register RTP relay target");
    }
}

/// Replace the value of a SIP header in a raw message string.
/// Only replaces the first occurrence (case-insensitive header name match).
pub(crate) fn replace_header_value(raw: &str, header_name: &str, new_value: &str) -> String {
    let needle_lower = header_name.to_ascii_lowercase();
    let mut result = String::with_capacity(raw.len() + 8);
    for line in raw.split_inclusive("\r\n") {
        let header_part = line.split(':').next().unwrap_or("");
        if header_part.trim().to_ascii_lowercase() == needle_lower {
            result.push_str(&format!("{header_name}: {new_value}\r\n"));
        } else {
            result.push_str(line);
        }
    }
    result
}

fn parse_sip_info_dtmf(content_type: &str, body: &[u8]) -> Option<char> {
    let body_str = std::str::from_utf8(body).ok()?.trim();
    if content_type.contains("application/dtmf-relay") {
        for line in body_str.lines() {
            let line = line.trim();
            if line.to_ascii_lowercase().starts_with("signal=") {
                let parts: Vec<&str> = line.split('=').collect();
                if parts.len() == 2 {
                    let signal = parts[1].trim();
                    if signal.len() == 1 {
                        let c = signal.chars().next()?;
                        if c.is_ascii_digit() || c == '*' || c == '#' || ('A'..='D').contains(&c) {
                            return Some(c);
                        }
                    }
                }
            }
        }
    } else if content_type.contains("application/dtmf") && body_str.len() == 1 {
        let c = body_str.chars().next()?;
        if c.is_ascii_digit() || c == '*' || c == '#' || ('A'..='D').contains(&c) {
            return Some(c);
        }
    }
    None
}
