//! IVR 按键动作执行模块。
//!
//! 实现 [`execute_ivr_action`]，根据 IVR 菜单配置的按键动作类型执行
//! 分机/PSTN 转接、排队、Webhook、TTS、挂断等操作。

use crate::sip::outbound;
use crate::{EdgeConfig, EdgeState};
use sip_core::{SipRequest, SipUri};
use std::net::SocketAddr;
use std::str::FromStr;
use tracing::{error, info, warn};

/// 供 IVR 拓扑引擎复用的转接动作执行入口。
///
/// 对 [`execute_ivr_action`] 的轻量包装：拓扑引擎不掌握 caller_peer，
/// 此处使用占位地址转发到既有转接逻辑（`execute_ivr_action` 内部未使用该参数）。
pub(crate) async fn execute_ivr_action_for_topology(
    edge_state: &EdgeState,
    edge_config: &EdgeConfig,
    call_id: &str,
    a_port: u16,
    action: &crate::edge_state::IvrAction,
    template_request: &SipRequest,
) {
    let placeholder_peer = std::net::SocketAddr::from(([0u8, 0, 0, 0], 0));
    execute_ivr_action(
        edge_state,
        edge_config,
        call_id,
        a_port,
        action,
        template_request,
        placeholder_peer,
    )
    .await;
}

/// 执行 IVR 菜单按键对应的动作。
///
/// 支持的动作类型：
/// - `extension` / `pstn`：发起 B-leg 出站 INVITE 转接到分机或 PSTN
/// - `queue`：播放 MOH 进入排队
/// - `webhook`：调用第三方 HTTP 接口
/// - `say` / `collect_digits` / `voicemail`：占位实现（仅记录日志）
/// - `hangup`：终止当前呼叫
#[allow(clippy::too_many_arguments)]
pub(super) async fn execute_ivr_action(
    edge_state: &EdgeState,
    edge_config: &EdgeConfig,
    call_id: &str,
    _a_port: u16,
    action: &crate::edge_state::IvrAction,
    template_request: &SipRequest,
    _caller_peer: SocketAddr,
) {
    match action.action_type.as_str() {
        "extension" | "pstn" => {
            info!(call_id, target = %action.action_target, action_type = %action.action_type, "执行 IVR 按键转接动作");

            let mut outbound_uri = None;
            let mut target_override_addr = None;

            if action.action_type == "extension" {
                let mut dest_uri = template_request.uri.clone();
                dest_uri.user = Some(action.action_target.clone().into());
                if let Some(contact) = edge_state.lookup_contact(&dest_uri).await {
                    if let Ok(uri) = SipUri::from_str(&contact.uri) {
                        outbound_uri = Some(uri);
                        target_override_addr = Some(contact.received_from.clone());
                    }
                }
            } else {
                let mut dest_uri = template_request.uri.clone();
                dest_uri.user = Some(action.action_target.clone().into());
                if let Ok(route) = edge_state.call_manager.routes().select(&dest_uri) {
                    outbound_uri = Some(route.outbound_uri);
                }
            }

            let Some(uri) = outbound_uri else {
                warn!(call_id, "IVR 转接目标地址未注册或未配置路由，挂断呼叫");
                edge_state
                    .call_manager
                    .terminate_call_with_reason(call_id, "IVR Transfer Route Not Found");
                return;
            };

            // 发起 B-leg Outbound 呼叫
            let b_call_id = uuid::Uuid::new_v4().to_string();
            let Some(session_id) = edge_state
                .inbound_transactions
                .get(call_id)
                .map(|transaction| transaction.session_id.clone())
            else {
                warn!(call_id, "IVR 转接会话已不存在");
                return;
            };
            edge_state.bind_gateway_dialog(&session_id, &b_call_id);

            let b_relay_endpoint = match edge_state
                .media_relay
                .allocate_endpoint_for_call(&edge_config.media, &session_id)
            {
                Ok(ep) => ep,
                Err(e) => {
                    warn!(call_id, "IVR 转接为 B-leg 分配媒体端点失败: {}", e);
                    edge_state
                        .call_manager
                        .terminate_call_with_reason(call_id, "B-leg Media Alloc Fail");
                    return;
                }
            };

            // 保存 B-leg 媒体端点信息
            if let Some(mut t) = edge_state.inbound_transactions.get_mut(call_id) {
                t.gateway_relay_rtp = Some(b_relay_endpoint.clone());
            }

            // 构造 B-leg SDP Offer
            let sdp_offer = format!(
                "v=0\r\n\
                 o=vos-rs 123456 123456 IN IP4 {addr}\r\n\
                 s=-\r\n\
                 c=IN IP4 {addr}\r\n\
                 t=0 0\r\n\
                 m=audio {port} RTP/AVP 0 8 101\r\n\
                 a=rtpmap:0 PCMU/8000\r\n\
                 a=rtpmap:8 PCMA/8000\r\n\
                 a=rtpmap:101 telephone-event/8000\r\n\
                 a=fmtp:101 0-16\r\n\
                 a=sendrecv\r\n",
                addr = edge_config.media.advertised_addr,
                port = b_relay_endpoint.port,
            );

            let target_peer = target_override_addr
                .clone()
                .unwrap_or_else(|| outbound::target_addr_for(&uri));

            let Some(gateway_local_tag) = edge_state
                .inbound_transactions
                .get(&session_id)
                .map(|transaction| transaction.dialogs.gateway.local_tag.clone())
            else {
                warn!(call_id, "IVR 转接 B-leg 会话已不存在");
                return;
            };
            let invite_bytes = outbound::build_b2bua_outbound_invite(
                template_request,
                &uri,
                &edge_config.advertised_addr,
                sdp_offer.as_bytes(),
                edge_config.session_expires_gateway,
                &[],
                &b_call_id,
                &gateway_local_tag,
                None,
            );

            let socket_sender = edge_state.socket.get().expect("socket initialized");
            if let Ok(addr) = target_peer.parse::<SocketAddr>() {
                info!(call_id, %b_call_id, target = %target_peer, "IVR 转接发送出站 INVITE 至 B-leg");
                if let Err(e) = socket_sender.send_to(&invite_bytes, addr).await {
                    error!(call_id, "发送 B-leg INVITE 数据报失败: {}", e);
                }
            } else {
                warn!(call_id, target = %target_peer, "B-leg 目标 IP 地址解析错误");
            }
        }
        "queue" => {
            info!(call_id, target = %action.action_target, "执行 IVR 排队动作");
            let _ = edge_state
                .media_relay
                .start_playback(
                    _a_port,
                    std::path::PathBuf::from("moh.wav"),
                    crate::media::relay::PlaybackMode::Exclusive,
                    true,
                )
                .await;
            info!(
                call_id,
                "已将呼叫放入队列 {} 并播放 MOH", action.action_target
            );
        }
        "webhook" => {
            info!(call_id, target = %action.action_target, "执行 IVR 第三方 Webhook 动作");
            let method = action.webhook_method.as_deref().unwrap_or("POST");
            let client = reqwest::Client::new();
            let caller = template_request
                .headers
                .get("From")
                .map(|h| h.to_string())
                .unwrap_or_default();
            let callee = template_request
                .uri
                .user
                .as_deref()
                .unwrap_or("")
                .to_string();
            let payload = serde_json::json!({
                "call_id": call_id,
                "dtmf_key": action.action_target,
                "caller": caller,
                "callee": callee,
            });
            let res = if method.eq_ignore_ascii_case("GET") {
                client
                    .get(&action.action_target)
                    .query(&payload)
                    .send()
                    .await
            } else {
                client
                    .post(&action.action_target)
                    .json(&payload)
                    .send()
                    .await
            };
            if let Ok(resp) = res {
                info!(call_id, status = %resp.status(), "第三方 Webhook 返回响应成功");
            } else {
                warn!(call_id, "第三方 Webhook 请求失败");
            }
        }
        "say" => {
            info!(call_id, text = %action.action_target, "执行 IVR TTS 语音朗读动作");
        }
        "collect_digits" => {
            info!(call_id, target = %action.action_target, "执行 IVR 按键收集动作");
        }
        "voicemail" => {
            info!(call_id, target = %action.action_target, "进入 IVR 语音留言录音");
        }
        "hangup" => {
            info!(call_id, "IVR 挂断动作触发，释放呼叫");
            edge_state
                .call_manager
                .terminate_call_with_reason(call_id, "IVR Hangup Action");
        }
        _ => {
            warn!(call_id, action_type = %action.action_type, "未知的 IVR 动作类型");
            edge_state
                .call_manager
                .terminate_call_with_reason(call_id, "Unknown IVR Action Type");
        }
    }
}
