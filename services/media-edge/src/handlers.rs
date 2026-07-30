//! media-edge 扩展控制端点。
//!
//! 本模块聚合了 WebRTC 诊断、会议管理、eBPF/XDP 旁路、AI 语音插件、
//! io_uring 零拷贝传输与 SIP INFO DTMF 注入等控制端点，由 `main.rs`
//! 通过 `Router::merge` 挂载到 `control_routes` 之下。

use axum::{extract::State, Json};
use std::net::SocketAddr;
use std::sync::Arc;

use crate::media::relay::ai_plugin::{AiVoiceFrame, AiVoicePluginProxy};
use crate::media::relay::io_uring::IoUringUdpSocket;
use crate::media::relay::webrtc::WebRtcSessionDiagnostics;
use crate::media::relay::XdpMediaEngine;
use crate::AppState;

// ===== WebRTC 诊断端点 =====

#[derive(serde::Deserialize)]
pub struct WebRtcDiagnosticsReq {
    pub port: u16,
}

/// 查询指定端口的 WebRTC 会话诊断信息（ICE/DTLS 状态、远端候选列表）。
pub async fn webrtc_diagnostics(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<WebRtcDiagnosticsReq>,
) -> Json<Option<WebRtcSessionDiagnostics>> {
    let Some(session) = state
        .media_relay
        .webrtc_sessions
        .get(&payload.port)
        .map(|entry| entry.clone())
    else {
        return Json(None);
    };
    Json(Some(session.diagnostics().await))
}

/// 列出所有活跃 WebRTC 会话的诊断摘要。
pub async fn webrtc_diagnostics_all(
    State(state): State<Arc<AppState>>,
) -> Json<Vec<WebRtcSessionDiagnostics>> {
    let mut diagnostics = Vec::new();
    for entry in state.media_relay.webrtc_sessions.iter() {
        diagnostics.push(entry.diagnostics().await);
    }
    Json(diagnostics)
}

// ===== 会议管理端点 =====

#[derive(serde::Deserialize)]
pub struct JoinConferenceReq {
    pub conference_id: String,
    pub port: u16,
    pub codec: rtp_core::AudioCodec,
    pub target_addr: SocketAddr,
}

/// 将指定端口加入会议（mix-minus 混音）。
pub async fn join_conference(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<JoinConferenceReq>,
) -> Json<Result<bool, String>> {
    let Some(socket) = state
        .media_relay
        .active_sockets
        .get(&payload.port)
        .map(|entry| Arc::clone(entry.value()))
    else {
        return Json(Err(format!("端口 {} 未绑定活动 UDP socket", payload.port)));
    };
    state
        .media_relay
        .conference_manager
        .join_conference(
            &payload.conference_id,
            payload.port,
            payload.codec,
            payload.target_addr,
            socket,
        )
        .await;
    Json(Ok(true))
}

#[derive(serde::Deserialize)]
pub struct LeaveConferenceReq {
    pub port: u16,
}

/// 将指定端口移出会议。
pub async fn leave_conference(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<LeaveConferenceReq>,
) -> Json<bool> {
    state
        .media_relay
        .conference_manager
        .leave_conference(payload.port)
        .await;
    Json(true)
}

#[derive(serde::Deserialize)]
pub struct SetParticipantMuteReq {
    pub conference_id: String,
    pub port: u16,
    pub mute: bool,
}

/// 设置会议成员的静音状态。
pub async fn set_participant_mute(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<SetParticipantMuteReq>,
) -> Json<bool> {
    Json(
        state
            .media_relay
            .conference_manager
            .set_participant_mute(&payload.conference_id, payload.port, payload.mute)
            .await,
    )
}

/// 列出所有活跃会议及其参会人数。
pub async fn list_conferences(State(state): State<Arc<AppState>>) -> Json<Vec<(String, usize)>> {
    Json(state.media_relay.conference_manager.list_conferences())
}

#[derive(serde::Deserialize)]
pub struct ConferenceForPortReq {
    pub port: u16,
}

/// 查询指定端口所属的会议 ID。
pub async fn conference_for_port(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<ConferenceForPortReq>,
) -> Json<Option<String>> {
    Json(
        state
            .media_relay
            .conference_manager
            .conference_for_port(payload.port),
    )
}

// ===== eBPF/XDP 内核旁路端点 =====

#[derive(serde::Deserialize)]
pub struct InitXdpReq {
    pub iface: String,
}

/// 加载 XDP 内核旁路驱动（Linux 专用，非 Linux 平台为占位实现）。
pub async fn init_xdp(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<InitXdpReq>,
) -> Json<Result<XdpStatus, String>> {
    match XdpMediaEngine::new(&payload.iface) {
        Ok(engine) => {
            let status = XdpStatus {
                iface: payload.iface.clone(),
                is_active: engine.is_active(),
                rule_count: engine.rule_count(),
                redirect_count: engine.redirect_count(),
            };
            let mut guard = state.xdp_engines.lock().await;
            guard.insert(payload.iface, engine);
            Json(Ok(status))
        }
        Err(e) => Json(Err(e)),
    }
}

#[derive(serde::Serialize)]
pub struct XdpStatus {
    pub iface: String,
    pub is_active: bool,
    pub rule_count: usize,
    pub redirect_count: u64,
}

#[derive(serde::Deserialize)]
pub struct RegisterXdpRuleReq {
    pub iface: String,
    pub src: std::net::SocketAddrV4,
    pub local_port: u16,
    pub target: std::net::SocketAddrV4,
}

/// 向 XDP 内核 Map 写入一条 RTP 旁路转发规则。
pub async fn register_xdp_rule(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<RegisterXdpRuleReq>,
) -> Json<Result<(), String>> {
    let guard = state.xdp_engines.lock().await;
    let Some(engine) = guard.get(&payload.iface) else {
        return Json(Err(format!(
            "网卡 {} 未加载 XDP 驱动，请先调用 /init_xdp",
            payload.iface
        )));
    };
    Json(
        engine
            .register_rule(payload.src, payload.local_port, payload.target)
            .map_err(|e| e.to_string()),
    )
}

#[derive(serde::Deserialize)]
pub struct UnregisterXdpRuleReq {
    pub iface: String,
    pub src: std::net::SocketAddrV4,
    pub local_port: u16,
}

/// 从 XDP 内核 Map 撤销一条旁路规则。
pub async fn unregister_xdp_rule(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<UnregisterXdpRuleReq>,
) -> Json<Result<(), String>> {
    let guard = state.xdp_engines.lock().await;
    let Some(engine) = guard.get(&payload.iface) else {
        return Json(Err(format!("网卡 {} 未加载 XDP 驱动", payload.iface)));
    };
    Json(
        engine
            .unregister_rule(payload.src, payload.local_port)
            .map_err(|e| e.to_string()),
    )
}

#[derive(serde::Deserialize)]
pub struct XdpStatusReq {
    pub iface: String,
}

/// 查询指定网卡的 XDP 状态。
pub async fn xdp_status(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<XdpStatusReq>,
) -> Json<Option<XdpStatus>> {
    let guard = state.xdp_engines.lock().await;
    Json(guard.get(&payload.iface).map(|engine| XdpStatus {
        iface: engine.iface_name().to_string(),
        is_active: engine.is_active(),
        rule_count: engine.rule_count(),
        redirect_count: engine.redirect_count(),
    }))
}

// ===== AI 语音插件端点 =====

#[derive(serde::Deserialize)]
pub struct StartAiPluginReq {
    pub bind_addr: SocketAddr,
    pub plugin_addr: SocketAddr,
}

/// 启动 AI 语音插件双向代理会话（上行 PCM → AI 插件，下行 TTS → media-edge）。
pub async fn start_ai_plugin(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<StartAiPluginReq>,
) -> Json<Result<String, String>> {
    match AiVoicePluginProxy::start(payload.bind_addr, payload.plugin_addr).await {
        Ok(proxy) => {
            let id = format!("{}-{}", payload.bind_addr, payload.plugin_addr);
            let mut guard = state.ai_proxies.lock().await;
            guard.insert(id.clone(), proxy);
            Json(Ok(id))
        }
        Err(e) => Json(Err(e)),
    }
}

#[derive(serde::Deserialize)]
pub struct SendAiUpstreamReq {
    pub session_id: String,
    pub call_id: u32,
    pub seq: u32,
    pub timestamp: u64,
    pub pcm_base64: String,
}

/// 向 AI 语音插件上行通道推送一帧 PCM 音频。
pub async fn send_ai_upstream(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<SendAiUpstreamReq>,
) -> Json<Result<(), String>> {
    let pcm_data = match base64_decode(&payload.pcm_base64) {
        Ok(data) => data,
        Err(e) => return Json(Err(format!("base64 解码失败: {e}"))),
    };
    let frame = AiVoiceFrame {
        call_id: payload.call_id,
        seq: payload.seq,
        timestamp: payload.timestamp,
        pcm_data,
    };
    let guard = state.ai_proxies.lock().await;
    let Some(proxy) = guard.get(&payload.session_id) else {
        return Json(Err(format!("AI 插件会话 {} 不存在", payload.session_id)));
    };
    Json(proxy.send_upstream(frame).await)
}

#[derive(serde::Deserialize)]
pub struct RecvAiDownstreamReq {
    pub session_id: String,
}

#[derive(serde::Serialize)]
pub struct RecvAiDownstreamResp {
    pub frame: Option<AiVoiceFrame>,
}

/// 尝试非阻塞地从 AI 插件下行通道获取一帧 TTS 音频。
pub async fn try_recv_ai_downstream(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<RecvAiDownstreamReq>,
) -> Json<Result<RecvAiDownstreamResp, String>> {
    let guard = state.ai_proxies.lock().await;
    let Some(proxy) = guard.get(&payload.session_id) else {
        return Json(Err(format!("AI 插件会话 {} 不存在", payload.session_id)));
    };
    let frame = proxy.try_recv_downstream().await;
    Json(Ok(RecvAiDownstreamResp { frame }))
}

/// 简易 base64 解码（避免引入额外依赖）。
fn base64_decode(input: &str) -> Result<Vec<u8>, String> {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let input: Vec<u8> = input
        .bytes()
        .filter(|&b| b != b'\n' && b != b'\r' && b != b' ')
        .collect();
    if !input.len().is_multiple_of(4) {
        return Err(format!("invalid base64 length: {}", input.len()));
    }
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    for chunk in input.chunks(4) {
        let mut buf = [0u8; 4];
        for (i, &b) in chunk.iter().enumerate() {
            buf[i] = if b == b'=' {
                0
            } else {
                TABLE
                    .iter()
                    .position(|&t| t == b)
                    .ok_or_else(|| format!("invalid base64 char: {}", b as char))?
                    as u8
            };
        }
        out.push((buf[0] << 2) | (buf[1] >> 4));
        if chunk[2] != b'=' {
            out.push((buf[1] << 4) | (buf[2] >> 2));
            if chunk[3] != b'=' {
                out.push((buf[2] << 6) | buf[3]);
            }
        }
    }
    Ok(out)
}

// ===== io_uring 零拷贝传输端点 =====

#[derive(serde::Deserialize)]
pub struct InitIoUringReq {
    pub bind_addr: SocketAddr,
    pub queue_depth: u32,
}

#[derive(serde::Serialize)]
pub struct IoUringStatus {
    pub bind_addr: SocketAddr,
    pub is_active: bool,
    pub queue_depth: u32,
}

/// 初始化 io_uring UDP 零拷贝通道（Linux 专用，非 Linux 平台为占位实现）。
pub async fn init_io_uring(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<InitIoUringReq>,
) -> Json<Result<IoUringStatus, String>> {
    match IoUringUdpSocket::bind(payload.bind_addr, payload.queue_depth) {
        Ok(socket) => {
            let status = IoUringStatus {
                bind_addr: socket.local_addr(),
                is_active: socket.is_active(),
                queue_depth: socket.queue_depth(),
            };
            let mut guard = state.io_uring_sockets.lock().await;
            guard.insert(payload.bind_addr, socket);
            Json(Ok(status))
        }
        Err(e) => Json(Err(e.to_string())),
    }
}

#[derive(serde::Deserialize)]
pub struct PollIoUringReq {
    pub bind_addr: SocketAddr,
    pub max_packets: usize,
}

#[derive(serde::Serialize)]
pub struct IoUringPollResult {
    pub bind_addr: SocketAddr,
    pub packets: Vec<IoUringPacketEntry>,
}

#[derive(serde::Serialize)]
pub struct IoUringPacketEntry {
    pub source: SocketAddr,
    pub size: usize,
}

/// 从 io_uring Ring 缓冲区批量提取就绪的 UDP 数据包（诊断用）。
pub async fn poll_io_uring(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<PollIoUringReq>,
) -> Json<Result<IoUringPollResult, String>> {
    let guard = state.io_uring_sockets.lock().await;
    let Some(socket) = guard.get(&payload.bind_addr) else {
        return Json(Err(format!(
            "地址 {} 未初始化 io_uring 通道",
            payload.bind_addr
        )));
    };
    let raw_packets = socket.poll_recv_batch(payload.max_packets);
    let packets = raw_packets
        .into_iter()
        .map(|(data, source)| IoUringPacketEntry {
            source,
            size: data.len(),
        })
        .collect();
    Json(Ok(IoUringPollResult {
        bind_addr: socket.local_addr(),
        packets,
    }))
}

// ===== SIP INFO DTMF 注入端点 =====

#[derive(serde::Deserialize)]
pub struct RegisterInfoDtmfDigitReq {
    pub call_id: String,
    pub digit: char,
}

/// 注入通过 SIP INFO 消息接收到的 DTMF 数字到指定呼叫的累积器。
pub async fn register_info_dtmf_digit(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<RegisterInfoDtmfDigitReq>,
) -> Json<bool> {
    state
        .media_relay
        .register_info_dtmf_digit(&payload.call_id, payload.digit);
    Json(true)
}
