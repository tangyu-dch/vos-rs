use serde::Serialize;
use std::{net::SocketAddr, sync::Arc};
use tokio::{net::UdpSocket, sync::RwLock};

use crate::media::rtcp_processor::MediaPacketKind;

use super::{
    dtls::{DtlsIdentity, DtlsTransport},
    ice::binding_success_response,
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
}

impl WebRtcSession {
    /// 创建 ICE-Lite/DTLS-SRTP 会话并启动被动 DTLS 服务端握手。
    pub fn start(
        local_port: u16,
        socket: Arc<UdpSocket>,
    ) -> Result<(Self, WebRtcSessionDescription), String> {
        let identity = DtlsIdentity::generate()?;
        let description = WebRtcSessionDescription {
            ice: IceCredentials::generate(),
            fingerprint_sha256: identity.fingerprint().to_string(),
            dtls_setup: "passive",
        };
        let crypto = Arc::new(RwLock::new(None));
        let dtls = Arc::new(DtlsTransport::start(socket, identity, Arc::clone(&crypto)));
        Ok((
            Self {
                local_port,
                ice: description.ice.clone(),
                dtls,
                crypto,
            },
            description,
        ))
    }

    /// 校验 ICE Binding Request 并生成带完整性与指纹的成功响应。
    pub async fn handle_stun_packet(
        &self,
        packet: &[u8],
        source: SocketAddr,
    ) -> Result<Vec<u8>, String> {
        let response = binding_success_response(packet, source, &self.ice)?;
        self.dtls.set_peer(source).await;
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
