use std::net::SocketAddr;
use std::str::FromStr;

use sip_core::{SipRequest, SipUri};
use tracing::{info, warn};

use crate::config::EdgeConfig;
use crate::edge_state::{EdgeState, PendingDatagram};
use crate::sip::response;

use super::super::response_for_media_error;

/// 处理会议 INVITE，自动分配媒体中继端口并将参会成员加入混音管理器。
///
/// 完成以下步骤：
/// 1. 为会议分配媒体中继端口
/// 2. 解析客户端 SDP 媒体端点与编解码器
/// 3. 将参会成员加入混音管理器
/// 4. 在 CallManager 中建立呼叫会话
/// 5. 记录事务状态
/// 6. 返回 SDP 应答并发送 200 OK
pub(super) async fn handle_conference_invitation(
    request: &SipRequest,
    peer: SocketAddr,
    edge_state: &EdgeState,
    edge_config: &EdgeConfig,
    session_id: &str,
    conf_id: &str,
) -> Vec<PendingDatagram> {
    info!(conf_id, "incoming SIP INVITE to join conference");

    // 1. 自动为会议分配媒体中继端口
    let local_ep = match edge_state
        .media_relay
        .allocate_endpoint_for_call(&edge_config.media, session_id)
    {
        Ok(ep) => ep,
        Err(e) => {
            warn!(error = %e, "failed to allocate endpoint for conference");
            return vec![PendingDatagram::new(
                peer.to_string(),
                response_for_media_error(request, &e),
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
                response_for_media_error(request, &e),
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
                response_for_media_error(request, &e),
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
                    request,
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
        request,
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
        session_id.to_string(),
        request,
        peer,
        SipUri::from_str(&format!("sip:{}@localhost", conf_id)).unwrap(),
        Some(client_ep.clone()),
        Some(local_ep.clone()),
        None,
    );

    // 修正本地中继关联为 caller 侧中继
    if let Some(mut tx) = edge_state.inbound_transactions.get_mut(&internal_call_id) {
        tx.caller_relay_rtp = Some(local_ep.clone());
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
        request,
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

    vec![PendingDatagram::new(peer.to_string(), response)]
}

/// 检查反欺诈规则（黑名单、并发限制），返回 `Some(datagrams)` 表示呼叫被拦截。
pub(super) fn check_anti_fraud_rules(
    request: &SipRequest,
    peer: SocketAddr,
    edge_state: &EdgeState,
) -> Option<Vec<PendingDatagram>> {
    let rules = edge_state
        .anti_fraud_rules
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let caller = EdgeState::username_from_request(request).unwrap_or_default();
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
                    return Some(vec![PendingDatagram::new(
                        peer.to_string(),
                        response::build_response_with_owned_headers(
                            request,
                            403,
                            "Forbidden - Anti-Fraud Blacklist Match",
                            &[(
                                "X-VOS-RS-Error".to_string(),
                                "Callee number is blacklisted".to_string(),
                            )],
                            "",
                        ),
                    )]);
                }
            }
            "user_concurrency" | "limit_concurrent" | "ip_concurrency" => {
                let limit = rule.limit_number.unwrap_or(0) as u32;
                if rule.target_value == client_ip {
                    let current_ip_concurrency = edge_state
                        .inbound_transactions
                        .iter()
                        .filter(|entry| {
                            entry
                                .value()
                                .dialogs
                                .caller
                                .peer
                                .as_deref()
                                .map(|p| p.contains(&client_ip))
                                .unwrap_or(false)
                        })
                        .count() as u32;
                    if current_ip_concurrency >= limit {
                        warn!(%client_ip, current_ip_concurrency, limit, "防盗打限制：IP 并发超限被拦截");
                        return Some(vec![PendingDatagram::new(
                            peer.to_string(),
                            response::build_response_with_owned_headers(
                                request,
                                503,
                                "Service Unavailable - IP Concurrency Limit Exceeded",
                                &[(
                                    "X-VOS-RS-Error".to_string(),
                                    "IP concurrent call limit exceeded".to_string(),
                                )],
                                "",
                            ),
                        )]);
                    }
                } else if rule.target_value == caller && !caller.is_empty() {
                    let active_count = edge_state.user_concurrent_count(&caller);
                    if active_count >= limit {
                        warn!(%caller, active_count, limit, "防盗打限制：用户并发超限被拦截");
                        return Some(vec![PendingDatagram::new(
                            peer.to_string(),
                            response::build_response_with_owned_headers(
                                request,
                                503,
                                "Service Unavailable - User Concurrency Limit Exceeded",
                                &[(
                                    "X-VOS-RS-Error".to_string(),
                                    "User concurrent call limit exceeded".to_string(),
                                )],
                                "",
                            ),
                        )]);
                    }
                }
            }
            _ => {}
        }
    }

    None
}
