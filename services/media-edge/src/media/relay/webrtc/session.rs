use serde::Serialize;
use std::{net::SocketAddr, sync::Arc};
use tokio::{net::UdpSocket, sync::RwLock};

use crate::media::rtcp_processor::MediaPacketKind;

use super::{
    dtls::{DtlsIdentity, DtlsTransport},
    ice::{binding_success_response, CandidateSummary, IceAgent, RemoteCandidate},
    srtp::SrtpContexts,
    IceCredentials,
};

/// 返回给信令层、用于生成 WebRTC SDP Answer 的会话参数。
#[derive(Debug, Clone, Serialize)]
pub struct WebRtcSessionDescription {
    pub ice: IceCredentials,
    pub fingerprint_sha256: String,
    pub dtls_setup: &'static str,
}

/// media-edge 中的一条浏览器 WebRTC 媒体腿。
#[derive(Clone)]
pub struct WebRtcSession {
    local_port: u16,
    ice: IceCredentials,
    dtls: Arc<DtlsTransport>,
    crypto: Arc<RwLock<Option<Arc<SrtpContexts>>>>,
    ice_agent: Arc<tokio::sync::Mutex<IceAgent>>,
    pub ice_connected: Arc<std::sync::atomic::AtomicBool>,
    pub dtls_connected: Arc<std::sync::atomic::AtomicBool>,
    pub dtls_failed: Arc<std::sync::atomic::AtomicBool>,
}

impl WebRtcSession {
    /// 创建 ICE-Lite/DTLS-SRTP 会话并启动被动 DTLS 服务端握手。
    pub fn start(
        local_port: u16,
        socket: Arc<UdpSocket>,
    ) -> Result<(Self, WebRtcSessionDescription), String> {
        let identity = DtlsIdentity::generate()?;
        let ice_credentials = IceCredentials::generate();
        let description = WebRtcSessionDescription {
            ice: ice_credentials.clone(),
            fingerprint_sha256: identity.fingerprint().to_string(),
            dtls_setup: "passive",
        };
        let crypto = Arc::new(RwLock::new(None));
        let ice_connected = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let dtls_connected = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let dtls_failed = Arc::new(std::sync::atomic::AtomicBool::new(false));

        let ice_agent = Arc::new(tokio::sync::Mutex::new(IceAgent::new(
            ice_credentials.username_fragment.clone(),
        )));

        let dtls = Arc::new(DtlsTransport::start(
            socket,
            identity,
            Arc::clone(&crypto),
            Arc::clone(&dtls_connected),
            Arc::clone(&dtls_failed),
        ));
        Ok((
            Self {
                local_port,
                ice: description.ice.clone(),
                dtls,
                crypto,
                ice_agent,
                ice_connected,
                dtls_connected,
                dtls_failed,
            },
            description,
        ))
    }

    /// 设置远端 SDP 中解析得到的 ICE 凭据与候选列表。
    ///
    /// 由信令层在收到远端 SDP answer/offer 后调用，将 `a=ice-ufrag` 与
    /// `a=ice-pwd` 属性注入 ICE agent，供后续 STUN MESSAGE-INTEGRITY 校验使用。
    pub async fn set_remote_ice_credentials(&self, ufrag: String, password: String) {
        self.ice_agent
            .lock()
            .await
            .set_remote_credentials(ufrag, password);
    }

    /// 添加从远端 SDP `a=candidate:` 行解析的候选地址。
    ///
    /// 信令层应遍历远端 SDP 中所有 `a=candidate:` 行，调用 `parse_candidate_line`
    /// 解析后通过此方法注入。ICE agent 会去重并学习已知 peer 地址。
    pub async fn add_remote_candidate(&self, candidate: RemoteCandidate) {
        self.ice_agent.lock().await.add_remote_candidate(candidate);
    }

    /// 校验 ICE Binding Request 并生成带完整性与指纹的成功响应。
    ///
    /// 同时更新 ICE agent 状态：
    /// - 学习 peer-reflexive candidate（若源地址未知）
    /// - 标记 selected candidate pair（若对端为 controlling role）
    /// - 检测 ICE role conflict 并记录告警（RFC 8445 §7.3.1.1）
    pub async fn handle_stun_packet(
        &self,
        packet: &[u8],
        source: SocketAddr,
    ) -> Result<Vec<u8>, String> {
        let response = binding_success_response(packet, source, &self.ice)?;

        // 更新 ICE agent 状态
        let mut agent = self.ice_agent.lock().await;
        agent.learn_or_match_peer_address(source);

        // 解析 STUN 请求以检查 USE-CANDIDATE 与 role conflict
        let mut request = stun::message::Message::new();
        request.raw.clear();
        request.raw.extend_from_slice(packet);
        let request_valid = request.decode().is_ok();

        if request_valid {
            if agent.is_use_candidate(&request) {
                agent.mark_selected(source);
                if let Some(selected_addr) = agent.selected_remote_address() {
                    tracing::info!(
                        local_port = self.local_port,
                        local_ufrag = agent.local_ufrag(),
                        selected = %selected_addr,
                        "ICE selected pair established (USE-CANDIDATE)"
                    );
                }
            }
            // RFC 8445 §7.3.1.1: 若对端发送 ICE-CONTROLLED 而本地也是 controlled
            // （ICE-Lite），则发生 role conflict。ICE-Lite 实现保持 controlled 角色，
            // 仅记录告警以辅助排障。
            if agent.check_role_conflict(&request) {
                tracing::warn!(
                    local_port = self.local_port,
                    local_ufrag = agent.local_ufrag(),
                    %source,
                    "ICE role conflict detected: both peers are controlled; \
                     staying controlled as ICE-Lite per RFC 8445 §7.3.1.1"
                );
            }
        }

        drop(agent);

        self.dtls.set_peer(source).await;
        self.ice_connected
            .store(true, std::sync::atomic::Ordering::Release);
        Ok(response)
    }

    /// 将复用端口收到的 DTLS 报文送入标准握手状态机。
    pub fn handle_dtls_packet(&self, packet: &[u8]) -> Result<(), String> {
        self.dtls.push_packet(packet)
    }

    pub(crate) async fn decrypt(
        &self,
        packet_kind: MediaPacketKind,
        packet: &[u8],
    ) -> Result<Vec<u8>, String> {
        let contexts = self
            .crypto
            .read()
            .await
            .clone()
            .ok_or_else(|| format!("端口 {} 的 DTLS 握手尚未完成", self.local_port))?;
        match packet_kind {
            MediaPacketKind::Rtp => contexts.decrypt_rtp(packet).await,
            MediaPacketKind::Rtcp => contexts.decrypt_rtcp(packet).await,
        }
    }

    pub(crate) async fn encrypt(
        &self,
        packet_kind: MediaPacketKind,
        packet: &[u8],
    ) -> Result<Vec<u8>, String> {
        let contexts = self
            .crypto
            .read()
            .await
            .clone()
            .ok_or_else(|| format!("端口 {} 的 DTLS 握手尚未完成", self.local_port))?;
        match packet_kind {
            MediaPacketKind::Rtp => contexts.encrypt_rtp(packet).await,
            MediaPacketKind::Rtcp => contexts.encrypt_rtcp(packet).await,
        }
    }

    /// 返回 ICE 连通性与远端候选的诊断快照，供运维监控端点使用。
    pub async fn diagnostics(&self) -> WebRtcSessionDiagnostics {
        let agent = self.ice_agent.lock().await;
        WebRtcSessionDiagnostics {
            local_port: self.local_port,
            ice_connected: self
                .ice_connected
                .load(std::sync::atomic::Ordering::Acquire),
            dtls_connected: self
                .dtls_connected
                .load(std::sync::atomic::Ordering::Acquire),
            dtls_failed: self.dtls_failed.load(std::sync::atomic::Ordering::Acquire),
            local_ufrag: agent.local_ufrag().to_string(),
            remote_candidate_count: agent.remote_candidate_count(),
            highest_priority_remote: agent
                .highest_priority_candidate()
                .map(|candidate| candidate.address),
            remote_candidates: agent.candidate_summaries(),
            selected_remote: agent.selected_remote_address(),
        }
    }
}

/// WebRTC 会话诊断信息，由 `WebRtcSession::diagnostics` 返回。
#[derive(Debug, Clone, serde::Serialize)]
pub struct WebRtcSessionDiagnostics {
    pub local_port: u16,
    pub ice_connected: bool,
    pub dtls_connected: bool,
    pub dtls_failed: bool,
    pub local_ufrag: String,
    pub remote_candidate_count: usize,
    pub highest_priority_remote: Option<SocketAddr>,
    pub remote_candidates: Vec<CandidateSummary>,
    pub selected_remote: Option<SocketAddr>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use tokio::time::{timeout, Duration};
    use webrtc_dtls::{
        config::Config, conn::DTLSConn, crypto::Certificate,
        extension::extension_use_srtp::SrtpProtectionProfile,
    };
    use webrtc_util::conn::Conn;

    #[tokio::test]
    async fn real_dtls_handshake_installs_srtp_contexts() {
        let server_socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let server_address = server_socket.local_addr().unwrap();
        let (session, description) =
            WebRtcSession::start(server_address.port(), Arc::clone(&server_socket)).unwrap();

        let client_socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let client_address = client_socket.local_addr().unwrap();
        client_socket.connect(server_address).await.unwrap();
        session.dtls.set_peer(client_address).await;

        let server_session = session.clone();
        let receiver = tokio::spawn(async move {
            let mut buffer = vec![0_u8; 2_048];
            loop {
                let (size, _) = server_socket.recv_from(&mut buffer).await.unwrap();
                server_session.handle_dtls_packet(&buffer[..size]).unwrap();
            }
        });

        let client_config = Config {
            certificates: vec![
                Certificate::generate_self_signed(vec!["browser".to_string()]).unwrap(),
            ],
            srtp_protection_profiles: vec![SrtpProtectionProfile::Srtp_Aes128_Cm_Hmac_Sha1_80],
            insecure_skip_verify: true,
            server_name: "vos-rs".to_string(),
            ..Default::default()
        };
        let client_connection: Arc<dyn Conn + Send + Sync> = client_socket;
        let client = timeout(
            Duration::from_secs(5),
            DTLSConn::new(client_connection, client_config, true, None),
        )
        .await
        .unwrap()
        .unwrap();

        timeout(Duration::from_secs(2), async {
            while session.crypto.read().await.is_none() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();

        let peer_certificate = client
            .connection_state()
            .await
            .peer_certificates
            .first()
            .cloned()
            .unwrap();
        let actual_fingerprint = Sha256::digest(peer_certificate)
            .iter()
            .map(|byte| format!("{byte:02X}"))
            .collect::<Vec<_>>()
            .join(":");
        assert_eq!(actual_fingerprint, description.fingerprint_sha256);
        receiver.abort();
    }
}
