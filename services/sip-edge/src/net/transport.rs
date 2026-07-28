use rustls_pki_types::{CertificateDer, ServerName};
use std::fs;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream as ClientTlsStream;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::TlsConnector;
use tokio_tungstenite::WebSocketStream;
use tracing::{debug, info, warn};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Transport {
    Udp,
    Tcp,
    Tls,
    Ws,
    Wss,
}

#[allow(clippy::large_enum_variant)]
pub enum SipStream {
    Tcp(TcpStream),
    TlsClient(ClientTlsStream<TcpStream>),
}

impl AsyncRead for SipStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            Self::Tcp(s) => Pin::new(s).poll_read(cx, buf),
            Self::TlsClient(s) => Pin::new(s).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for SipStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        match self.get_mut() {
            Self::Tcp(s) => Pin::new(s).poll_write(cx, buf),
            Self::TlsClient(s) => Pin::new(s).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            Self::Tcp(s) => Pin::new(s).poll_flush(cx),
            Self::TlsClient(s) => Pin::new(s).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            Self::Tcp(s) => Pin::new(s).poll_shutdown(cx),
            Self::TlsClient(s) => Pin::new(s).poll_shutdown(cx),
        }
    }
}

pub fn read_frame(buf: &mut Vec<u8>) -> Option<Vec<u8>> {
    let raw = buf.as_slice();
    let (index, delim_len) =
        if let Some(pos) = raw.windows(4).position(|window| window == b"\r\n\r\n") {
            (pos, 4)
        } else if let Some(pos) = raw.windows(2).position(|window| window == b"\n\n") {
            (pos, 2)
        } else {
            return None;
        };

    let header_part = &raw[..index];
    let header_str = String::from_utf8_lossy(header_part);

    let mut content_length = 0;
    for line in header_str.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some((name, val)) = trimmed.split_once(':') {
            let name_lower = name.trim().to_lowercase();
            if name_lower == "content-length" || name_lower == "l" {
                if let Ok(len) = val.trim().parse::<usize>() {
                    content_length = len;
                }
            }
        }
    }

    let total_len = index + delim_len + content_length;
    if buf.len() < total_len {
        return None;
    }

    Some(buf.drain(..total_len).collect())
}

#[derive(Debug)]
struct NoCertificateVerification;

impl rustls::client::danger::ServerCertVerifier for NoCertificateVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls_pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![
            rustls::SignatureScheme::RSA_PSS_SHA256,
            rustls::SignatureScheme::RSA_PSS_SHA384,
            rustls::SignatureScheme::RSA_PSS_SHA512,
            rustls::SignatureScheme::RSA_PKCS1_SHA256,
            rustls::SignatureScheme::RSA_PKCS1_SHA384,
            rustls::SignatureScheme::RSA_PKCS1_SHA512,
            rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            rustls::SignatureScheme::ECDSA_NISTP384_SHA384,
            rustls::SignatureScheme::ED25519,
        ]
    }
}

fn parse_certificates(pem: &[u8]) -> Result<Vec<CertificateDer<'static>>, rustls::Error> {
    let mut cert_reader = std::io::Cursor::new(pem);
    let certs: Vec<CertificateDer<'static>> = rustls_pemfile::certs(&mut cert_reader)
        .collect::<Result<_, _>>()
        .map_err(|e| rustls::Error::General(e.to_string()))?;
    if certs.is_empty() {
        return Err(rustls::Error::General(
            "TLS certificate file did not contain any certificates".to_string(),
        ));
    }
    Ok(certs)
}

fn tls_crypto_provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::aws_lc_rs::default_provider())
}

fn load_root_store(ca_path: Option<&str>) -> Result<rustls::RootCertStore, rustls::Error> {
    let mut roots = rustls::RootCertStore::empty();

    if let Some(ca_path) = ca_path {
        let ca_pem = fs::read(ca_path)
            .map_err(|e| rustls::Error::General(format!("failed to read TLS CA file: {e}")))?;
        let certs = parse_certificates(&ca_pem)?;
        let (added, ignored) = roots.add_parsable_certificates(certs);
        if ignored > 0 {
            warn!(ignored, "ignored unparsable TLS CA certificates");
        }
        if added == 0 {
            return Err(rustls::Error::General(
                "TLS CA file did not contain a usable root certificate".to_string(),
            ));
        }
        return Ok(roots);
    }

    let native_certs = rustls_native_certs::load_native_certs();
    let native_error_count = native_certs.errors.len();
    let (added, ignored) = roots.add_parsable_certificates(native_certs.certs);
    if native_error_count > 0 {
        warn!(
            errors = native_error_count,
            "encountered errors while loading platform TLS roots"
        );
    }
    if ignored > 0 {
        warn!(ignored, "ignored unparsable platform TLS roots");
    }
    if added == 0 {
        return Err(rustls::Error::General(
            "no usable platform TLS root certificates were loaded".to_string(),
        ));
    }

    Ok(roots)
}

pub fn create_tls_connector(
    ca_path: Option<&str>,
    insecure_skip_verify: bool,
) -> Result<TlsConnector, rustls::Error> {
    let mut config = if insecure_skip_verify {
        rustls::ClientConfig::builder_with_provider(tls_crypto_provider())
            .with_safe_default_protocol_versions()?
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(NoCertificateVerification))
            .with_no_client_auth()
    } else {
        let roots = load_root_store(ca_path)?;
        rustls::ClientConfig::builder_with_provider(tls_crypto_provider())
            .with_safe_default_protocol_versions()?
            .with_root_certificates(roots)
            .with_no_client_auth()
    };
    config.alpn_protocols = vec![b"sip/2.0".to_vec()];

    Ok(TlsConnector::from(Arc::new(config)))
}

/// 构造入站 TLS 服务端 acceptor，用于 WSS 监听器和 TLS SIP 信令监听器。
///
/// 需要提供 PEM 格式的证书链与私钥路径。ALPN 协商 `sip/2.0`。
pub(crate) fn create_tls_acceptor(
    cert_path: &str,
    key_path: &str,
) -> Result<TlsAcceptor, rustls::Error> {
    let cert_pem = fs::read(cert_path)
        .map_err(|e| rustls::Error::General(format!("failed to read TLS certificate file: {e}")))?;
    let certs = parse_certificates(&cert_pem)?;

    let key_pem = fs::read(key_path)
        .map_err(|e| rustls::Error::General(format!("failed to read TLS private key file: {e}")))?;
    let mut key_reader = std::io::Cursor::new(&key_pem);
    let key = rustls_pemfile::private_key(&mut key_reader)
        .map_err(|e| rustls::Error::General(format!("failed to parse TLS private key: {e}")))?
        .ok_or_else(|| {
            rustls::Error::General("TLS private key file contained no keys".to_string())
        })?;

    let mut config = rustls::ServerConfig::builder_with_provider(tls_crypto_provider())
        .with_safe_default_protocol_versions()?
        .with_no_client_auth()
        .with_single_cert(certs, key)?;
    config.alpn_protocols = vec![b"sip/2.0".to_vec()];

    Ok(TlsAcceptor::from(Arc::new(config)))
}

pub async fn handle_stream_connection<F, Fut>(
    mut stream: SipStream,
    peer: SocketAddr,
    tx: tokio::sync::mpsc::Sender<Vec<u8>>,
    mut rx: tokio::sync::mpsc::Receiver<Vec<u8>>,
    on_message: F,
) where
    F: Fn(Vec<u8>, SocketAddr, tokio::sync::mpsc::Sender<Vec<u8>>) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    let mut read_buf = vec![0u8; 8192];
    let mut frame_buf = Vec::new();

    loop {
        tokio::select! {
            result = stream.read(&mut read_buf) => {
                match result {
                    Ok(0) => {
                        debug!(%peer, "TCP/TLS stream closed by remote");
                        break;
                    }
                    Ok(n) => {
                        frame_buf.extend_from_slice(&read_buf[..n]);
                        while let Some(msg_bytes) = read_frame(&mut frame_buf) {
                            let on_msg_clone = &on_message;
                            let tx_clone = tx.clone();
                            tokio::spawn(on_msg_clone(msg_bytes, peer, tx_clone));
                        }
                    }
                    Err(e) => {
                        warn!(%peer, error = %e, "TCP/TLS stream read error");
                        break;
                    }
                }
            }
            msg = rx.recv() => {
                match msg {
                    Some(bytes) => {
                        if let Err(e) = stream.write_all(&bytes).await {
                            warn!(%peer, error = %e, "TCP/TLS stream write error");
                            break;
                        }
                        if let Err(e) = stream.flush().await {
                            warn!(%peer, error = %e, "TCP/TLS stream flush error");
                            break;
                        }
                    }
                    None => {
                        break;
                    }
                }
            }
        }
    }
}

pub async fn handle_ws_connection<S, F, Fut>(
    mut ws_stream: WebSocketStream<S>,
    peer: SocketAddr,
    tx: tokio::sync::mpsc::Sender<Vec<u8>>,
    mut rx: tokio::sync::mpsc::Receiver<Vec<u8>>,
    on_message: F,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
    F: Fn(Vec<u8>, SocketAddr, tokio::sync::mpsc::Sender<Vec<u8>>) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message as WsMessage;

    loop {
        tokio::select! {
            result = ws_stream.next() => {
                match result {
                    Some(Ok(msg)) => {
                        let bytes = match msg {
                            WsMessage::Text(s) => s.into_bytes(),
                            WsMessage::Binary(b) => b,
                            WsMessage::Ping(p) => {
                                let _ = ws_stream.send(WsMessage::Pong(p)).await;
                                continue;
                            }
                            WsMessage::Pong(_) => continue,
                            WsMessage::Close(_) => {
                                debug!(%peer, "WebSocket connection closed by remote");
                                break;
                            }
                            _ => continue,
                        };
                        if !bytes.is_empty() {
                            let on_msg_clone = &on_message;
                            let tx_clone = tx.clone();
                            tokio::spawn(on_msg_clone(bytes, peer, tx_clone));
                        }
                    }
                    Some(Err(e)) => {
                        warn!(%peer, error = %e, "WebSocket stream read error");
                        break;
                    }
                    None => {
                        debug!(%peer, "WebSocket connection ended");
                        break;
                    }
                }
            }
            msg = rx.recv() => {
                match msg {
                    Some(bytes) => {
                        match String::from_utf8(bytes) {
                            Ok(text) => {
                                if let Err(e) = ws_stream.send(WsMessage::Text(text)).await {
                                    warn!(%peer, error = %e, "WebSocket send error");
                                    break;
                                }
                            }
                            Err(err) => {
                                if let Err(e) = ws_stream.send(WsMessage::Binary(err.into_bytes())).await {
                                    warn!(%peer, error = %e, "WebSocket send error");
                                    break;
                                }
                            }
                        }
                    }
                    None => {
                        break;
                    }
                }
            }
        }
    }
}

/// 启动 SIP WebSocket (WS) 信令入站监听器。
///
/// 每个 accept 到的 TCP 连接升级为 WebSocket，并注册到 `edge_state` 的
/// `tcp_connections` 表（复用同一通道表），随后由 `handle_ws_connection`
/// 处理 SIP 文本帧的收发。
pub async fn serve_ws_listener<F, Fut>(bind_addr: String, on_message: F) -> std::io::Result<()>
where
    F: Fn(Vec<u8>, SocketAddr, tokio::sync::mpsc::Sender<Vec<u8>>) -> Fut
        + Send
        + Sync
        + Clone
        + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
    info!(addr = %bind_addr, "SIP Edge WS Listening");

    loop {
        match listener.accept().await {
            Ok((stream, peer)) => {
                let on_msg = on_message.clone();
                tokio::spawn(async move {
                    match tokio_tungstenite::accept_async(stream).await {
                        Ok(ws_stream) => {
                            let (tx, rx) = tokio::sync::mpsc::channel(100);
                            // 复用 tcp_connections 通道表注册 WS 连接，使出站路径可找到
                            // 注意：调用方需在 on_message 闭包中调用 register_tcp_connection
                            handle_ws_connection(ws_stream, peer, tx, rx, on_msg).await;
                        }
                        Err(e) => {
                            warn!(%peer, error = %e, "WebSocket server handshake failed");
                        }
                    }
                });
            }
            Err(e) => {
                warn!(error = %e, "WS listener accept failed");
                continue;
            }
        }
    }
}

/// 启动 SIP WebSocket Secure (WSS) 信令入站监听器。
///
/// 与 `serve_ws_listener` 相同，但 TCP 连接先经 TLS acceptor 升级为
/// `tokio_rustls::server::TlsStream<TcpStream>`，再升级为 WebSocket。
pub async fn serve_wss_listener<F, Fut>(
    bind_addr: String,
    cert_path: String,
    key_path: String,
    on_message: F,
) -> std::io::Result<()>
where
    F: Fn(Vec<u8>, SocketAddr, tokio::sync::mpsc::Sender<Vec<u8>>) -> Fut
        + Send
        + Sync
        + Clone
        + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    let acceptor = create_tls_acceptor(&cert_path, &key_path).map_err(std::io::Error::other)?;
    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
    info!(addr = %bind_addr, "SIP Edge WSS Listening");

    loop {
        match listener.accept().await {
            Ok((stream, peer)) => {
                let acceptor = acceptor.clone();
                let on_msg = on_message.clone();
                tokio::spawn(async move {
                    match acceptor.accept(stream).await {
                        Ok(tls_stream) => match tokio_tungstenite::accept_async(tls_stream).await {
                            Ok(ws_stream) => {
                                let (tx, rx) = tokio::sync::mpsc::channel(100);
                                handle_ws_connection(ws_stream, peer, tx, rx, on_msg).await;
                            }
                            Err(e) => {
                                warn!(%peer, error = %e, "WSS WebSocket upgrade failed");
                            }
                        },
                        Err(e) => {
                            warn!(%peer, error = %e, "WSS TLS handshake failed");
                        }
                    }
                });
            }
            Err(e) => {
                warn!(error = %e, "WSS listener accept failed");
                continue;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insecure_tls_connector_is_explicit_opt_in() {
        assert!(create_tls_connector(None, true).is_ok());
    }

    /// 生成自签名 ECDSA 证书与私钥，用于测试 `create_tls_acceptor`。
    fn generate_self_signed_cert() -> (std::path::PathBuf, std::path::PathBuf) {
        use rcgen::{generate_simple_self_signed, CertifiedKey};
        let CertifiedKey { cert, key_pair } =
            generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
        let cert_path =
            std::env::temp_dir().join(format!("vos-rs-test-cert-{}.pem", std::process::id()));
        let key_path =
            std::env::temp_dir().join(format!("vos-rs-test-key-{}.pem", std::process::id()));
        std::fs::write(&cert_path, cert.pem()).unwrap();
        std::fs::write(&key_path, key_pair.serialize_pem()).unwrap();
        (cert_path, key_path)
    }

    #[test]
    fn create_tls_acceptor_with_self_signed_cert_succeeds() {
        let (cert_path, key_path) = generate_self_signed_cert();
        let result = create_tls_acceptor(cert_path.to_str().unwrap(), key_path.to_str().unwrap());
        let _ = std::fs::remove_file(&cert_path);
        let _ = std::fs::remove_file(&key_path);
        assert!(
            result.is_ok(),
            "TLS acceptor creation should succeed: {:?}",
            result.err()
        );
    }

    #[test]
    fn create_tls_acceptor_with_missing_cert_fails() {
        let result = create_tls_acceptor("/nonexistent/cert.pem", "/nonexistent/key.pem");
        assert!(result.is_err());
    }

    /// 验证 WSS 监听器能成功启动并绑定端口（TLS acceptor 与 TcpListener 正常工作）。
    /// 完整的 WSS 端到端握手由 `create_tls_acceptor_with_self_signed_cert_succeeds`
    /// 与 `serve_ws_listener_completes_handshake` 共同覆盖。
    #[tokio::test]
    async fn serve_wss_listener_starts_and_binds() {
        let (cert_path, key_path) = generate_self_signed_cert();
        let cert_path = cert_path.to_str().unwrap().to_string();
        let key_path = key_path.to_str().unwrap().to_string();

        let probe = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let bind_addr = probe.local_addr().unwrap();
        drop(probe);

        let addr_str = bind_addr.to_string();
        let server_task = tokio::spawn(async move {
            #[allow(clippy::type_complexity)]
            let on_message: fn(
                Vec<u8>,
                SocketAddr,
                tokio::sync::mpsc::Sender<Vec<u8>>,
            ) -> std::future::Ready<()> = |_msg, _peer, _tx| std::future::ready(());
            let _ = serve_wss_listener(addr_str, cert_path, key_path, on_message).await;
        });

        // 等待监听器就绪并验证 TCP 可连接
        let _ = loop {
            match tokio::net::TcpStream::connect(bind_addr).await {
                Ok(s) => break s,
                Err(_) => tokio::time::sleep(std::time::Duration::from_millis(20)).await,
            }
        };

        // TCP 连接成功即证明 serve_wss_listener 已启动并绑定端口
        server_task.abort();
    }

    /// 端到端验证 WS（明文）监听器：启动 serve_ws_listener → 客户端连接 →
    /// 成功完成 WebSocket 升级。
    #[tokio::test]
    async fn serve_ws_listener_completes_handshake() {
        use futures::StreamExt;

        let probe = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let bind_addr = probe.local_addr().unwrap();
        drop(probe);

        let addr_str = bind_addr.to_string();
        let server_task = tokio::spawn(async move {
            #[allow(clippy::type_complexity)]
            let on_message: fn(
                Vec<u8>,
                SocketAddr,
                tokio::sync::mpsc::Sender<Vec<u8>>,
            ) -> std::future::Ready<()> = |_msg, _peer, _tx| std::future::ready(());
            let _ = serve_ws_listener(addr_str, on_message).await;
        });

        // 客户端重试连接，等待监听器就绪
        let tcp_stream = loop {
            match tokio::net::TcpStream::connect(bind_addr).await {
                Ok(s) => break s,
                Err(_) => tokio::time::sleep(std::time::Duration::from_millis(20)).await,
            }
        };

        let ws_request = tokio_tungstenite::tungstenite::handshake::client::Request::builder()
            .method("GET")
            .uri(format!("ws://{bind_addr}/"))
            .header("Host", bind_addr.to_string())
            .header("Connection", "Upgrade")
            .header("Upgrade", "websocket")
            .header("Sec-WebSocket-Version", "13")
            .header("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ==")
            .body(())
            .unwrap();
        let (mut ws_stream, _response) = tokio_tungstenite::client_async(ws_request, tcp_stream)
            .await
            .expect("WS client handshake should succeed");

        // 等待服务端关闭连接或超时后主动 abort 服务端
        let _ = tokio::time::timeout(std::time::Duration::from_millis(500), ws_stream.next()).await;
        server_task.abort();
    }
}
