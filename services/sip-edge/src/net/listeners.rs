use crate::config::EdgeConfig;
use crate::edge_state::EdgeState;
use crate::net::{handle_stream_connection, handle_ws_connection, SipStream};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing::{debug, error, warn};

pub(crate) fn start_tcp_listener(
    l: Arc<TcpListener>,
    edge_state: Arc<EdgeState>,
    edge_config: Arc<EdgeConfig>,
) {
    tokio::spawn(async move {
        loop {
            match l.accept().await {
                Ok((stream, peer)) => {
                    debug!(%peer, "accepted TCP connection");
                    let (tx, rx) = tokio::sync::mpsc::channel(100);
                    edge_state.register_tcp_connection(peer, tx.clone());

                    let state_clone = Arc::clone(&edge_state);
                    let config_clone = Arc::clone(&edge_config);
                    tokio::spawn(handle_stream_connection(
                        SipStream::Tcp(stream),
                        peer,
                        tx,
                        rx,
                        move |msg_bytes, peer_addr, connection_tx| {
                            let state = Arc::clone(&state_clone);
                            let config = Arc::clone(&config_clone);
                            async move {
                                let datagrams = crate::sip::handle_datagram(
                                    &msg_bytes, peer_addr, &state, &config,
                                )
                                .await;
                                for datagram in datagrams {
                                    let _ = connection_tx.send(datagram.bytes).await;
                                }
                            }
                        },
                    ));
                }
                Err(e) => {
                    error!(error = %e, "TCP accept error");
                }
            }
        }
    });
}

pub(crate) fn start_tls_listener(
    l: Arc<TcpListener>,
    acceptor: tokio_rustls::TlsAcceptor,
    edge_state: Arc<EdgeState>,
    edge_config: Arc<EdgeConfig>,
) {
    tokio::spawn(async move {
        loop {
            match l.accept().await {
                Ok((stream, peer)) => {
                    let acceptor_clone = acceptor.clone();
                    let edge_state_inner = Arc::clone(&edge_state);
                    let edge_config_inner = Arc::clone(&edge_config);
                    tokio::spawn(async move {
                        match acceptor_clone.accept(stream).await {
                            Ok(tls_stream) => {
                                debug!(%peer, "accepted TLS handshake");
                                let (tx, rx) = tokio::sync::mpsc::channel(100);
                                edge_state_inner.register_tcp_connection(peer, tx.clone());

                                let state_clone = Arc::clone(&edge_state_inner);
                                let config_clone = Arc::clone(&edge_config_inner);
                                handle_stream_connection(
                                    SipStream::TlsServer(tls_stream),
                                    peer,
                                    tx,
                                    rx,
                                    move |msg_bytes, peer_addr, connection_tx| {
                                        let state = Arc::clone(&state_clone);
                                        let config = Arc::clone(&config_clone);
                                        async move {
                                            let datagrams = crate::sip::handle_datagram(
                                                &msg_bytes, peer_addr, &state, &config,
                                            )
                                            .await;
                                            for datagram in datagrams {
                                                let _ = connection_tx.send(datagram.bytes).await;
                                            }
                                        }
                                    },
                                )
                                .await;
                            }
                            Err(e) => {
                                warn!(%peer, error = %e, "TLS handshake accept failed");
                            }
                        }
                    });
                }
                Err(e) => {
                    error!(error = %e, "TLS accept error");
                }
            }
        }
    });
}

pub(crate) fn start_ws_listener(
    ws_listener: TcpListener,
    acceptor: Option<tokio_rustls::TlsAcceptor>,
    edge_state: Arc<EdgeState>,
    edge_config: Arc<EdgeConfig>,
) {
    tokio::spawn(async move {
        loop {
            match ws_listener.accept().await {
                Ok((stream, peer)) => {
                    debug!(%peer, "accepted WebSocket TCP connection");
                    let state_clone = Arc::clone(&edge_state);
                    let config_clone = Arc::clone(&edge_config);
                    let acceptor_clone = acceptor.clone();
                    tokio::spawn(async move {
                        if let Some(acc) = acceptor_clone {
                            match acc.accept(stream).await {
                                Ok(tls_stream) => {
                                    match tokio_tungstenite::accept_async(tls_stream).await {
                                        Ok(ws_stream) => {
                                            setup_ws_connection(
                                                ws_stream,
                                                peer,
                                                state_clone,
                                                config_clone,
                                            )
                                            .await;
                                        }
                                        Err(e) => {
                                            warn!(%peer, error = %e, "WSS handshake failed");
                                        }
                                    }
                                }
                                Err(e) => {
                                    warn!(%peer, error = %e, "WSS TLS accept failed");
                                }
                            }
                        } else {
                            match tokio_tungstenite::accept_async(stream).await {
                                Ok(ws_stream) => {
                                    setup_ws_connection(ws_stream, peer, state_clone, config_clone)
                                        .await;
                                }
                                Err(e) => {
                                    warn!(%peer, error = %e, "WS handshake failed");
                                }
                            }
                        }
                    });
                }
                Err(e) => {
                    error!(error = %e, "WebSocket accept error");
                }
            }
        }
    });
}

async fn setup_ws_connection<S>(
    ws_stream: tokio_tungstenite::WebSocketStream<S>,
    peer: SocketAddr,
    state_clone: Arc<EdgeState>,
    config_clone: Arc<EdgeConfig>,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    debug!(%peer, "WebSocket handshake succeeded");
    let (tx, rx) = tokio::sync::mpsc::channel(100);
    state_clone.register_tcp_connection(peer, tx.clone());

    let on_msg_state = Arc::clone(&state_clone);
    let on_msg_config = Arc::clone(&config_clone);

    handle_ws_connection(
        ws_stream,
        peer,
        tx,
        rx,
        move |msg_bytes, peer_addr, connection_tx| {
            let state = Arc::clone(&on_msg_state);
            let config = Arc::clone(&on_msg_config);
            async move {
                let datagrams =
                    crate::sip::handle_datagram(&msg_bytes, peer_addr, &state, &config).await;
                for d in datagrams {
                    let _ = connection_tx.send(d.bytes).await;
                }
            }
        },
    )
    .await;
}
