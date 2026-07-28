//! IVR（交互式语音应答）模块入口。
//!
//! 主入口 [`handle_ivr_locally`] 处理呼入到 IVR 的呼叫：
//! 1. 分配 A-leg 本地媒体端点
//! 2. 解析客户端 SDP 并配置 RTP 转发
//! 3. 应答 200 OK 并启动 DTMF 检测
//! 4. 根据 IVR 菜单配置分流：
//!    - 有拓扑画布：进入 [`super::ivr_topology`] 执行引擎
//!    - 无拓扑画布：进入 [`menu_loop::run_ivr_menu_loop`] 扁平 DTMF 状态机

mod actions;
mod menu_loop;

#[cfg(test)]
mod tests;

pub(crate) use actions::execute_ivr_action_for_topology;

use crate::edge_state::PendingDatagram;
use crate::sip::response;
use crate::{EdgeConfig, EdgeState};
use sip_core::SipRequest;
use std::net::SocketAddr;
use std::sync::Arc;
use tracing::{info, warn};

/// 处理呼入到 IVR 的呼叫。
///
/// 在 B2BUA 中为呼入呼叫建立 A-leg 媒体通道，应答 200 OK 后
/// 根据 `did_dest.target_id` 对应的 IVR 菜单配置启动相应的执行引擎。
pub(crate) async fn handle_ivr_locally(
    request: SipRequest,
    peer: SocketAddr,
    edge_state: &EdgeState,
    edge_config: &EdgeConfig,
    did_dest: &cdr_core::DidDestination,
) -> Vec<PendingDatagram> {
    let internal_call_id = request
        .headers
        .get("call-id")
        .map(|v| v.as_str().to_string())
        .unwrap_or_default();
    let session_id = uuid::Uuid::new_v4().to_string();

    info!(call_id = %internal_call_id, ivr_id = %did_dest.target_id, "呼入呼叫进入本地 IVR 流程");

    // 1. 分配 A-leg 本地媒体端点
    let a_relay_endpoint = match edge_state
        .media_relay
        .allocate_endpoint_for_call(&edge_config.media, &session_id)
    {
        Ok(ep) => ep,
        Err(e) => {
            warn!(error = %e, "IVR 流程分配媒体端点失败");
            return vec![PendingDatagram::new(
                peer.to_string(),
                response::build_response_with_owned_headers(
                    &request,
                    500,
                    "Internal Server Error - Media Allocation Failed",
                    &[],
                    "",
                ),
            )];
        }
    };

    // 2. 解析客户端 SDP 并注册编解码器
    let client_ep = match crate::media::sdp::parse_sdp_rtp_endpoint(&request.body) {
        Ok(ep) => ep,
        Err(e) => {
            warn!(error = %e, "IVR 流程解析客户端 SDP 失败");
            edge_state.media_relay.clear_target(a_relay_endpoint.port);
            return vec![PendingDatagram::new(
                peer.to_string(),
                response::build_response_with_owned_headers(
                    &request,
                    400,
                    "Bad Request - SDP Parsing Failed",
                    &[],
                    "",
                ),
            )];
        }
    };

    let _client_addr = match crate::media::sdp::socket_addr_for_endpoint(&client_ep) {
        Ok(addr) => addr,
        Err(e) => {
            warn!(error = %e, "IVR 流程解析客户端 socket 地址失败");
            edge_state.media_relay.clear_target(a_relay_endpoint.port);
            return vec![PendingDatagram::new(
                peer.to_string(),
                response::build_response_with_owned_headers(
                    &request,
                    400,
                    "Bad Request - Invalid SDP Address",
                    &[],
                    "",
                ),
            )];
        }
    };

    let codec = crate::media::sdp::negotiated_audio_codec(&request.body)
        .unwrap_or(rtp_core::AudioCodec::Pcma);
    edge_state
        .media_relay
        .register_port_codec(a_relay_endpoint.port, codec);

    if let Err(e) = edge_state
        .media_relay
        .set_target(&a_relay_endpoint, &client_ep)
    {
        warn!(error = %e, "IVR 流程设置 RTP 转发目标失败");
        edge_state.media_relay.clear_target(a_relay_endpoint.port);
        return vec![PendingDatagram::new(
            peer.to_string(),
            response::build_response_with_owned_headers(
                &request,
                500,
                "Internal Server Error - Media Setup Failed",
                &[],
                "",
            ),
        )];
    }

    // 记录呼入 invite 并响应 200 OK
    edge_state.remember_inbound_invite(
        session_id.clone(),
        &request,
        peer,
        request.uri.clone(),
        Some(client_ep),
        Some(a_relay_endpoint.clone()),
        Some(3600),
    );

    let sdp_answer = format!(
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
        port = a_relay_endpoint.port,
    );

    let response_bytes = response::build_response_with_owned_headers(
        &request,
        200,
        "OK",
        &[
            ("Content-Type".to_string(), "application/sdp".to_string()),
            (
                "Contact".to_string(),
                format!("<sip:{}@{}>", internal_call_id, edge_config.advertised_addr),
            ),
        ],
        &sdp_answer,
    );

    // 启动 DTMF 检测
    edge_state
        .media_relay
        .register_port_dtmf_tracking(&session_id, a_relay_endpoint.port, 101);

    // 查找 IVR 菜单
    let menu = edge_state
        .ivr_menus
        .read()
        .ok()
        .and_then(|lock| lock.get(&did_dest.target_id).cloned());

    let Some(ivr_menu) = menu else {
        warn!(call_id = %internal_call_id, "未找到指定的 IVR 菜单配置");
        return vec![PendingDatagram::new(peer.to_string(), response_bytes)];
    };

    let edge_state_clone = match edge_state.self_weak.get().and_then(|w| w.upgrade()) {
        Some(arc) => arc,
        None => {
            warn!("无法升级 edge_state 为 Arc");
            return vec![PendingDatagram::new(peer.to_string(), response_bytes)];
        }
    };
    let edge_config_clone = Arc::new(edge_config.clone());
    let a_port = a_relay_endpoint.port;
    let internal_call_id_clone = internal_call_id.clone();
    let request_clone = request.clone();

    // 有拓扑画布时走拓扑执行引擎，否则走扁平 DTMF 表（向后兼容）
    if let Some(topology) = ivr_menu.topology.as_ref().filter(|t| !t.nodes.is_empty()) {
        spawn_topology_execution(
            edge_state_clone,
            edge_config_clone,
            topology.clone(),
            internal_call_id_clone,
            did_dest.number.clone(),
            a_port,
            peer,
            request_clone,
        );
        return vec![PendingDatagram::new(peer.to_string(), response_bytes)];
    }

    let current_menu = ivr_menu;

    // 启动 IVR 状态机后台监测协程
    tokio::spawn(async move {
        menu_loop::run_ivr_menu_loop(
            edge_state_clone,
            edge_config_clone,
            internal_call_id_clone,
            session_id,
            a_port,
            request_clone,
            peer,
            current_menu,
        )
        .await;
    });

    vec![PendingDatagram::new(peer.to_string(), response_bytes)]
}

/// 在独立 task 中执行 IVR 拓扑引擎。
///
/// 从 `template_request` 的 From 头提取主叫号码，构建 [`IvrExecutionContext`] 后
/// 调用 [`crate::sip::handlers::ivr_topology::execute`]。
#[allow(clippy::too_many_arguments)]
fn spawn_topology_execution(
    edge_state: Arc<EdgeState>,
    edge_config: Arc<EdgeConfig>,
    topology: crate::sip::handlers::ivr_topology::IvrTopology,
    call_id: String,
    did: String,
    a_port: u16,
    peer: SocketAddr,
    template_request: SipRequest,
) {
    let caller_id = template_request
        .headers
        .get("from")
        .and_then(|v| v.as_str().split("sip:").nth(1))
        .and_then(|s| s.split('@').next())
        .unwrap_or_default()
        .to_string();
    let mut context = crate::sip::handlers::ivr_topology::IvrExecutionContext::new(
        call_id.clone(),
        caller_id,
        did,
    );
    info!(
        call_id = %call_id,
        nodes = topology.nodes.len(),
        "IVR 菜单具备拓扑画布, 分流至拓扑执行引擎"
    );
    tokio::spawn(async move {
        crate::sip::handlers::ivr_topology::execute(
            &edge_state,
            &edge_config,
            &topology,
            &mut context,
            a_port,
            peer,
            &template_request,
        )
        .await;
    });
}
