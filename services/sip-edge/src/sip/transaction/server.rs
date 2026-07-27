//! # 服务端事务状态机
//!
//! 本模块实现 INVITE 和 Non-INVITE 服务端事务的 spawn 函数。
//!
//! ## INVITE 服务端事务（RFC 3261 §17.2.1）
//!
//! ```text
//! Proceeding ──1xx──> Proceeding
//!      │
//!      ├──2xx──> Confirmed (Timer G 重传 2xx，直到 ACK 或 Timer H)
//!      │              │
//!      │              └──ACK──> Terminated (Timer I 收尾)
//!      │
//!      └──3xx-6xx──> Completed (Timer G 重传，Timer H 超时)
//!                          │
//!                          └──ACK──> Confirmed (Timer I) ──> Terminated
//! ```
//!
//! ## Non-INVITE 服务端事务（RFC 3261 §17.2.2）
//!
//! ```text
//! Trying/Proceeding ──1xx/2xx-6xx──> Completed (Timer J 收尾) ──> Terminated
//! ```

use super::event::ServerTransactionEvent;
use super::keys::RequestTransactionKey;
use sip_core::SipRequest;
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tokio::net::UdpSocket;

/// 启动 Non-INVITE 服务端事务 task。
///
/// 事务 task 负责响应重传（Timer J 超时前）和入站请求去重。
/// 测试环境下使用更短的 timer（320ms），生产环境为 32s。
#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_non_invite_server_transaction(
    _key: RequestTransactionKey,
    _initial_request: SipRequest,
    peer: SocketAddr,
    socket: Option<Arc<UdpSocket>>,
    mut event_rx: tokio::sync::mpsc::Receiver<ServerTransactionEvent>,
) {
    tokio::spawn(async move {
        let timer_j_duration = if cfg!(test) {
            Duration::from_millis(320)
        } else {
            Duration::from_secs(32)
        };

        let mut last_response: Option<Vec<u8>> = None;
        let mut last_provisional: Option<Vec<u8>> = None;

        loop {
            tokio::select! {
                event_opt = event_rx.recv() => {
                    let Some(event) = event_opt else {
                        break;
                    };
                    match event {
                        ServerTransactionEvent::Request(_req) => {
                            if let Some(ref resp) = last_response {
                                if let Some(ref s) = socket {
                                    let _ = s.send_to(resp, peer).await;
                                }
                            } else if let Some(ref prov) = last_provisional {
                                if let Some(ref s) = socket {
                                    let _ = s.send_to(prov, peer).await;
                                }
                            }
                        }
                        ServerTransactionEvent::Response {
                            bytes: resp_bytes,
                            send_immediately,
                        } => {
                            let is_provisional = resp_bytes.starts_with(b"SIP/2.0 1");
                            if is_provisional {
                                last_provisional = Some(resp_bytes.clone());
                                if send_immediately {
                                    if let Some(ref s) = socket {
                                        let _ = s.send_to(&resp_bytes, peer).await;
                                    }
                                }
                            } else {
                                last_response = Some(resp_bytes.clone());
                                if send_immediately {
                                    if let Some(ref s) = socket {
                                        let _ = s.send_to(&resp_bytes, peer).await;
                                    }
                                }
                                break;
                            }
                        }
                        ServerTransactionEvent::UpdateLastProvisional(resp_bytes) => {
                            last_provisional = Some(resp_bytes);
                        }
                        ServerTransactionEvent::Ack => {}
                    }
                }
            }
        }

        if last_response.is_some() {
            let timer_j = tokio::time::sleep(timer_j_duration);
            tokio::pin!(timer_j);

            loop {
                tokio::select! {
                    _ = &mut timer_j => {
                        break;
                    }
                    event_opt = event_rx.recv() => {
                        let Some(event) = event_opt else {
                            break;
                        };
                        if let ServerTransactionEvent::Request(_req) = event {
                            if let Some(ref resp) = last_response {
                                if let Some(ref s) = socket {
                                    let _ = s.send_to(resp, peer).await;
                                }
                            }
                        }
                    }
                }
            }
        }
    });
}

/// 启动 INVITE 服务端事务 task。
///
/// 实现 RFC 3261 §17.2.1 的完整状态机：
///
/// - Timer G：2xx/3xx-6xx 响应重传（指数增长，上限 T2）
/// - Timer H：等待 ACK 超时（32s）
/// - Timer I：ACK 后的收尾等待（5s）
/// - Timer L：2xx 响应的重传截止（同 Timer H）
///
/// 测试环境下使用 5ms 起步的 timer 以加速测试。
#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_invite_server_transaction(
    _key: RequestTransactionKey,
    _initial_request: SipRequest,
    peer: SocketAddr,
    socket: Option<Arc<UdpSocket>>,
    mut event_rx: tokio::sync::mpsc::Receiver<ServerTransactionEvent>,
) {
    tokio::spawn(async move {
        let t1 = if cfg!(test) {
            Duration::from_millis(5)
        } else {
            Duration::from_millis(500)
        };
        let t2 = if cfg!(test) {
            Duration::from_millis(40)
        } else {
            Duration::from_secs(4)
        };
        let timer_h_duration = if cfg!(test) {
            Duration::from_millis(320)
        } else {
            Duration::from_secs(32)
        };
        let timer_i_duration = if cfg!(test) {
            Duration::from_millis(50)
        } else {
            Duration::from_secs(5)
        };

        let mut last_response: Option<Vec<u8>> = None;
        let mut last_provisional: Option<Vec<u8>> = None;
        let mut successful_final = false;

        loop {
            tokio::select! {
                event_opt = event_rx.recv() => {
                    let Some(event) = event_opt else {
                        break;
                    };
                    match event {
                        ServerTransactionEvent::Request(_req) => {
                            if let Some(ref prov) = last_provisional {
                                if let Some(ref s) = socket {
                                    let _ = s.send_to(prov, peer).await;
                                }
                            }
                        }
                        ServerTransactionEvent::Response {
                            bytes: resp_bytes,
                            send_immediately,
                        } => {
                            let is_provisional = resp_bytes.starts_with(b"SIP/2.0 1");
                            if is_provisional {
                                last_provisional = Some(resp_bytes.clone());
                                if send_immediately {
                                    if let Some(ref s) = socket {
                                        let _ = s.send_to(&resp_bytes, peer).await;
                                    }
                                }
                            } else {
                                let is_2xx = resp_bytes.starts_with(b"SIP/2.0 2");
                                if is_2xx {
                                    last_response = Some(resp_bytes.clone());
                                    if send_immediately {
                                        if let Some(ref s) = socket {
                                            let _ = s.send_to(&resp_bytes, peer).await;
                                        }
                                    }
                                    successful_final = true;
                                    break;
                                } else {
                                    last_response = Some(resp_bytes.clone());
                                    if send_immediately {
                                        if let Some(ref s) = socket {
                                            let _ = s.send_to(&resp_bytes, peer).await;
                                        }
                                    }
                                    break;
                                }
                            }
                        }
                        ServerTransactionEvent::UpdateLastProvisional(resp_bytes) => {
                            last_provisional = Some(resp_bytes);
                        }
                        ServerTransactionEvent::Ack => {
                            return;
                        }
                    }
                }
            }
        }

        if successful_final {
            let mut retransmit_interval = t1;
            let timer_l = tokio::time::sleep(timer_h_duration);
            tokio::pin!(timer_l);
            let mut retransmit_timer = Box::pin(tokio::time::sleep(retransmit_interval));

            loop {
                tokio::select! {
                    _ = &mut timer_l => break,
                    _ = &mut retransmit_timer => {
                        if let (Some(response), Some(s)) = (&last_response, &socket) {
                            let _ = s.send_to(response, peer).await;
                        }
                        retransmit_interval = std::cmp::min(retransmit_interval * 2, t2);
                        retransmit_timer = Box::pin(tokio::time::sleep(retransmit_interval));
                    }
                    event_opt = event_rx.recv() => {
                        let Some(event) = event_opt else {
                            break;
                        };
                        match event {
                            ServerTransactionEvent::Request(_) => {
                                if let (Some(response), Some(s)) = (&last_response, &socket) {
                                    let _ = s.send_to(response, peer).await;
                                }
                            }
                            ServerTransactionEvent::Ack => {
                                break;
                            }
                            _ => {}
                        }
                    }
                }
            }
            return;
        }

        if let Some(final_resp) = last_response {
            let mut current_g_timer = t1;
            let timer_h = tokio::time::sleep(timer_h_duration);
            tokio::pin!(timer_h);

            let mut timer_g = Box::pin(tokio::time::sleep(current_g_timer));
            let mut got_ack = false;

            loop {
                tokio::select! {
                    _ = &mut timer_h => {
                        break;
                    }
                    _ = &mut timer_g => {
                        if let Some(ref s) = socket {
                            let _ = s.send_to(&final_resp, peer).await;
                        }
                        current_g_timer = std::cmp::min(current_g_timer * 2, t2);
                        timer_g = Box::pin(tokio::time::sleep(current_g_timer));
                    }
                    event_opt = event_rx.recv() => {
                        let Some(event) = event_opt else {
                            break;
                        };
                        match event {
                            ServerTransactionEvent::Request(_req) => {
                                if let Some(ref s) = socket {
                                    let _ = s.send_to(&final_resp, peer).await;
                                }
                            }
                            ServerTransactionEvent::Ack => {
                                got_ack = true;
                                break;
                            }
                            _ => {}
                        }
                    }
                }
            }

            if got_ack {
                let timer_i = tokio::time::sleep(timer_i_duration);
                tokio::pin!(timer_i);

                loop {
                    tokio::select! {
                        _ = &mut timer_i => {
                            break;
                        }
                        event_opt = event_rx.recv() => {
                            if event_opt.is_none() {
                                break;
                            }
                        }
                    }
                }
            }
        }
    });
}
