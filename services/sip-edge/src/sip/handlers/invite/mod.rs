use std::net::SocketAddr;
use std::sync::atomic::Ordering;

use sip_core::SipRequest;
use tracing::{debug, info};

use crate::config::EdgeConfig;
use crate::edge_state::{EdgeState, PendingDatagram};
use crate::sip::response;

mod conference;
mod outbound;
mod resolution;
mod routing;

pub(crate) async fn handle_invite_request(
    request: SipRequest,
    peer: SocketAddr,
    edge_state: &EdgeState,
    edge_config: &EdgeConfig,
) -> Vec<PendingDatagram> {
    let session_id = uuid::Uuid::new_v4().to_string();

    // 1. 500ms 内重复 INVITE 抑制（仅对无 Authorization 头的请求生效）
    if let Some(call_id) = request.headers.get("call-id").map(|v| v.as_str()) {
        let has_auth = request.headers.get("authorization").is_some()
            || request.headers.get("proxy-authorization").is_some();
        let now = std::time::Instant::now();
        if !has_auth {
            if let Some(last_seen) = edge_state.recent_inbound_invites.get(call_id) {
                if now.duration_since(*last_seen) < std::time::Duration::from_millis(500) {
                    debug!(
                        call_id,
                        "duplicate inbound INVITE received within 500ms, returning empty datagrams"
                    );
                    return Vec::new();
                }
            }
        }
        edge_state
            .recent_inbound_invites
            .insert(call_id.to_string(), now);
    }

    // 2. Drain 状态下拒绝新呼叫
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

    // 3. 解析 callee 并判定是否为会议呼叫
    let callee = extract_callee_from_to_header(&request);
    let callee_num = request.uri.user.as_deref().unwrap_or("");
    let is_conf = callee.starts_with("conf_")
        || callee.starts_with("room_")
        || callee_num.starts_with("conf_")
        || callee_num.starts_with("room_");

    if is_conf {
        let conf_id = if callee.starts_with("conf_") || callee.starts_with("room_") {
            callee.as_str()
        } else {
            callee_num
        };
        return conference::handle_conference_invitation(
            &request,
            peer,
            edge_state,
            edge_config,
            &session_id,
            conf_id,
        )
        .await;
    }

    // 4. 反欺诈规则检查（黑名单、并发限制）
    if let Some(rejection) = conference::check_anti_fraud_rules(&request, peer, edge_state) {
        return rejection;
    }

    // 5. 单用户并发上限检查
    if let Some(rejection) =
        resolution::check_user_concurrency(&request, peer, edge_state, edge_config)
    {
        return rejection;
    }

    // 6. 解析呼叫源（trunk/extension、出口网关、呼叫方向、计费账户）
    let call_resolution =
        match resolution::resolve_call_source(&request, peer, edge_state, edge_config).await {
            Ok(resolution) => resolution,
            Err(rejection) => return rejection,
        };

    // 7. 跨租户域隔离检查
    let caller_domain = EdgeState::domain_from_request(&request);
    if let Some(rejection) =
        resolution::check_cross_tenant(&request, peer, edge_state, &caller_domain).await
    {
        return rejection;
    }

    // 8. Webhook 控制模式与 DID 离线检查
    let route_preparation = match routing::prepare_webhook_and_did(
        &request,
        peer,
        edge_state,
        edge_config,
        call_resolution.from_gw,
        &call_resolution.inbound_did_destination,
        &call_resolution.request_number,
    )
    .await
    {
        Ok(preparation) => preparation,
        Err(rejection) => return rejection,
    };

    // 9. 路由分发（IVR / extension_group / 标准分机 / 网关）
    let routing_outcome = match routing::dispatch_routing(
        &request,
        peer,
        edge_state,
        edge_config,
        &call_resolution,
        &route_preparation.registered_contact,
    )
    .await
    {
        Ok(outcome) => outcome,
        Err(rejection) => return rejection,
    };

    // 10. 出站 INVITE 分发（计费、余额、资源租约、SDP 改写、forking）
    let ctx = outbound::OutboundContext {
        request: &request,
        peer,
        edge_state,
        edge_config,
        session_id: &session_id,
        egress_trunk_id: &call_resolution.egress_trunk_id,
        billing_account: &call_resolution.billing_account,
        caller_domain: &caller_domain,
        inbound_did_destination: &call_resolution.inbound_did_destination,
        registered_contact: &route_preparation.registered_contact,
        response: routing_outcome.response,
        outbound_invite: routing_outcome.outbound_invite,
    };

    outbound::dispatch_outbound_invite(ctx).await
}

/// 从 To 头中提取 callee 用户名部分。
fn extract_callee_from_to_header(request: &SipRequest) -> String {
    request
        .headers
        .get("to")
        .and_then(|v| {
            let s = v.as_str();
            let start = s.find("sip:").map(|i| i + 4)?;
            let end = s[start..].find('@')?;
            Some(s[start..start + end].to_string())
        })
        .unwrap_or_default()
}
