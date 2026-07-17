use async_trait::async_trait;
use sha2::{Digest, Sha256};
use std::{net::SocketAddr, sync::Arc};
use tokio::{
    net::UdpSocket,
    sync::{mpsc, Mutex, RwLock},
};
use webrtc_dtls::{
    config::{Config, ExtendedMasterSecretType},
    conn::DTLSConn,
    crypto::Certificate,
    extension::extension_use_srtp::SrtpProtectionProfile,
};
use webrtc_util::{conn::Conn, Error as UtilError, Result as UtilResult};

use super::srtp::{default_config, SrtpContexts};

pub(super) struct DtlsIdentity {
    certificate: Certificate,
    fingerprint: String,
}

impl DtlsIdentity {
    pub(super) fn generate() -> Result<Self, String> {
        // media-edge 同时依赖 HTTP 客户端和 DTLS，可能激活多个 rustls provider；
        // WebRTC DTLS 固定选择纯 Rust ring provider，避免启动顺序导致握手 panic。
        let _ = rustls::crypto::ring::default_provider().install_default();
        let certificate = Certificate::generate_self_signed(vec!["vos-rs".to_string()])
            .map_err(|error| error.to_string())?;
        let der = certificate
            .certificate
            .first()
            .ok_or_else(|| "DTLS 证书链为空".to_string())?;
        let digest = Sha256::digest(der.as_ref());
        let fingerprint = digest
            .iter()
            .map(|byte| format!("{byte:02X}"))
            .collect::<Vec<_>>()
            .join(":");
        Ok(Self {
            certificate,
            fingerprint,
        })
    }

    pub(super) fn fingerprint(&self) -> &str {
        &self.fingerprint
    }
}

pub(super) struct DtlsTransport {
    inbound_tx: mpsc::Sender<Vec<u8>>,
    peer: Arc<RwLock<Option<SocketAddr>>>,
}

impl DtlsTransport {
    pub(super) fn start(
        socket: Arc<UdpSocket>,
        identity: DtlsIdentity,
        crypto: Arc<RwLock<Option<Arc<SrtpContexts>>>>,
        dtls_connected: Arc<std::sync::atomic::AtomicBool>,
        dtls_failed: Arc<std::sync::atomic::AtomicBool>,
    ) -> Self {
        let (inbound_tx, inbound_rx) = mpsc::channel(128);
        let peer = Arc::new(RwLock::new(None));
        let connection = Arc::new(ChannelConn {
            socket,
            inbound_rx: Mutex::new(inbound_rx),
            peer: Arc::clone(&peer),
        });
        tokio::spawn(run_handshake(connection, identity, crypto, dtls_connected, dtls_failed));
        Self { inbound_tx, peer }
    }

    pub(super) fn push_packet(&self, packet: &[u8]) -> Result<(), String> {
        self.inbound_tx
            .try_send(packet.to_vec())
            .map_err(|error| format!("DTLS 输入队列不可用: {error}"))
    }

    pub(super) async fn set_peer(&self, peer: SocketAddr) {
        *self.peer.write().await = Some(peer);
    }
}

async fn run_handshake(
    connection: Arc<ChannelConn>,
    identity: DtlsIdentity,
    crypto: Arc<RwLock<Option<Arc<SrtpContexts>>>>,
    dtls_connected: Arc<std::sync::atomic::AtomicBool>,
    dtls_failed: Arc<std::sync::atomic::AtomicBool>,
) {
    let config = Config {
        certificates: vec![identity.certificate],
        srtp_protection_profiles: vec![SrtpProtectionProfile::Srtp_Aes128_Cm_Hmac_Sha1_80],
        extended_master_secret: ExtendedMasterSecretType::Require,
        ..Default::default()
    };
    let connection: Arc<dyn Conn + Send + Sync> = connection;
    match DTLSConn::new(connection, config, false, None).await {
        Ok(dtls) => {
            let mut srtp_config = default_config();
            let state = dtls.connection_state().await;
            let contexts = match srtp_config
                .extract_session_keys_from_dtls(state, false)
                .await
            {
                Ok(()) => SrtpContexts::from_config(srtp_config),
                Err(error) => Err(error.to_string()),
            };
            match contexts {
                Ok(contexts) => {
                    *crypto.write().await = Some(Arc::new(contexts));
                    dtls_connected.store(true, std::sync::atomic::Ordering::Release);
                    tracing::info!("WebRTC DTLS 握手完成，SRTP 密钥已安装");
                }
                Err(error) => {
                    dtls_failed.store(true, std::sync::atomic::Ordering::Release);
                    tracing::warn!(%error, "WebRTC SRTP 密钥派生失败");
                }
            }
        }
        Err(error) => {
            dtls_failed.store(true, std::sync::atomic::Ordering::Release);
            tracing::warn!(%error, "WebRTC DTLS 握手失败");
        }
    }
}

struct ChannelConn {
    socket: Arc<UdpSocket>,
    inbound_rx: Mutex<mpsc::Receiver<Vec<u8>>>,
    peer: Arc<RwLock<Option<SocketAddr>>>,
}

#[async_trait]
impl Conn for ChannelConn {
    async fn connect(&self, address: SocketAddr) -> UtilResult<()> {
        *self.peer.write().await = Some(address);
        Ok(())
    }

    async fn recv(&self, buffer: &mut [u8]) -> UtilResult<usize> {
        let packet = self
            .inbound_rx
            .lock()
            .await
            .recv()
            .await
            .ok_or(UtilError::ErrBufferClosed)?;
        if packet.len() > buffer.len() {
            return Err(UtilError::ErrBufferShort);
        }
        buffer[..packet.len()].copy_from_slice(&packet);
        Ok(packet.len())
    }

    async fn recv_from(&self, buffer: &mut [u8]) -> UtilResult<(usize, SocketAddr)> {
        let size = self.recv(buffer).await?;
        let peer = (*self.peer.read().await).ok_or(UtilError::ErrNoRemAddr)?;
        Ok((size, peer))
    }

    async fn send(&self, packet: &[u8]) -> UtilResult<usize> {
        let peer = (*self.peer.read().await).ok_or(UtilError::ErrNoRemAddr)?;
        self.socket
            .send_to(packet, peer)
            .await
            .map_err(UtilError::from_std)
    }

    async fn send_to(&self, packet: &[u8], target: SocketAddr) -> UtilResult<usize> {
        self.socket
            .send_to(packet, target)
            .await
            .map_err(UtilError::from_std)
    }

    fn local_addr(&self) -> UtilResult<SocketAddr> {
        self.socket.local_addr().map_err(UtilError::from_std)
    }

    fn remote_addr(&self) -> Option<SocketAddr> {
        self.peer.try_read().ok().and_then(|value| *value)
    }

    async fn close(&self) -> UtilResult<()> {
        Ok(())
    }

    fn as_any(&self) -> &(dyn std::any::Any + Send + Sync) {
        self
    }
}
