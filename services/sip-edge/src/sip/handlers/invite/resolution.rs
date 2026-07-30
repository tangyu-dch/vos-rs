use std::net::SocketAddr;

use sip_core::SipRequest;
use tracing::warn;

use crate::config::EdgeConfig;
use crate::edge_state::{EdgeState, PendingDatagram};
use crate::sip::{response, AuthDecision};
use crate::tenant::TenantContext;

use super::super::proxy_unauthorized_for_request;

/// 呼叫源解析结果，承载在路由与出站分发阶段需要的状态。
pub(super) struct CallResolution {
    pub source: call_core::CallSource,
    pub billing_account: Option<String>,
    pub egress_trunk_id: Option<String>,
    pub from_gw: bool,
    pub call_direction: call_core::CallDirection,
    pub request_number: String,
    pub inbound_did_destination: Option<cdr_core::DidDestination>,
    /// 已解析的租户上下文（若 tenant_enabled=false 则为 anonymous）。
    pub tenant_ctx: TenantContext,
}

/// 检查单用户并发上限，返回 `Some(datagrams)` 表示呼叫被拒绝。
pub(super) fn check_user_concurrency(
    request: &SipRequest,
    peer: SocketAddr,
    edge_state: &EdgeState,
    edge_config: &EdgeConfig,
) -> Option<Vec<PendingDatagram>> {
    let username = request.headers.get("from").and_then(|v| {
        let s = v.as_str();
        let start = s.find("sip:")?;
        let end = s[start..].find('@')?;
        Some(s[start + 4..start + end].to_string())
    })?;

    // O(1) 并发数查询，替代原来 O(n) inbound_transactions.iter() 扫描
    let active_count = edge_state.user_concurrent_count(&username);

    if active_count >= edge_config.sbc_max_concurrency {
        warn!(%username, active_count, limit = edge_config.sbc_max_concurrency, "rejecting INVITE due to user concurrency limit exceeded");
        Some(vec![PendingDatagram::new(
            peer.to_string(),
            response::build_response_with_owned_headers(
                request,
                486,
                "Busy Here - Concurrency Limit Exceeded",
                &[],
                "",
            ),
        )])
    } else {
        None
    }
}

/// 解析多租户上下文（若 TenantRegistry 未注入则返回 anonymous）。
pub(super) async fn resolve_tenant_context_pub(
    request: &SipRequest,
    edge_state: &EdgeState,
) -> TenantContext {
    let Some(registry) = edge_state.tenant_registry() else {
        return TenantContext::anonymous();
    };
    let from_header = request
        .headers
        .get("from")
        .map(|v| v.as_str())
        .unwrap_or("");
    registry.context_for_from_header(from_header).await
}

/// 检查租户级并发/CPS 限额，返回 `Some(datagrams)` 表示呼叫被拒绝。
///
/// 应在 `check_user_concurrency` 之后、`resolve_call_source` 之前调用。
/// 若 `tenant_registry` 未注入或租户未配置限额，直接放行。
pub(super) async fn check_tenant_limits(
    request: &SipRequest,
    peer: SocketAddr,
    edge_state: &EdgeState,
) -> Option<(Vec<PendingDatagram>, TenantContext)> {
    let tenant_ctx = resolve_tenant_context_pub(request, edge_state).await;
    if !tenant_ctx.is_bound() {
        return None;
    }

    if let Some(reason) = edge_state.check_tenant_concurrency(&tenant_ctx) {
        warn!(
            tenant_id = tenant_ctx.tenant_id_str(),
            tenant_name = tenant_ctx.tenant_name_str(),
            %reason,
            "rejecting INVITE due to tenant concurrency limit"
        );
        return Some((
            vec![PendingDatagram::new(
                peer.to_string(),
                response::build_response_with_owned_headers(
                    request,
                    486,
                    "Busy Here - Tenant Concurrency Limit Exceeded",
                    &[(
                        "X-VOS-RS-Error".to_string(),
                        format!(
                            "Tenant {} concurrency limit exceeded",
                            tenant_ctx.tenant_id_str()
                        ),
                    )],
                    "",
                ),
            )],
            tenant_ctx,
        ));
    }

    if let Some(reason) = edge_state.check_tenant_cps(&tenant_ctx) {
        warn!(
            tenant_id = tenant_ctx.tenant_id_str(),
            tenant_name = tenant_ctx.tenant_name_str(),
            %reason,
            "rejecting INVITE due to tenant CPS limit"
        );
        return Some((
            vec![PendingDatagram::new(
                peer.to_string(),
                response::build_response_with_owned_headers(
                    request,
                    503,
                    "Service Unavailable - Tenant CPS Limit Exceeded",
                    &[(
                        "X-VOS-RS-Error".to_string(),
                        format!("Tenant {} CPS limit exceeded", tenant_ctx.tenant_id_str()),
                    )],
                    "Retry-After: 1",
                ),
            )],
            tenant_ctx,
        ));
    }

    None
}

/// 解析出口网关、呼叫方向与呼叫源（trunk/extension）。
///
/// 返回 `Ok(CallResolution)` 表示继续后续流程，返回 `Err(datagrams)` 表示被拒绝。
pub(super) async fn resolve_call_source(
    request: &SipRequest,
    peer: SocketAddr,
    edge_state: &EdgeState,
    edge_config: &EdgeConfig,
    tenant_ctx: &TenantContext,
) -> Result<CallResolution, Vec<PendingDatagram>> {
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
    let request_number = request.uri.user.as_deref().unwrap_or("").to_string();
    let inbound_did_destination = egress_trunk_id
        .as_ref()
        .and_then(|_| edge_state.did_destination(&request_number));

    // 租户网关白名单检查：仅在租户已绑定且出口网关已确定时生效
    if let Some(reason) = edge_state.check_tenant_gateway(tenant_ctx, egress_trunk_id.as_deref()) {
        warn!(
            tenant_id = tenant_ctx.tenant_id_str(),
            gateway_id = ?egress_trunk_id,
            %reason,
            "rejecting INVITE due to tenant gateway whitelist"
        );
        return Err(vec![PendingDatagram::new(
            peer.to_string(),
            response::build_response_with_owned_headers(
                request,
                403,
                "Forbidden - Tenant Gateway Not Allowed",
                &[(
                    "X-VOS-RS-Error".to_string(),
                    format!(
                        "Tenant {} does not allow gateway {}",
                        tenant_ctx.tenant_id_str(),
                        egress_trunk_id.as_deref().unwrap_or("unknown")
                    ),
                )],
                "",
            ),
        )]);
    }

    let mut call_source: Option<call_core::CallSource> = None;

    if let Some(ref trunk_id) = egress_trunk_id {
        if trunk_id != "test-gateway"
            && trunk_id != "default"
            && !edge_state
                .call_manager
                .owns_number(&request_number, trunk_id)
        {
            warn!(trunk_id = %trunk_id, number = %request_number, "egress trunk does not own callee number");
            return Err(vec![PendingDatagram::new(
                peer.to_string(),
                response::build_response_with_owned_headers(
                    request,
                    403,
                    "Forbidden - Number Ownership Validation Failed",
                    &[],
                    "",
                ),
            )]);
        }
        if let Some(destination) = inbound_did_destination.as_ref() {
            if destination.target_type == "reject" {
                return Err(vec![PendingDatagram::new(
                    peer.to_string(),
                    response::build_response_with_owned_headers(
                        request,
                        403,
                        "Forbidden - DID Rejected",
                        &[],
                        "",
                    ),
                )]);
            }
            if destination.target_type != "extension"
                && destination.target_type != "extension_group"
                && destination.target_type != "ivr"
            {
                // 数据库 schema 已限制 target_type ∈ {extension, extension_group, ivr, reject}，
                // 走到此分支意味着数据异常，返回 500 并记录告警以便排查。
                warn!(
                    target_type = %destination.target_type,
                    number = %request_number,
                    "DID 目标类型不在支持集合中，疑似数据异常"
                );
                return Err(vec![PendingDatagram::new(
                    peer.to_string(),
                    response::build_response_with_owned_headers(
                        request,
                        500,
                        "Internal Server Error - Invalid DID Target Type",
                        &[],
                        "",
                    ),
                )]);
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
                    let realm = edge_state.resolve_auth_realm(request).await;
                    let auth_res = edge_state
                        .verify_sip_auth(&edge_config.auth, request, true, realm.as_deref())
                        .await;
                    match auth_res {
                        AuthDecision::Challenge => {
                            return Err(vec![PendingDatagram::new(
                                peer.to_string(),
                                proxy_unauthorized_for_request(
                                    request,
                                    &edge_config.auth,
                                    realm.as_deref(),
                                ),
                            )]);
                        }
                        AuthDecision::ChallengeWithFailure => {
                            edge_state.sbc_engine.register_auth_failure(peer.ip());
                            return Err(vec![PendingDatagram::new(
                                peer.to_string(),
                                proxy_unauthorized_for_request(
                                    request,
                                    &edge_config.auth,
                                    realm.as_deref(),
                                ),
                            )]);
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
                return Err(vec![PendingDatagram::new(
                    peer.to_string(),
                    response::build_response_with_owned_headers(
                        request,
                        403,
                        "Forbidden - Overlapping IP Rules",
                        &[],
                        "",
                    ),
                )]);
            }
            _ => {}
        }
    }

    if call_source.is_none() && edge_config.auth.is_enabled() {
        let username_opt = edge_config.auth.authorization_username(request);
        let username = username_opt
            .clone()
            .or_else(|| EdgeState::username_from_request(request))
            .unwrap_or_default();
        let is_trunk = edge_state.is_registered_access_username(&username);
        let realm = edge_state.resolve_auth_realm(request).await;
        let auth_res = edge_state
            .verify_sip_auth(&edge_config.auth, request, is_trunk, realm.as_deref())
            .await;
        match auth_res {
            AuthDecision::Challenge => {
                return Err(vec![PendingDatagram::new(
                    peer.to_string(),
                    proxy_unauthorized_for_request(request, &edge_config.auth, realm.as_deref()),
                )]);
            }
            AuthDecision::ChallengeWithFailure => {
                edge_state.sbc_engine.register_auth_failure(peer.ip());
                return Err(vec![PendingDatagram::new(
                    peer.to_string(),
                    proxy_unauthorized_for_request(request, &edge_config.auth, realm.as_deref()),
                )]);
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
        let username_opt = edge_config.auth.authorization_username(request);
        let username = username_opt
            .clone()
            .or_else(|| EdgeState::username_from_request(request))
            .unwrap_or_else(|| "1001".to_string());
        call_source = Some(call_core::CallSource::new("extension", username));
    }

    let source = call_source.expect("source must be resolved here");
    // 租户只负责开户和网关隔离，费用始终落在实际接入账户。
    let billing_account = if source.source_type == "extension" {
        Some(source.source_id.clone())
    } else {
        edge_state.resolve_trunk_billing_account(&source.source_id)
    };

    Ok(CallResolution {
        source,
        billing_account,
        egress_trunk_id,
        from_gw,
        call_direction,
        request_number,
        inbound_did_destination,
        tenant_ctx: tenant_ctx.clone(),
    })
}

/// 检查跨租户域隔离，返回 `Some(datagrams)` 表示呼叫被拒绝。
///
/// 当 `EdgeState.tenant_registry` 已注入时，使用多租户策略（基于 TenantPolicy）。
/// 否则回退到旧的"同域 + 注册命中"启发式判断，保持向后兼容。
pub(super) async fn check_cross_tenant(
    request: &SipRequest,
    peer: SocketAddr,
    edge_state: &EdgeState,
    caller_domain: &Option<String>,
) -> Option<Vec<PendingDatagram>> {
    let caller_dom = caller_domain.as_ref()?;
    let callee_domain = request.uri.host.to_string();
    let caller_dom_no_port = caller_dom
        .split(':')
        .next()
        .unwrap_or(caller_dom)
        .to_string();
    let callee_dom_no_port = callee_domain
        .split(':')
        .next()
        .unwrap_or(&callee_domain)
        .to_string();
    if callee_dom_no_port == caller_dom_no_port {
        return None;
    }

    // 优先使用多租户策略（若已注入 TenantRegistry）
    if let Some(registry) = edge_state.tenant_registry() {
        let from_header = request
            .headers
            .get("from")
            .map(|v| v.as_str())
            .unwrap_or("");
        let ctx = registry.context_for_from_header(from_header).await;
        if ctx.is_bound() {
            if !ctx.allows_cross_tenant_call_to(&callee_domain) {
                warn!(
                    tenant_id = ctx.tenant_id_str(),
                    tenant_name = ctx.tenant_name_str(),
                    caller_domain = ctx.domain_str(),
                    caller = %from_header,
                    callee_domain = %callee_domain,
                    policy = ?ctx.policy.cross_tenant_policy,
                    "cross-tenant call denied by tenant policy"
                );
                return Some(vec![PendingDatagram::new(
                    peer.to_string(),
                    response::build_response_with_owned_headers(
                        request,
                        403,
                        "Forbidden - Cross-Tenant Calling Disabled",
                        &[(
                            "X-VOS-RS-Error".to_string(),
                            format!(
                                "Tenant {} policy denies cross-tenant calling",
                                ctx.tenant_id_str()
                            ),
                        )],
                        "",
                    ),
                )]);
            }
            // 策略允许，继续后续流程
            return None;
        }
        // 未绑定租户：若域命中了注册表中的 contact（本地分机），仍走旧逻辑拒绝
    }

    // 回退：旧的"同域 + 注册命中"启发式
    let registered_contact = edge_state.lookup_destination_contact(&request.uri).await;
    if registered_contact.is_some() {
        warn!(
            caller = %request.headers.get("from").map(|v| v.as_str()).unwrap_or(""),
            callee = %request.uri,
            "cross-tenant call forbidden by domain isolation"
        );
        Some(vec![PendingDatagram::new(
            peer.to_string(),
            response::build_response_with_owned_headers(
                request,
                403,
                "Forbidden - Cross-Tenant Calling Disabled",
                &[(
                    "X-VOS-RS-Error".to_string(),
                    "Cross-tenant calling is disabled".to_string(),
                )],
                "",
            ),
        )])
    } else {
        None
    }
}
