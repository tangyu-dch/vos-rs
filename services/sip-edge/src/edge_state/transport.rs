//! # SIP 出站传输
//!
//! 本模块扩展 [`EdgeState`][super::EdgeState]，处理 SIP 出站数据报的发送：
//!
//! - UDP 直发（fallback socket）
//! - TCP/TLS 出站连接建立与复用
//! - WebSocket 已连接通道查找
//! - 跨节点流量转发（ClusterEgress + Redis flow table）
//! - Keepalive 探测
//!
//! ## INVITE 响应顺序保护
//!
//! [`send_sip_datagram`] 在真正写入 socket 前会获取
//! [`InviteResponseOrder`][super::models::InviteResponseOrder] 锁，确保
//! provisional 响应不会被乱序到 final 响应之后发送。

use std::net::SocketAddr;
use std::sync::Arc;

use rustls_pki_types::ServerName;
use sip_core::parse_message;
use tokio::net::{TcpStream, UdpSocket};
use tracing::{debug, error};

use crate::cluster::{flow_key, FlowRecord};
use crate::config::EdgeConfig;
use crate::handle_datagram;
use crate::net::{create_tls_connector, handle_stream_connection, SipStream, Transport};

use super::models::PendingDatagram;
use super::EdgeState;

impl EdgeState {
    /// 将 cluster egress 流量转发给真正的 flow owner 节点。
    ///
    /// 返回 `Ok(true)` 表示已通过集群通道转发；`Ok(false)` 表示本节点应自行处理。
    pub(crate) async fn forward_to_flow_owner(
        &self,
        target: SocketAddr,
        bytes: Vec<u8>,
    ) -> Result<bool, std::io::Error> {
        let Some(egress) = self.cluster_egress.get() else {
            return Ok(false);
        };
        let Some(mut redis) = self.redis_connection() else {
            return Ok(false);
        };
        let payload: Option<String> = redis::cmd("GET")
            .arg(flow_key(target))
            .query_async(&mut redis)
            .await
            .map_err(std::io::Error::other)?;
        let Some(flow) = payload
            .as_deref()
            .and_then(|value| serde_json::from_str::<FlowRecord>(value).ok())
        else {
            return Ok(false);
        };
        if flow.owner_node_id == egress.node_id {
            return Ok(false);
        }
        egress
            .publish(&flow.owner_node_id, target, bytes)
            .await
            .map_err(std::io::Error::other)?;
        Ok(true)
    }

    pub(crate) fn set_socket(&self, socket: Arc<UdpSocket>) {
        let _ = self.socket.set(socket);
    }

    pub(crate) fn get_socket(&self) -> Option<Arc<UdpSocket>> {
        self.socket.get().cloned()
    }

    pub(crate) fn register_tcp_connection(
        &self,
        addr: SocketAddr,
        tx: tokio::sync::mpsc::Sender<Vec<u8>>,
    ) {
        self.tcp_connections.insert(addr, tx);
    }

    pub(crate) fn get_tcp_connection(
        &self,
        addr: &SocketAddr,
    ) -> Option<tokio::sync::mpsc::Sender<Vec<u8>>> {
        if let Some(tx) = self.tcp_connections.get(addr) {
            if tx.is_closed() {
                drop(tx);
                self.tcp_connections.remove(addr);
                None
            } else {
                Some(tx.clone())
            }
        } else {
            None
        }
    }

    /// 发送 SIP 数据报。
    ///
    /// 依据 Via 头推断传输协议（UDP/TCP/TLS/WS/WSS），按需建立出站连接并复用
    /// 已注册的 TCP 通道。INVITE 响应会经过 [`InviteResponseOrder`] 顺序保护。
    ///
    /// [`InviteResponseOrder`]: super::models::InviteResponseOrder
    pub(crate) fn send_sip_datagram<'a>(
        self: &'a Arc<Self>,
        datagram: PendingDatagram,
        fallback_socket: &'a UdpSocket,
        edge_config: &'a EdgeConfig,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), std::io::Error>> + Send + 'a>>
    {
        Box::pin(async move {
            // The guard intentionally covers the actual socket/channel write. Merely serializing
            // response construction still allows a suspended provisional-response task to send
            // after a final response under high scheduler pressure.
            let _invite_response_guard = match datagram.invite_response.as_ref() {
                Some(metadata) => {
                    let mut order = metadata.order.lock().await;
                    if order.cseq != metadata.cseq {
                        if order
                            .cseq
                            .zip(metadata.cseq)
                            .is_some_and(|(current, pending)| pending < current)
                        {
                            debug!(
                                cseq = ?metadata.cseq,
                                current_cseq = ?order.cseq,
                                status = metadata.status_code,
                                "dropping response from an older INVITE transaction"
                            );
                            return Ok(());
                        }
                        order.cseq = metadata.cseq;
                        order.final_response_seen = metadata.status_code >= 200;
                        order.final_response_send_started = false;
                    }
                    if metadata.status_code < 200 && order.final_response_send_started {
                        debug!(
                            cseq = ?metadata.cseq,
                            status = metadata.status_code,
                            "dropping late provisional INVITE response before network send"
                        );
                        return Ok(());
                    }
                    if metadata.status_code >= 200 {
                        order.final_response_send_started = true;
                    }
                    Some(order)
                }
                None => None,
            };

            let target_addr: SocketAddr = match datagram.target.parse() {
                Ok(addr) => addr,
                Err(_) => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "invalid target address",
                    ));
                }
            };

            if self
                .sipflow_enabled
                .load(std::sync::atomic::Ordering::Relaxed)
            {
                self.capture_sip_packet(&datagram.bytes, "out", target_addr);
            }

            let mut transport = Transport::Udp;
            if let Ok(msg) = parse_message(&datagram.bytes) {
                if let Some(via) = msg.headers().get("via") {
                    let via_str = via.as_str().to_uppercase();
                    if via_str.contains("SIP/2.0/TLS") {
                        transport = Transport::Tls;
                    } else if via_str.contains("SIP/2.0/TCP") {
                        transport = Transport::Tcp;
                    } else if via_str.contains("SIP/2.0/WSS") {
                        transport = Transport::Wss;
                    } else if via_str.contains("SIP/2.0/WS") {
                        transport = Transport::Ws;
                    }
                }
            }

            if let Some(tx) = self.get_tcp_connection(&target_addr) {
                if tx.send(datagram.bytes.clone()).await.is_ok() {
                    return Ok(());
                }
            }

            if matches!(
                transport,
                Transport::Tcp | Transport::Tls | Transport::Ws | Transport::Wss
            ) && self
                .forward_to_flow_owner(target_addr, datagram.bytes.clone())
                .await?
            {
                return Ok(());
            }

            match transport {
                Transport::Tcp => {
                    debug!(%target_addr, "establishing new outbound TCP connection");
                    match TcpStream::connect(target_addr).await {
                        Ok(stream) => {
                            let (tx, rx) = tokio::sync::mpsc::channel(100);
                            self.register_tcp_connection(target_addr, tx.clone());

                            let state_clone = Arc::clone(self);
                            let config_clone = edge_config.clone();
                            tokio::spawn(handle_stream_connection(
                                SipStream::Tcp(stream),
                                target_addr,
                                tx.clone(),
                                rx,
                                move |msg_bytes, peer_addr, connection_tx| {
                                    let state = Arc::clone(&state_clone);
                                    let config = config_clone.clone();
                                    let fut: std::pin::Pin<
                                        Box<dyn std::future::Future<Output = ()> + Send>,
                                    > = Box::pin(async move {
                                        let datagrams =
                                            handle_datagram(&msg_bytes, peer_addr, &state, &config)
                                                .await;
                                        for d in datagrams {
                                            let _ = connection_tx.send(d.bytes).await;
                                        }
                                    });
                                    fut
                                },
                            ));

                            let _ = tx.send(datagram.bytes).await;
                            Ok(())
                        }
                        Err(e) => {
                            error!(%target_addr, error = %e, "failed to establish outbound TCP connection");
                            Err(e)
                        }
                    }
                }
                Transport::Tls => {
                    debug!(%target_addr, "establishing new outbound TLS connection");
                    match TcpStream::connect(target_addr).await {
                        Ok(stream) => {
                            let connector = create_tls_connector(
                                edge_config.tls_ca_path.as_deref(),
                                edge_config.tls_insecure_skip_verify,
                            )
                            .map_err(|e| {
                                std::io::Error::new(std::io::ErrorKind::InvalidInput, e)
                            })?;
                            let domain = match &edge_config.tls_server_name {
                                Some(name) => ServerName::try_from(name.clone()).map_err(|_| {
                                    std::io::Error::new(
                                        std::io::ErrorKind::InvalidInput,
                                        format!("invalid TLS server name: {name}"),
                                    )
                                })?,
                                None => ServerName::from(target_addr.ip()),
                            };
                            match connector.connect(domain, stream).await {
                                Ok(tls_stream) => {
                                    let (tx, rx) = tokio::sync::mpsc::channel(100);
                                    self.register_tcp_connection(target_addr, tx.clone());

                                    let state_clone = Arc::clone(self);
                                    let config_clone = edge_config.clone();
                                    tokio::spawn(handle_stream_connection(
                                        SipStream::TlsClient(tls_stream),
                                        target_addr,
                                        tx.clone(),
                                        rx,
                                        move |msg_bytes, peer_addr, connection_tx| {
                                            let state = Arc::clone(&state_clone);
                                            let config = config_clone.clone();
                                            let fut: std::pin::Pin<
                                                Box<dyn std::future::Future<Output = ()> + Send>,
                                            > = Box::pin(async move {
                                                let datagrams = handle_datagram(
                                                    &msg_bytes, peer_addr, &state, &config,
                                                )
                                                .await;
                                                for d in datagrams {
                                                    let _ = connection_tx.send(d.bytes).await;
                                                }
                                            });
                                            fut
                                        },
                                    ));

                                    let _ = tx.send(datagram.bytes).await;
                                    Ok(())
                                }
                                Err(e) => {
                                    error!(%target_addr, error = %e, "failed to establish outbound TLS handshake");
                                    Err(std::io::Error::new(
                                        std::io::ErrorKind::ConnectionRefused,
                                        e,
                                    ))
                                }
                            }
                        }
                        Err(e) => {
                            error!(%target_addr, error = %e, "failed to connect TCP for TLS");
                            Err(e)
                        }
                    }
                }
                Transport::Ws | Transport::Wss => {
                    error!(%target_addr, "no active WebSocket connection found for outbound datagram");
                    Err(std::io::Error::new(
                        std::io::ErrorKind::NotConnected,
                        "no active WebSocket connection",
                    ))
                }
                Transport::Udp => {
                    fallback_socket
                        .send_to(&datagram.bytes, target_addr)
                        .await?;
                    Ok(())
                }
            }
        })
    }

    pub async fn send_keepalive_probe(&self, target_str: &str, fallback_socket: &UdpSocket) {
        let Ok(target_addr) = target_str.parse::<SocketAddr>() else {
            return;
        };

        if let Some(tx) = self.get_tcp_connection(&target_addr) {
            let _ = tx.send(b"\r\n\r\n".to_vec()).await;
            return;
        }

        let _ = fallback_socket.send_to(b"\r\n", target_addr).await;
    }
}
