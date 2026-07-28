//! 运行时任务启动辅助函数
//!
//! 从 `main.rs` 提取的 CDR 批量写入 worker 和 UDP worker 池启动逻辑。

use crate::cdr;
use crate::edge_state::{CdrSinks, EdgeState};
use crate::net::{PooledBuffer, Transport};
use crate::sip;
use crate::sip::client_transaction::spawn_client_transaction_retransmission;
use sip_core::{parse_message, Method, SipMessageBorrow};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::UdpSocket;
use tracing::{debug, warn};

/// 启动 CDR 批量写入 worker，返回关闭信号和任务句柄。
///
/// Worker 从 channel 消费 CDR 记录，按 100 条或 100ms 间隔批量刷新到持久化 sink。
/// 收到关闭信号后会排空剩余记录再退出。
pub(crate) fn spawn_cdr_worker(
    mut cdr_rx: tokio::sync::mpsc::Receiver<call_core::CallCdr>,
    cdr_sinks: Arc<CdrSinks>,
    cdr_spool: cdr::CdrSpool,
    cdr_persistence_enabled: bool,
) -> (
    tokio::sync::oneshot::Sender<()>,
    tokio::task::JoinHandle<()>,
) {
    let cdr_sinks_bg = Arc::clone(&cdr_sinks);
    let cdr_spool_bg = cdr_spool.clone();
    let (cdr_shutdown_tx, mut cdr_shutdown_rx) = tokio::sync::oneshot::channel();
    let cdr_worker = tokio::spawn(async move {
        let mut batch = Vec::new();
        let mut interval = tokio::time::interval(Duration::from_millis(100));
        loop {
            tokio::select! {
                Some(cdr) = cdr_rx.recv() => {
                    batch.push(cdr);
                    if batch.len() >= 100 && cdr_persistence_enabled {
                        flush_cdr_batch(&cdr_sinks_bg, &cdr_spool_bg, &batch).await;
                        batch.clear();
                    } else if batch.len() >= 100 {
                        batch.clear();
                    }
                }
                _ = interval.tick() => {
                    if !batch.is_empty() && cdr_persistence_enabled {
                        flush_cdr_batch(&cdr_sinks_bg, &cdr_spool_bg, &batch).await;
                        batch.clear();
                    } else if !batch.is_empty() {
                        batch.clear();
                    }
                }
                _ = &mut cdr_shutdown_rx => {
                    while let Ok(cdr) = cdr_rx.try_recv() {
                        batch.push(cdr);
                    }
                    if !batch.is_empty() && cdr_persistence_enabled {
                        flush_cdr_batch(&cdr_sinks_bg, &cdr_spool_bg, &batch).await;
                    }
                    break;
                }
            }
        }
    });
    (cdr_shutdown_tx, cdr_worker)
}

async fn flush_cdr_batch(
    sinks: &Arc<CdrSinks>,
    spool: &cdr::CdrSpool,
    batch: &[call_core::CallCdr],
) {
    crate::cdr::flush_cdr_batch_with_retry_and_spool(sinks, spool, batch).await;
}

/// 启动 UDP worker 池，返回每个 worker 的发送端 channel。
///
/// 每个 worker 独立处理 SIP 数据报：解析、路由、事务管理、发送响应。
pub(crate) fn spawn_udp_workers(
    edge_state: Arc<EdgeState>,
    socket: Arc<UdpSocket>,
    edge_config: Arc<crate::config::EdgeConfig>,
    num_workers: usize,
    queue_capacity: usize,
) -> Vec<tokio::sync::mpsc::Sender<(PooledBuffer, SocketAddr)>> {
    let mut worker_txs = Vec::with_capacity(num_workers);

    for worker_id in 0..num_workers {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<(PooledBuffer, SocketAddr)>(queue_capacity);
        worker_txs.push(tx);

        let state = Arc::clone(&edge_state);
        let sock = Arc::clone(&socket);
        let cfg = edge_config.clone();

        tokio::spawn(async move {
            debug!("UDP Worker {} started", worker_id);
            while let Some((packet, peer)) = rx.recv().await {
                let datagrams = sip::handle_datagram(&packet, peer, &state, &cfg).await;
                if datagrams.is_empty() {
                    debug!(%peer, "received datagram without response");
                }

                for datagram in datagrams {
                    let transport = if let Ok(msg) = parse_message(&datagram.bytes) {
                        if let Some(via) = msg.headers().get("via") {
                            let via_str = via.as_str().to_uppercase();
                            if via_str.contains("SIP/2.0/TLS") {
                                Transport::Tls
                            } else if via_str.contains("SIP/2.0/TCP") {
                                Transport::Tcp
                            } else {
                                Transport::Udp
                            }
                        } else {
                            Transport::Udp
                        }
                    } else {
                        Transport::Udp
                    };

                    let client_transaction_key =
                        if transport == Transport::Udp && datagram.is_request() {
                            parse_message(&datagram.bytes)
                                .ok()
                                .and_then(|message| match message {
                                    SipMessageBorrow::Request(request)
                                        if !matches!(&request.method, Method::Ack) =>
                                    {
                                        sip::ClientTransactionKey::from_request(&request)
                                    }
                                    _ => None,
                                })
                        } else {
                            None
                        };
                    let registered_transaction = client_transaction_key.clone().and_then(|key| {
                        spawn_client_transaction_retransmission(
                            Arc::clone(&state),
                            Arc::clone(&sock),
                            datagram.target.clone(),
                            datagram.bytes.clone(),
                            key.clone(),
                            cfg.clone(),
                        )
                        .then_some(key)
                    });
                    if client_transaction_key.is_some() && registered_transaction.is_none() {
                        continue;
                    }

                    if let Err(error) = state.send_sip_datagram(datagram.clone(), &sock, &cfg).await
                    {
                        if let Some(key) = registered_transaction.as_ref() {
                            state.client_transactions.cancel(key);
                        }
                        warn!(target = %datagram.target, error = %error, "failed to send SIP message");
                    } else if datagram.bytes.starts_with(b"INVITE ") {
                        let msg_head = String::from_utf8_lossy(
                            &datagram.bytes[..datagram.bytes.len().min(300)],
                        );
                        debug!(target = %datagram.target, head = %msg_head, "sending outbound INVITE datagram");
                    } else {
                        debug!(
                            peer = %datagram.target,
                            bytes = datagram.bytes.len(),
                            "sent SIP datagram"
                        );
                    }
                }
            }
        });
    }

    worker_txs
}

/// UDP 入站数据包缓冲池容量计算。
pub(crate) fn udp_buffer_pool_capacity(num_workers: usize, queue_capacity: usize) -> usize {
    (num_workers * queue_capacity).min(4096) + 256
}
