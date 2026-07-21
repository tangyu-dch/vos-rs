use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::atomic::Ordering;

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

    let callee_num = request.uri.user.as_deref().unwrap_or("");
    let is_conf = callee.starts_with("conf_")
        || callee.starts_with("room_")
        || callee_num.starts_with("conf_")
        || callee_num.starts_with("room_");

    if is_conf {
        let conf_id = if callee.starts_with("conf_") || callee.starts_with("room_") {
            &callee
        } else {
            callee_num
        };

        info!(conf_id, "incoming SIP INVITE to join conference");

        // 1. 自动为会议分配媒体中继端口
        let call_id = request
            .headers
            .get("call-id")
            .map(|value| value.as_str())
            .unwrap_or("");
        let local_ep = match edge_state
            .media_relay
            .allocate_endpoint_for_call(&edge_config.media, call_id)
        {
            Ok(ep) => ep,
            Err(e) => {
                warn!(error = %e, "failed to allocate endpoint for conference");
                return vec![PendingDatagram::new(
                    peer.to_string(),
                    response_for_media_error(&request, &e),
                )];
            }
        };

        // 2. 解析客户端 SDP 媒体端点与协商的编解码器
        let client_ep = match crate::media::sdp::parse_sdp_rtp_endpoint(&request.body) {
            Ok(ep) => ep,
            Err(e) => {
                warn!(error = %e, "failed to parse client SDP for conference");
                edge_state.media_relay.clear_target(local_ep.port);
                return vec![PendingDatagram::new(
                    peer.to_string(),
                    response_for_media_error(&request, &e),
                )];
            }
        };

        let client_addr = match crate::media::sdp::socket_addr_for_endpoint(&client_ep) {
            Ok(addr) => addr,
            Err(e) => {
                warn!(error = %e, "failed to resolve client SDP socket addr for conference");
                edge_state.media_relay.clear_target(local_ep.port);
                return vec![PendingDatagram::new(
                    peer.to_string(),
                    response_for_media_error(&request, &e),
                )];
            }
        };

        let codec = crate::media::sdp::negotiated_audio_codec(&request.body)
            .unwrap_or(rtp_core::AudioCodec::Pcma);

        // 注册局部编解码器关联
        edge_state
            .media_relay
            .register_port_codec(local_ep.port, codec);

        // 从 active_sockets 中获取已分配的 UDP Socket
        let socket = match edge_state.media_relay.active_sockets.get(&local_ep.port) {
            Some(s) => s.value().clone(),
            None => {
                warn!(
                    port = local_ep.port,
                    "UDP socket not found in active_sockets"
                );
                edge_state.media_relay.clear_target(local_ep.port);
                return vec![PendingDatagram::new(
                    peer.to_string(),
                    response::build_response_with_owned_headers(
                        &request,
                        500,
                        "Internal Server Error",
                        &[],
                        "",
                    ),
                )];
            }
        };

        // 3. 将参会成员加入混音管理器
        edge_state
            .media_relay
            .conference_manager
            .join_conference(conf_id, local_ep.port, codec, client_addr, socket)
            .await;
        edge_state
            .media_relay
            .mark_relay_features_changed(local_ep.port);

        let internal_call_id = request
            .headers
            .get("call-id")
            .map(|v| v.as_str().to_string())
            .unwrap_or_default();

        // 4. 在 CallManager 中建立此呼叫会话以支持生命周期和 CDR 跟踪
        let _ = edge_state.call_manager.handle_inbound_invite_to_uri(
            &request,
            SipUri::from_str(&format!("sip:{}@localhost", conf_id)).unwrap(),
        );

        // 将呼叫置为已应答/已接通
        let dummy_resp = sip_core::SipResponse {
            version: std::borrow::Cow::Borrowed("SIP/2.0"),
            status_code: 200,
            reason_phrase: std::borrow::Cow::Borrowed("OK"),
            headers: request.headers.clone(),
            body: std::borrow::Cow::Borrowed(&[]),
        };
        let _ = edge_state
            .call_manager
            .handle_outbound_response(&dummy_resp);

        // 5. 记录事务状态
        edge_state.remember_inbound_invite(
            &request,
            peer,
            SipUri::from_str(&format!("sip:{}@localhost", conf_id)).unwrap(),
            Some(client_ep.clone()),
            Some(local_ep.clone()),
            false,
            None,
        );

        let to_tag = "vosrs-edge".to_string();

        // 修正本地中继关联为 caller 侧中继
        if let Some(mut tx) = edge_state.inbound_transactions.get_mut(&internal_call_id) {
            tx.caller_relay_rtp = Some(local_ep.clone());
            tx.inbound_to_tag = Some(to_tag.clone());
        }

        // 6. 返回 SDP 应答并发送 200 OK 接通
        let pt = codec.static_payload_type().unwrap_or(8);
        let codec_name = match codec {
            rtp_core::AudioCodec::Pcmu => "PCMU",
            _ => "PCMA",
        };

        let sdp_answer = format!(
            "v=0\r\n\
             o=vos-rs 123456 123456 IN IP4 {addr}\r\n\
             s=vos-rs-conference\r\n\
             c=IN IP4 {addr}\r\n\
             t=0 0\r\n\
             m=audio {port} RTP/AVP {pt}\r\n\
             a=rtpmap:{pt} {codec_name}/8000\r\n\
             a=sendrecv\r\n",
            addr = edge_config.media.advertised_addr,
            port = local_ep.port,
        );

        let response = response::build_response_with_owned_headers(
            &request,
            200,
            "OK",
            &[
                ("Content-Type".to_string(), "application/sdp".to_string()),
                (
                    "Contact".to_string(),
                    format!("<sip:{}@{}>", conf_id, edge_config.advertised_addr),
                ),
            ],
            &sdp_answer,
        );

        return vec![PendingDatagram::new(peer.to_string(), response)];
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

    let transport = if request
        .headers
        .get("via")
        .is_some_and(|v| v.as_str().to_ascii_uppercase().contains("TCP"))
    {
        "tcp"
    } else {
        "udp"
    };

    let egress_trunk_id = edge_state.identify_egress_trunk(peer).await;
    let from_gw = egress_trunk_id.is_some();
    let call_direction = if from_gw {
        call_core::CallDirection::Inbound
    } else {
        call_core::CallDirection::Outbound
    };
    let request_number = request.uri.user.as_deref().unwrap_or("");
    let inbound_did_destination = egress_trunk_id
        .as_ref()
        .and_then(|_| edge_state.did_destination(request_number));

    let mut call_source: Option<call_core::CallSource> = None;

    if let Some(ref trunk_id) = egress_trunk_id {
        if trunk_id != "test-gateway"
            && trunk_id != "default"
            && !edge_state
                .call_manager
                .owns_number(request_number, trunk_id)
        {
            warn!(trunk_id = %trunk_id, number = %request_number, "egress trunk does not own callee number");
            return vec![PendingDatagram::new(
                peer.to_string(),
                response::build_response_with_owned_headers(
                    &request,
                    403,
                    "Forbidden - Number Ownership Validation Failed",
                    &[],
                    "",
                ),
            )];
        }
        if let Some(destination) = inbound_did_destination.as_ref() {
            if destination.target_type == "reject" {
                return vec![PendingDatagram::new(
                    peer.to_string(),
                    response::build_response_with_owned_headers(
                        &request,
                        403,
                        "Forbidden - DID Rejected",
                        &[],
                        "",
                    ),
                )];
            }
            if destination.target_type != "extension"
                && destination.target_type != "extension_group"
                && destination.target_type != "ivr"
            {
                return vec![PendingDatagram::new(
                    peer.to_string(),
                    response::build_response_with_owned_headers(
                        &request,
                        501,
                        "Not Implemented - DID Target",
                        &[],
                        "",
                    ),
                )];
            }
        }
        call_source = Some(call_core::CallSource::new("trunk", trunk_id));
    }

    if call_source.is_none() {
        match edge_state.identify_access_trunk(peer, transport) {
            Ok(Some(trunk_id)) => {
                let mode = edge_state.access_trunk_auth_mode(&trunk_id);
                let is_auth_bypass = !edge_config.auth.is_enabled()
                    || std::env::var("VOS_RS_AUTH_BYPASS").ok().as_deref() == Some("true");
                if mode == "ip_allowlist" || is_auth_bypass {
                    call_source = Some(call_core::CallSource::new("trunk", trunk_id));
                } else if mode == "ip_and_digest" {
                    let auth_res = edge_state
                        .verify_sip_auth(&edge_config.auth, &request, true)
                        .await;
                    match auth_res {
                        AuthDecision::Challenge => {
                            return vec![PendingDatagram::new(
                                peer.to_string(),
                                proxy_unauthorized_for_request(&request, &edge_config.auth),
                            )];
                        }
                        AuthDecision::ChallengeWithFailure => {
                            edge_state.sbc_engine.register_auth_failure(peer.ip());
                            return vec![PendingDatagram::new(
                                peer.to_string(),
                                proxy_unauthorized_for_request(&request, &edge_config.auth),
                            )];
                        }
                        _ => {
                            call_source = Some(call_core::CallSource::new("trunk", trunk_id));
                        }
                    }
                } else {
                    call_source = Some(call_core::CallSource::new("trunk", trunk_id));
                }
            }
            Err(_) => {
                warn!(?peer, "overlapping access trunk IP rules matched");
                return vec![PendingDatagram::new(
                    peer.to_string(),
                    response::build_response_with_owned_headers(
                        &request,
                        403,
                        "Forbidden - Overlapping IP Rules",
                        &[],
                        "",
                    ),
                )];
            }
            _ => {}
        }
    }

    if call_source.is_none() && edge_config.auth.is_enabled() {
        let username_opt = edge_config.auth.authorization_username(&request);
        let username = username_opt
            .clone()
            .or_else(|| EdgeState::username_from_request(&request))
            .unwrap_or_default();
        let is_trunk = edge_state.is_registered_access_username(&username);
        let auth_res = edge_state
            .verify_sip_auth(&edge_config.auth, &request, is_trunk)
            .await;
        match auth_res {
            AuthDecision::Challenge => {
                return vec![PendingDatagram::new(
                    peer.to_string(),
                    proxy_unauthorized_for_request(&request, &edge_config.auth),
                )];
            }
            AuthDecision::ChallengeWithFailure => {
                edge_state.sbc_engine.register_auth_failure(peer.ip());
                return vec![PendingDatagram::new(
                    peer.to_string(),
                    proxy_unauthorized_for_request(&request, &edge_config.auth),
                )];
            }
            _ => {
                if is_trunk {
                    if let Some(trunk_id) = edge_state.resolve_access_username_to_trunk(&username) {
                        call_source = Some(call_core::CallSource::new("trunk", trunk_id));
                    } else {
                        call_source = Some(call_core::CallSource::new("trunk", username));
                    }
                } else {
                    call_source = Some(call_core::CallSource::new("extension", username));
                }
            }
        }
    } else if call_source.is_none() {
        let username_opt = edge_config.auth.authorization_username(&request);
        let username = username_opt
            .clone()
            .or_else(|| EdgeState::username_from_request(&request))
            .unwrap_or_else(|| "1001".to_string());
        call_source = Some(call_core::CallSource::new("trunk", username));
    }

    let source = call_source.expect("source must be resolved here");
    let billing_account = if from_gw {
        None
    } else if source.source_type == "extension" {
        Some(source.source_id.clone())
    } else {
        edge_state.resolve_trunk_billing_account(&source.source_id)
    };

    let caller_domain = EdgeState::domain_from_request(&request);
    let callee_domain = request.uri.host.clone();

    if let Some(ref caller_dom) = caller_domain {
        if callee_domain != *caller_dom {
            let registered_contact = edge_state.lookup_destination_contact(&request.uri).await;

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
    if edge_config.webhooks.control_mode == "http" || edge_config.webhooks.control_mode == "nats" {
        let edge_state_arc = match edge_state.self_weak.get().and_then(|w| w.upgrade()) {
            Some(arc) => arc,
            None => {
                warn!("self_weak not initialized inside handle_invite_request for VCI control");
                return Vec::new();
            }
        };
        return crate::sip::handlers::interactive_control::handle_interactive_webhook_call(
            request,
            peer,
            &edge_state_arc,
            edge_config,
        )
        .await;
    }

    let registered_contact = edge_state.lookup_destination_contact(&request.uri).await;

    if from_gw
        && registered_contact.is_none()
        && inbound_did_destination
            .as_ref()
            .is_some_and(|destination| destination.target_type == "extension")
    {
        warn!(
            trunk_id = egress_trunk_id.as_deref().unwrap_or(""),
            did = request_number,
            "DID target extension is not registered"
        );
        return vec![PendingDatagram::new(
            peer.to_string(),
            response::build_response_with_owned_headers(
                &request,
                480,
                "Temporarily Unavailable - DID Extension Offline",
                &[],
                "",
            ),
        )];
    }

    let response::RequestHandling {
        response,
        mut outbound_invite,
    } = if let Some(ref did_dest) = inbound_did_destination {
        if did_dest.target_type == "ivr" {
            return super::ivr::handle_ivr_locally(
                request,
                peer,
                edge_state,
                edge_config,
                did_dest,
            )
            .await;
        } else if did_dest.target_type == "extension_group" {
            let members = edge_state
                .extension_groups
                .read()
                .ok()
                .and_then(|lock| lock.get(&did_dest.target_id).cloned())
                .unwrap_or_default();

            let mut group_contacts = Vec::new();
            for member in members {
                let mut member_uri = request.uri.clone();
                member_uri.user = Some(member.into());
                if let Some(contact) = edge_state.lookup_contact(&member_uri).await {
                    group_contacts.push(contact);
                }
            }

            if group_contacts.is_empty() {
                warn!(group_id = %did_dest.target_id, "分机组内没有在线成员");
                return vec![PendingDatagram::new(
                    peer.to_string(),
                    response::build_response_with_owned_headers(
                        &request,
                        480,
                        "Temporarily Unavailable - Extension Group Offline",
                        &[],
                        "",
                    ),
                )];
            }

            let first_contact = &group_contacts[0];
            if let Ok(outbound_uri) = SipUri::from_str(&first_contact.uri) {
                let outcome = response::response_for_invite_to_uri_with_direction(
                    &request,
                    &edge_state.call_manager,
                    outbound_uri,
                    call_direction,
                );

                let internal_call_id = request
                    .headers
                    .get("call-id")
                    .map(|v| v.as_str().to_string())
                    .unwrap_or_default();
                let mut candidates = Vec::new();
                for contact in group_contacts {
                    if let Ok(mut outbound_uri) = SipUri::from_str(&contact.uri) {
                        if let Ok(received_addr) =
                            contact.received_from.parse::<std::net::SocketAddr>()
                        {
                            outbound_uri.host = received_addr.ip().to_string().into();
                            outbound_uri.port = Some(received_addr.port());
                        }
                        candidates.push(call_core::SelectedRoute {
                            route_id: format!("group-{}", did_dest.target_id),
                            target: call_core::RouteTarget::new(
                                "extension-group-gateway",
                                outbound_uri.host.to_string(),
                                outbound_uri.port,
                            ),
                            outbound_uri,
                        });
                    }
                }
                edge_state
                    .call_manager
                    .set_candidates(&call_core::CallId::new(internal_call_id), candidates);
                outcome
            } else {
                return vec![PendingDatagram::new(
                    peer.to_string(),
                    response::build_response_with_owned_headers(
                        &request,
                        500,
                        "Internal Server Error - Invalid Group Contact",
                        &[],
                        "",
                    ),
                )];
            }
        } else {
            // 标准分机
            if let Some(ref contact) = registered_contact {
                if let Ok(outbound_uri) = SipUri::from_str(&contact.uri) {
                    response::response_for_invite_to_uri_with_direction(
                        &request,
                        &edge_state.call_manager,
                        outbound_uri,
                        call_direction,
                    )
                } else {
                    response::response_for_request_with_health_and_direction(
                        &request,
                        &edge_state.call_manager,
                        Some(&source),
                        Some(&edge_state.gateway_health),
                        call_direction,
                    )
                }
            } else {
                response::response_for_request_with_health_and_direction(
                    &request,
                    &edge_state.call_manager,
                    Some(&source),
                    Some(&edge_state.gateway_health),
                    call_direction,
                )
            }
        }
    } else if let Some(ref contact) = registered_contact {
        if let Ok(outbound_uri) = SipUri::from_str(&contact.uri) {
            response::response_for_invite_to_uri_with_direction(
                &request,
                &edge_state.call_manager,
                outbound_uri,
                call_direction,
            )
        } else {
            response::response_for_request_with_health_and_direction(
                &request,
                &edge_state.call_manager,
                Some(&source),
                Some(&edge_state.gateway_health),
                call_direction,
            )
        }
    } else {
        response::response_for_request_with_health_and_direction(
            &request,
            &edge_state.call_manager,
            Some(&source),
            Some(&edge_state.gateway_health),
            call_direction,
        )
    };

    if outbound_invite.is_some() {
        if let Some(call_id) = request.headers.get("call-id") {
            edge_state.call_manager.set_billing_account(
                &call_core::CallId::new(call_id.as_str()),
                billing_account.clone(),
            );
        }
    }

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
    let mut billing_pulse: Option<(u32, f64)> = None;

    // 呼叫热路径只从 Redis 读取余额和费率，不回退查询 PostgreSQL。
    if edge_config.balance_enforcement_enabled && !from_gw && outbound_invite.is_some() {
        let callee = request.uri.user.as_deref().unwrap_or("");
        if let Some(caller_user) = billing_account.as_deref() {
            match edge_state.redis_balance_check(caller_user, callee).await {
                Some(check) if !check.has_balance => {
                    warn!(caller = %caller_user, balance = check.balance, interval = check.billing_interval_secs, price = check.price_per_interval, "pre-call Redis balance check failed");
                    return vec![PendingDatagram::new(
                        peer.to_string(),
                        response::error_for_call_error(
                            &request,
                            &call_core::CallError::GatewayUnavailable("余额不足".to_string()),
                        ),
                    )];
                }
                Some(check) if check.price_per_interval > 0.0 => {
                    billing_pulse = Some((check.billing_interval_secs, check.price_per_interval));
                    calculated_max_duration = crate::billing_settlement::maximum_duration_secs(
                        check.balance,
                        check.billing_interval_secs,
                        check.price_per_interval,
                    );
                }
                Some(_) => {}
                None => {
                    warn!(caller = %caller_user, "Redis balance check unavailable, allowing call");
                }
            }
        }
    }

    let lease_call_id = request
        .headers
        .get("call-id")
        .map(|value| call_core::CallId::new(value.as_str()))
        .filter(|_| outbound_invite.is_some());
    if let Some(call_id) = lease_call_id.as_ref() {
        edge_state.call_manager.set_cdr_audit_context(
            call_id,
            egress_trunk_id.clone(),
            billing_pulse.map(|pulse| pulse.0),
            billing_pulse.map(|pulse| pulse.1),
        );
        let lease_error = loop {
            match crate::resource_lease::acquire(edge_state, call_id, calculated_max_duration).await
            {
                Ok(()) => break None,
                Err(
                    error @ (crate::resource_lease::LeaseError::NumberBusy
                    | crate::resource_lease::LeaseError::TrunkAtCapacity),
                ) => {
                    let Some(next) = edge_state.call_manager.advance_caller_pool(call_id) else {
                        break Some(error);
                    };
                    if let Some(plan) = outbound_invite.as_mut() {
                        plan.outbound_uri = next.outbound_uri;
                        plan.gateway_id = next
                            .caller_identity
                            .as_ref()
                            .map(|identity| identity.owner_gateway_id.as_str().to_string())
                            .unwrap_or_default();
                        plan.caller_identity = next.caller_identity;
                        plan.target_override_addr = None;
                    }
                    warn!(
                        call_id = %call_id.as_str(),
                        gateway_id = %outbound_invite.as_ref().map(|plan| plan.gateway_id.as_str()).unwrap_or(""),
                        reason = %error,
                        "caller pool member resources at capacity, trying next member"
                    );
                }
                Err(error) => break Some(error),
            }
        };
        if let Some(error) = lease_error {
            warn!(call_id = %call_id.as_str(), %error, "outbound call resource lease rejected");
            edge_state
                .call_manager
                .terminate_call_with_reason(call_id.as_str(), &error.to_string());
            return vec![PendingDatagram::new(
                peer.to_string(),
                response::error_for_call_error(
                    &request,
                    &call_core::CallError::GatewayUnavailable(error.to_string()),
                ),
            )];
        }
    }

    if let Some(outbound_invite) = outbound_invite.as_ref() {
        let rewritten_sdp = match prepare_rewritten_sdp(
            &request.headers,
            &request.body,
            &edge_state.media_relay,
            &edge_config.media,
            "inbound INVITE offer",
            request
                .headers
                .get("call-id")
                .map(|value| value.as_str())
                .unwrap_or(""),
        ) {
            Ok(rewritten_sdp) => rewritten_sdp,
            Err(error) => {
                warn!(%error, "rejecting INVITE after media negotiation failure");
                if let Some(call_id) = lease_call_id.as_ref() {
                    edge_state
                        .call_manager
                        .terminate_call_with_reason(call_id.as_str(), &error.to_string());
                    crate::resource_lease::release(edge_state, call_id);
                }
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

        let forking_enabled = request
            .headers
            .get("x-forking-enabled")
            .map(|v| v.as_str().trim().to_lowercase() == "true")
            .unwrap_or(false)
            || request
                .headers
                .get("x-call-forking")
                .map(|v| v.as_str().trim().to_lowercase() == "true")
                .unwrap_or(false)
            || inbound_did_destination
                .as_ref()
                .is_some_and(|d| d.target_type == "extension_group");

        let managed_resources = crate::resource_lease::requires_single_leg(
            edge_state,
            &call_core::CallId::new(internal_call_id.clone()),
        );
        if forking_enabled && candidates.len() > 1 && !managed_resources {
            let fork_candidates = candidates.iter().take(3).cloned().collect::<Vec<_>>();
            let mut forks_to_save = Vec::new();
            for candidate in &fork_candidates {
                let external_call_id = uuid::Uuid::new_v4().to_string();
                edge_state.register_call_id_mapping(&internal_call_id, &external_call_id);
                forks_to_save.push((
                    external_call_id.clone(),
                    candidate.target.gateway_id.as_str().to_string(),
                ));

                let target = outbound::target_addr_for(&candidate.outbound_uri);
                let bytes = outbound::build_outbound_invite_with_session_timer_call_id_and_caller(
                    &request,
                    &candidate.outbound_uri,
                    &edge_config.advertised_addr,
                    rewritten_sdp
                        .as_ref()
                        .map(|sdp| sdp.body.as_slice())
                        .unwrap_or(request.body.as_ref()),
                    edge_config.session_expires_gateway,
                    path,
                    &external_call_id,
                    outbound_invite.caller_identity.as_ref(),
                );
                datagrams.push(PendingDatagram::new(target, bytes));

                let gw_id = candidate.target.gateway_id.as_str().to_string();
                if !gw_id.is_empty() {
                    edge_state.gateway_health.increment_active(&gw_id);
                    let status = edge_state.gateway_health.get_gateway_status(&gw_id);
                    crate::timers::persist_gateway_health(edge_state, gw_id.clone(), status);
                }
            }

            if let Some(mut t_mut) = edge_state.inbound_transactions.get_mut(&internal_call_id) {
                t_mut.active_forks = forks_to_save;
            }
        } else {
            let external_call_id = uuid::Uuid::new_v4().to_string();
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
            info!(
                internal_call_id,
                external_call_id,
                gateway_id = %outbound_invite.gateway_id,
                outbound_uri = %outbound_invite.outbound_uri,
                target = %target,
                "bench: sending outbound INVITE to gateway"
            );

            let bytes = outbound::build_outbound_invite_with_session_timer_call_id_and_caller(
                &request,
                &outbound_invite.outbound_uri,
                &edge_config.advertised_addr,
                rewritten_sdp
                    .as_ref()
                    .map(|sdp| sdp.body.as_slice())
                    .unwrap_or(request.body.as_ref()),
                edge_config.session_expires_gateway,
                path,
                &external_call_id,
                outbound_invite.caller_identity.as_ref(),
            );
            datagrams.push(PendingDatagram::new(target, bytes));

            if !outbound_invite.gateway_id.is_empty() {
                edge_state
                    .gateway_health
                    .increment_active(&outbound_invite.gateway_id);
                let status = edge_state
                    .gateway_health
                    .get_gateway_status(&outbound_invite.gateway_id);
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
