//! # SIP 事务管理
//!
//! 本模块实现了 SIP 事务状态机，包括：
//!
//! - **INVITE 客户端事务**：出站 INVITE 的重传和超时处理
//! - **Non-INVITE 客户端事务**：出站 BYE/OPTIONS 等的重传
//! - **INVITE 服务端事务**：入站 INVITE 的响应处理
//! - **Non-INVITE 服务端事务**：入站 BYE/INFO 等的响应处理
//!
//! ## 事务状态机
//!
//! ```text
//! INVITE 客户端事务：
//!   调用 → Calling → Proceeding → Completed → Terminated
//!
//! Non-INVITE 客户端事务：
//!   调用 → Trying → Proceeding → Completed → Terminated
//! ```
//!
//! ## 重传机制
//!
//! - INVITE 事务使用 Timer A（初始重传间隔）和 Timer B（事务超时）
//! - 非 INVITE 事务使用 Timer F（事务超时）
//! - 重传间隔指数增长，最大不超过 Timer B/F

use sip_core::{Method, SipRequest};
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tokio::net::UdpSocket;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct ClientTransactionKey {
    pub call_id: String,
    pub method: String,
    pub branch: String,
}

impl ClientTransactionKey {
    pub(crate) fn from_request(request: &SipRequest) -> Option<Self> {
        if matches!(&request.method, Method::Ack) {
            return None;
        }
        let branch = request
            .headers
            .get("via")
            .and_then(|via| branch_param(via.as_str()))?;
        let call_id = request.headers.get("call-id")?.as_str().to_string();
        let method = request.method.as_str().to_string();
        Some(Self {
            call_id,
            method,
            branch,
        })
    }

    pub(crate) fn from_response(response: &sip_core::SipResponse) -> Option<Self> {
        let branch = response
            .headers
            .get("via")
            .and_then(|via| branch_param(via.as_str()))?;
        let call_id = response.headers.get("call-id")?.as_str().to_string();
        let cseq = response.headers.get("cseq")?.as_str();
        let method = cseq.split_whitespace().nth(1)?.to_string();
        Some(Self {
            call_id,
            method,
            branch,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct RequestTransactionKey {
    peer: String,
    method: String,
    branch: Option<String>,
    call_id: Option<String>,
    cseq: Option<String>,
}

impl RequestTransactionKey {
    pub(crate) fn from_request(request: &SipRequest, peer: SocketAddr) -> Option<Self> {
        if matches!(&request.method, Method::Ack) {
            return None;
        }

        let branch = request
            .headers
            .get("via")
            .and_then(|via| branch_param(via.as_str()));
        let call_id = request
            .headers
            .get("call-id")
            .map(|value| value.as_str().to_string());
        let cseq = request
            .headers
            .get("cseq")
            .map(|value| value.as_str().to_string());

        if branch.is_none() && call_id.is_none() && cseq.is_none() {
            return None;
        }

        Some(Self {
            peer: peer.to_string(),
            method: request.method.as_str().to_string(),
            branch,
            call_id,
            cseq,
        })
    }

    pub(crate) fn new_manual(
        peer: String,
        method: String,
        branch: Option<String>,
        call_id: Option<String>,
        cseq: Option<String>,
    ) -> Self {
        Self {
            peer,
            method,
            branch,
            call_id,
            cseq,
        }
    }
}

pub(crate) fn branch_param(via: &str) -> Option<String> {
    via.split(';').skip(1).find_map(|param| {
        let (name, value) = param.trim().split_once('=')?;
        name.trim()
            .eq_ignore_ascii_case("branch")
            .then(|| value.trim().to_string())
            .filter(|value| !value.is_empty())
    })
}

#[cfg(test)]
mod tests {
    use super::{
        spawn_invite_server_transaction, spawn_non_invite_server_transaction,
        RequestTransactionKey, ServerTransactionEvent,
    };
    use sip_core::{parse_message, SipMessage};
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::net::UdpSocket;
    use tokio::sync::mpsc;

    #[test]
    fn key_uses_branch_call_id_cseq_method_and_peer() {
        let request = request(concat!(
            "INVITE sip:1002@example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;received=198.51.100.10;branch=z9hG4bK-abc\r\n",
            "Call-ID: call-1@example.com\r\n",
            "CSeq: 1 INVITE\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        ));

        let first =
            RequestTransactionKey::from_request(&request, "192.0.2.10:5060".parse().unwrap());
        let second =
            RequestTransactionKey::from_request(&request, "192.0.2.11:5060".parse().unwrap());

        assert_ne!(first, second);
        assert!(first.is_some());
    }

    #[test]
    fn ack_does_not_create_request_transaction_key() {
        let request = request(concat!(
            "ACK sip:1002@example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-ack\r\n",
            "Call-ID: call-1@example.com\r\n",
            "CSeq: 1 ACK\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        ));

        assert!(
            RequestTransactionKey::from_request(&request, "192.0.2.10:5060".parse().unwrap())
                .is_none()
        );
    }

    #[tokio::test]
    async fn test_non_invite_server_transaction_retransmission() {
        let client_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let client_addr = client_socket.local_addr().unwrap();

        let server_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let _server_addr = server_socket.local_addr().unwrap();

        let key = RequestTransactionKey::new_manual(
            client_addr.to_string(),
            "OPTIONS".to_string(),
            Some("z9hG4bK-non-invite".to_string()),
            Some("call-non-invite@example.com".to_string()),
            Some("1 OPTIONS".to_string()),
        );

        let initial_request = request(concat!(
            "OPTIONS sip:edge.example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 127.0.0.1:0;branch=z9hG4bK-non-invite\r\n",
            "Call-ID: call-non-invite@example.com\r\n",
            "CSeq: 1 OPTIONS\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        ));

        let (event_tx, event_rx) = mpsc::channel(16);

        spawn_non_invite_server_transaction(
            key,
            initial_request.clone(),
            client_addr,
            Some(Arc::new(server_socket)),
            event_rx,
        );

        // Feed Response
        let resp_bytes = b"SIP/2.0 200 OK\r\nContent-Length: 0\r\n\r\n".to_vec();
        event_tx
            .send(ServerTransactionEvent::Response(resp_bytes.clone()))
            .await
            .unwrap();

        // Verify client receives response
        let mut buf = [0u8; 1024];
        let (len, _from) = tokio::time::timeout(
            Duration::from_millis(100),
            client_socket.recv_from(&mut buf),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(&buf[..len], &resp_bytes);

        // Feed duplicate request
        event_tx
            .send(ServerTransactionEvent::Request(initial_request))
            .await
            .unwrap();

        // Verify client receives retransmitted response
        let (len, _from) = tokio::time::timeout(
            Duration::from_millis(100),
            client_socket.recv_from(&mut buf),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(&buf[..len], &resp_bytes);

        // Wait for Timer J (320ms)
        tokio::time::sleep(Duration::from_millis(400)).await;

        // Verify transaction task is terminated (channel is closed)
        assert!(event_tx.is_closed());
    }

    #[tokio::test]
    async fn test_invite_server_transaction_lifecycle() {
        let client_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let client_addr = client_socket.local_addr().unwrap();

        let server_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let _server_addr = server_socket.local_addr().unwrap();

        let key = RequestTransactionKey::new_manual(
            client_addr.to_string(),
            "INVITE".to_string(),
            Some("z9hG4bK-invite".to_string()),
            Some("call-invite@example.com".to_string()),
            Some("1 INVITE".to_string()),
        );

        let initial_request = request(concat!(
            "INVITE sip:1002@example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 127.0.0.1:0;branch=z9hG4bK-invite\r\n",
            "Call-ID: call-invite@example.com\r\n",
            "CSeq: 1 INVITE\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        ));

        let (event_tx, event_rx) = mpsc::channel(16);

        spawn_invite_server_transaction(
            key,
            initial_request.clone(),
            client_addr,
            Some(Arc::new(server_socket)),
            event_rx,
        );

        // Feed provisional response
        let trying_bytes = b"SIP/2.0 100 Trying\r\nContent-Length: 0\r\n\r\n".to_vec();
        event_tx
            .send(ServerTransactionEvent::Response(trying_bytes.clone()))
            .await
            .unwrap();

        // Verify client receives 100 Trying
        let mut buf = [0u8; 1024];
        let (len, _from) = tokio::time::timeout(
            Duration::from_millis(100),
            client_socket.recv_from(&mut buf),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(&buf[..len], &trying_bytes);

        // Feed duplicate INVITE request
        event_tx
            .send(ServerTransactionEvent::Request(initial_request.clone()))
            .await
            .unwrap();

        // Verify client receives retransmitted 100 Trying
        let (len, _from) = tokio::time::timeout(
            Duration::from_millis(100),
            client_socket.recv_from(&mut buf),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(&buf[..len], &trying_bytes);

        // Feed final 302 response
        let final_bytes = b"SIP/2.0 302 Moved\r\nContent-Length: 0\r\n\r\n".to_vec();
        event_tx
            .send(ServerTransactionEvent::Response(final_bytes.clone()))
            .await
            .unwrap();

        // Verify client receives 302 final response
        let (len, _from) = tokio::time::timeout(
            Duration::from_millis(100),
            client_socket.recv_from(&mut buf),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(&buf[..len], &final_bytes);

        // Timer G should trigger retransmission of 302 (t1 is 5ms)
        let (len, _from) =
            tokio::time::timeout(Duration::from_millis(50), client_socket.recv_from(&mut buf))
                .await
                .unwrap()
                .unwrap();
        assert_eq!(&buf[..len], &final_bytes);

        // Send ACK
        let ack_request = request(concat!(
            "ACK sip:1002@example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 127.0.0.1:0;branch=z9hG4bK-invite\r\n",
            "Call-ID: call-invite@example.com\r\n",
            "CSeq: 1 ACK\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        ));
        event_tx
            .send(ServerTransactionEvent::Ack(ack_request))
            .await
            .unwrap();

        // In Confirmed state, Timer G retransmissions must stop.
        // Wait and verify we don't receive any more packets.
        let timeout_res =
            tokio::time::timeout(Duration::from_millis(50), client_socket.recv_from(&mut buf))
                .await;
        assert!(
            timeout_res.is_err(),
            "should not receive retransmission after ACK"
        );

        // Wait for Timer I to expire (50ms)
        tokio::time::sleep(Duration::from_millis(80)).await;

        // Verify transaction task is terminated (channel is closed)
        assert!(event_tx.is_closed());
    }

    #[tokio::test]
    async fn test_invite_2xx_transaction_replays_final_for_duplicate_request() {
        let client_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let client_addr = client_socket.local_addr().unwrap();
        let server_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let initial_request = request(concat!(
            "INVITE sip:1002@example.com SIP/2.0\r\n",
            "Via: SIP/2.0/UDP 127.0.0.1:0;branch=z9hG4bK-success\r\n",
            "Call-ID: call-success@example.com\r\n",
            "CSeq: 1 INVITE\r\n",
            "Content-Length: 0\r\n",
            "\r\n"
        ));
        let key = RequestTransactionKey::from_request(&initial_request, client_addr).unwrap();
        let (event_tx, event_rx) = mpsc::channel(16);
        spawn_invite_server_transaction(
            key,
            initial_request.clone(),
            client_addr,
            Some(Arc::new(server_socket)),
            event_rx,
        );

        let provisional = b"SIP/2.0 183 Session Progress\r\nContent-Length: 0\r\n\r\n".to_vec();
        event_tx
            .send(ServerTransactionEvent::UpdateLastProvisional(provisional))
            .await
            .unwrap();
        let final_response = b"SIP/2.0 200 OK\r\nContent-Length: 0\r\n\r\n".to_vec();
        event_tx
            .send(ServerTransactionEvent::Response(final_response.clone()))
            .await
            .unwrap();

        let mut buffer = [0_u8; 1024];
        let (length, _) = tokio::time::timeout(
            Duration::from_millis(100),
            client_socket.recv_from(&mut buffer),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(&buffer[..length], final_response);

        event_tx
            .send(ServerTransactionEvent::Request(initial_request))
            .await
            .unwrap();
        let (length, _) = tokio::time::timeout(
            Duration::from_millis(100),
            client_socket.recv_from(&mut buffer),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(&buffer[..length], final_response);
    }

    fn request(raw: &str) -> sip_core::SipRequest {
        let SipMessage::Request(request) = parse_message(raw.as_bytes()).unwrap() else {
            panic!("expected request");
        };
        request
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) enum ServerTransactionEvent {
    Request(SipRequest),
    Response(Vec<u8>),
    UpdateLastProvisional(Vec<u8>),
    Ack(SipRequest),
}

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
                        ServerTransactionEvent::Response(resp_bytes) => {
                            let is_provisional = resp_bytes.starts_with(b"SIP/2.0 1");
                            if is_provisional {
                                last_provisional = Some(resp_bytes.clone());
                                if let Some(ref s) = socket {
                                    let _ = s.send_to(&resp_bytes, peer).await;
                                }
                            } else {
                                last_response = Some(resp_bytes.clone());
                                if let Some(ref s) = socket {
                                    let _ = s.send_to(&resp_bytes, peer).await;
                                }
                                break;
                            }
                        }
                        ServerTransactionEvent::UpdateLastProvisional(resp_bytes) => {
                            last_provisional = Some(resp_bytes);
                        }
                        ServerTransactionEvent::Ack(_) => {}
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
                        ServerTransactionEvent::Response(resp_bytes) => {
                            let is_provisional = resp_bytes.starts_with(b"SIP/2.0 1");
                            if is_provisional {
                                last_provisional = Some(resp_bytes.clone());
                                if let Some(ref s) = socket {
                                    let _ = s.send_to(&resp_bytes, peer).await;
                                }
                            } else {
                                let is_2xx = resp_bytes.starts_with(b"SIP/2.0 2");
                                if is_2xx {
                                    last_response = Some(resp_bytes.clone());
                                    if let Some(ref s) = socket {
                                        let _ = s.send_to(&resp_bytes, peer).await;
                                    }
                                    successful_final = true;
                                    break;
                                } else {
                                    last_response = Some(resp_bytes.clone());
                                    if let Some(ref s) = socket {
                                        let _ = s.send_to(&resp_bytes, peer).await;
                                    }
                                    break;
                                }
                            }
                        }
                        ServerTransactionEvent::UpdateLastProvisional(resp_bytes) => {
                            last_provisional = Some(resp_bytes);
                        }
                        ServerTransactionEvent::Ack(_) => {}
                    }
                }
            }
        }

        if successful_final {
            let timer_l = tokio::time::sleep(timer_h_duration);
            tokio::pin!(timer_l);

            loop {
                tokio::select! {
                    _ = &mut timer_l => break,
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
                            ServerTransactionEvent::Ack(_) => break,
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
                            ServerTransactionEvent::Ack(_) => {
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
