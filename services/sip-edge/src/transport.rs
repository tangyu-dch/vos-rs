use rustls_pki_types::{CertificateDer, PrivateKeyDer, ServerName};
use std::fs;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream as ClientTlsStream;
use tokio_rustls::server::TlsStream as ServerTlsStream;
use tokio_rustls::{TlsAcceptor, TlsConnector};
use tokio_tungstenite::WebSocketStream;
use tracing::{debug, warn};

const TEST_CERT_PEM: &str = concat!(
    "-----BEGIN CERTIFICATE-----\n",
    "MIIDCTCCAfGgAwIBAgIUL/JT3Lyi5KmFFEnLwO23akuY8+QwDQYJKoZIhvcNAQEL\n",
    "BQAwFDESMBAGA1UEAwwJbG9jYWxob3N0MB4XDTI2MDcwMzEzMTU0MVoXDTI3MDcw\n",
    "MzEzMTU0MVowFDESMBAGA1UEAwwJbG9jYWxob3N0MIIBIjANBgkqhkiG9w0BAQEF\n",
    "AAOCAQ8AMIIBCgKCAQEAsEEie0EUX6Izyuu/tw+3+K2n0QG6z6Yac5arhBPgfs6d\n",
    "/Oso10Z9N31DR1vkb3Pg/UCep8eAGtxgi9BDC+wVDWRIVghvWYzT//N0fq8ctu/+\n",
    "R/olIrBIesNY31CcLxM2iZrgv8SnHCTyMEUPzPqgZ+YbtC3g1TgBMUJPeHKRVfFf\n",
    "fdtH3LPq7Xm8JaQnzILqWzpCt1YXZUq5m49prjAILPt+nYiV8De5+oPH+na7BOQs\n",
    "asZ2kAy5g00/canuqNjHZ6PQ52NVOBavlXaPpx9HMoCKHJtaF21DXKDPAE28qatC\n",
    "ljNAuTI5YrAEeqnL60WWB3bPMFLteqoVagkX/KNMMwIDAQABo1MwUTAdBgNVHQ4E\n",
    "FgQUTEoPM++sg+dxmPm9E7Al94/DVR8wHwYDVR0jBBgwFoAUTEoPM++sg+dxmPm9\n",
    "E7Al94/DVR8wDwYDVR0TAQH/BAUwAwEB/zANBgkqhkiG9w0BAQsFAAOCAQEAXdXC\n",
    "ZoOC2TxaS1soaYifIKULRuezHgWWtVl2qWrA8MRfJaSCcXlpksTVBxTHBfbLYUCj\n",
    "qXhjkfxwk4SomJaPmjP1RAAHjdCpRtvPPjyzJ8d5lCCZD1b1/GW8ZfC7c4hEBz1t\n",
    "w7PUak8JXOjMV/HEWOe5l+E5ZbOxpHtQGuMfvMiF9USy/c4THQRZLzTBYCEqhGq0\n",
    "h1XpHapWIRCYJWKbjtA+rfMDu6dFTG/YX2n39NZLcxAmsc8AGI1vMuFEuKEQfnb/\n",
    "uZcr8g0jexaR0SsSw5h0F+gpLxS70wNKz4NpVrvfeyVt7tMV/e8QbsqsEGEl7l8M\n",
    "8DBSi1ezSpceKJC8uw==\n",
    "-----END CERTIFICATE-----\n"
);

const TEST_KEY_PEM: &str = concat!(
    "-----BEGIN PRIVATE KEY-----\n",
    "MIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQCwQSJ7QRRfojPK\n",
    "67+3D7f4rafRAbrPphpzlquEE+B+zp386yjXRn03fUNHW+Rvc+D9QJ6nx4Aa3GCL\n",
    "0EML7BUNZEhWCG9ZjNP/83R+rxy27/5H+iUisEh6w1jfUJwvEzaJmuC/xKccJPIw\n",
    "RQ/M+qBn5hu0LeDVOAExQk94cpFV8V9920fcs+rtebwlpCfMgupbOkK3VhdlSrmb\n",
    "j2muMAgs+36diJXwN7n6g8f6drsE5CxqxnaQDLmDTT9xqe6o2Mdno9DnY1U4Fq+V\n",
    "do+nH0cygIocm1oXbUNcoM8ATbypq0KWM0C5MjlisAR6qcvrRZYHds8wUu16qhVq\n",
    "CRf8o0wzAgMBAAECggEAHBwkl9wPF++Ca3Rs4jvKCgmxcDaNPjfOE9hC/jC+BvYE\n",
    "oW/kgWzsZIYATb2x8YqIJqvV1zQMp3wKk9HWbOzX210T4lC8aDT7IguIbU9Co58k\n",
    "Aq5k73gw9GSlDxHypC61bdMl0WqCLQ1sE4G4MudX33+GCexKkDGQkx7HFj0ja71s\n",
    "HnDdxzjYU2HtwRgaCvGvXVk7IV9bAeoit76F9RC+P+QjnIiM8febk6d2lWVaN3nF\n",
    "o7K0ZPi0aqqCnnyK5q53ba0ApT5n9S2JvT1aox/SrUFo2YudJqRV9axXhPRJ5yKp\n",
    "Xd01dle2/jwxDqsZTn08SyO8T6y5zPN5gY0p2S7VdQKBgQDkokYqL+z7zZmPfIBC\n",
    "em1vVf8Hwyx+qlUfHWzpq3zzVr7FOdwBXuXAQEXdcttE/XwLqI2ZmqbeIHsDf1Ut\n",
    "z15zmj+EpMktNHOnWlDIkkZv6kchDSaSC460xUbZw2bMmOqYLLyjMNrYvKuPuVmb\n",
    "qXBtFS9o7Ru2MD89nfZJOyEb5wKBgQDFWd3HpqFqGI71PNVtMRt4Ng0BVzkN6H/v\n",
    "Hgpj75QFb5mTRlYW7stu0NfpQnIuIqSJNTKtLnzPKMR+uspQ7fPn090UOjuNlSnU\n",
    "LXhk29d2RPXm/nBX7fwWon8ME/31zG3aMCcuoyb3zi0yQdvDjZZG0GOBfxIIaFxj\n",
    "q1XFzbWj1QKBgCZsdypD36n5xaLto4iIlretVizxyhqHecK+6TzkCx3CKFFxBd5d\n",
    "GnOS2ar70Inpp901uIZmDUEraEEQNzp5rT/0XlRmdUDZnc40SXtLyfapAsc1NJQ6\n",
    "yQLsXJngUvhzgomMiy9J2J2wJ40B82NLuI88jjkuEAgwV5B9aZSpEUllAoGBALou\n",
    "ofCs3zM8oAH0tlUhMw0h0Psm0oiwg6GO8bZ+W2MVeglbHfTcq8eL92X0bcvgmuFm\n",
    "8rw3U0AM8fOtPRlEpApd8gAXP/++bYviqeZdENRfEq4t9Ma/mkewXbODWN//UNO7\n",
    "AfwZp7W5KSJ0x2Ohu9hq2LVesCCGdEMDbRQDkg1RAoGBAKTzVfp1RCaIKpGixfmT\n",
    "klY2Kl++ZuccUpswzcQi2TDv/wFs/TQpIzz45F2tnKWzq1Ue4qhtB1rXg38fQnLP\n",
    "/rQvko4XzPKCqn13A6yLh1pmAypqrQ/JMw25OoI/XUbCzkV5bvHPGAzunpTDvKhV\n",
    "Echf3Nlt9LL4SZphNNusdnZx\n",
    "-----END PRIVATE KEY-----\n"
);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Transport {
    Udp,
    Tcp,
    Tls,
    Ws,
    Wss,
}

impl Transport {
    #[allow(dead_code)]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Udp => "udp",
            Self::Tcp => "tcp",
            Self::Tls => "tls",
            Self::Ws => "ws",
            Self::Wss => "wss",
        }
    }
}

pub enum SipStream {
    Tcp(TcpStream),
    TlsServer(ServerTlsStream<TcpStream>),
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
            Self::TlsServer(s) => Pin::new(s).poll_read(cx, buf),
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
            Self::TlsServer(s) => Pin::new(s).poll_write(cx, buf),
            Self::TlsClient(s) => Pin::new(s).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            Self::Tcp(s) => Pin::new(s).poll_flush(cx),
            Self::TlsServer(s) => Pin::new(s).poll_flush(cx),
            Self::TlsClient(s) => Pin::new(s).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            Self::Tcp(s) => Pin::new(s).poll_shutdown(cx),
            Self::TlsServer(s) => Pin::new(s).poll_shutdown(cx),
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

fn parse_private_key(pem: &[u8]) -> Result<PrivateKeyDer<'static>, rustls::Error> {
    let mut key_reader = std::io::Cursor::new(pem);
    rustls_pemfile::private_key(&mut key_reader)
        .map_err(|e| rustls::Error::General(e.to_string()))?
        .ok_or_else(|| rustls::Error::General("TLS private key file is empty".to_string()))
}

fn tls_crypto_provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::aws_lc_rs::default_provider())
}

pub fn create_tls_acceptor_from_pem(
    cert_pem: &[u8],
    key_pem: &[u8],
) -> Result<TlsAcceptor, rustls::Error> {
    let certs = parse_certificates(cert_pem)?;
    let key = parse_private_key(key_pem)?;
    let mut config = rustls::ServerConfig::builder_with_provider(tls_crypto_provider())
        .with_safe_default_protocol_versions()?
        .with_no_client_auth()
        .with_single_cert(certs, key)?;
    config.alpn_protocols = vec![b"sip/2.0".to_vec()];

    Ok(TlsAcceptor::from(Arc::new(config)))
}

pub fn create_test_tls_acceptor() -> Result<TlsAcceptor, rustls::Error> {
    create_tls_acceptor_from_pem(TEST_CERT_PEM.as_bytes(), TEST_KEY_PEM.as_bytes())
}

pub fn create_tls_acceptor(
    cert_path: Option<&str>,
    key_path: Option<&str>,
    allow_test_certificate: bool,
) -> Result<Option<TlsAcceptor>, rustls::Error> {
    match (cert_path, key_path) {
        (Some(cert_path), Some(key_path)) => {
            let cert_pem = fs::read(cert_path)
                .map_err(|e| rustls::Error::General(format!("failed to read TLS cert: {e}")))?;
            let key_pem = fs::read(key_path)
                .map_err(|e| rustls::Error::General(format!("failed to read TLS key: {e}")))?;
            create_tls_acceptor_from_pem(&cert_pem, &key_pem).map(Some)
        }
        (None, None) if allow_test_certificate => create_test_tls_acceptor().map(Some),
        (None, None) => Ok(None),
        _ => Err(rustls::Error::General(
            "both TLS cert and key paths must be configured".to_string(),
        )),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tls_acceptor_is_disabled_without_certificate_material() {
        let acceptor = create_tls_acceptor(None, None, false).unwrap();
        assert!(acceptor.is_none());
    }

    #[test]
    fn tls_acceptor_requires_cert_and_key_together() {
        assert!(create_tls_acceptor(Some("/tmp/cert.pem"), None, false).is_err());
        assert!(create_tls_acceptor(None, Some("/tmp/key.pem"), false).is_err());
    }

    #[test]
    fn test_tls_certificate_is_explicit_opt_in() {
        let acceptor = create_tls_acceptor(None, None, true).unwrap();
        assert!(acceptor.is_some());
    }

    #[test]
    fn insecure_tls_connector_is_explicit_opt_in() {
        assert!(create_tls_connector(None, true).is_ok());
    }
}
