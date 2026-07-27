//! 事务模块单元测试。

use super::event::ServerTransactionEvent;
use super::keys::{InviteAckKey, RequestTransactionKey};
use super::server::{spawn_invite_server_transaction, spawn_non_invite_server_transaction};
use sip_core::parse_message;
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

    let first = RequestTransactionKey::from_request(&request, "192.0.2.10:5060".parse().unwrap());
    let second = RequestTransactionKey::from_request(&request, "192.0.2.11:5060".parse().unwrap());

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
        RequestTransactionKey::from_request(&request, "192.0.2.10:5060".parse().unwrap()).is_none()
    );
}

#[test]
fn invite_ack_key_ignores_the_separate_ack_branch() {
    let invite = request(concat!(
        "INVITE sip:1002@example.com SIP/2.0\r\n",
        "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-invite\r\n",
        "Call-ID: 2af167fa-2b9e-47e9-9c3a-7d20c4361f8c\r\n",
        "CSeq: 42 INVITE\r\n",
        "Content-Length: 0\r\n\r\n"
    ));
    let ack = request(concat!(
        "ACK sip:1002@example.com SIP/2.0\r\n",
        "Via: SIP/2.0/UDP 192.0.2.10:5060;branch=z9hG4bK-new-ack\r\n",
        "Call-ID: 2af167fa-2b9e-47e9-9c3a-7d20c4361f8c\r\n",
        "CSeq: 42 ACK\r\n",
        "Content-Length: 0\r\n\r\n"
    ));

    assert_eq!(
        InviteAckKey::from_request(&invite),
        InviteAckKey::from_request(&ack)
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
        .send(ServerTransactionEvent::send_response(resp_bytes.clone()))
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
        .send(ServerTransactionEvent::send_response(trying_bytes.clone()))
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
        .send(ServerTransactionEvent::send_response(final_bytes.clone()))
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
    let _ = request(concat!(
        "ACK sip:1002@example.com SIP/2.0\r\n",
        "Via: SIP/2.0/UDP 127.0.0.1:0;branch=z9hG4bK-invite\r\n",
        "Call-ID: call-invite@example.com\r\n",
        "CSeq: 1 ACK\r\n",
        "Content-Length: 0\r\n",
        "\r\n"
    ));
    event_tx.send(ServerTransactionEvent::Ack).await.unwrap();

    // In Confirmed state, Timer G retransmissions must stop.
    // Wait and verify we don't receive any more packets.
    let timeout_res =
        tokio::time::timeout(Duration::from_millis(50), client_socket.recv_from(&mut buf)).await;
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
async fn test_invite_2xx_transaction_retransmits_until_ack() {
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
        .send(ServerTransactionEvent::send_response(
            final_response.clone(),
        ))
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

    // A successful final response is retransmitted by the UAS core even when the caller's
    // duplicate INVITE was lost.
    let (length, _) = tokio::time::timeout(
        Duration::from_millis(50),
        client_socket.recv_from(&mut buffer),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(&buffer[..length], final_response);

    let _ = request(concat!(
        "ACK sip:1002@example.com SIP/2.0\r\n",
        "Via: SIP/2.0/UDP 127.0.0.1:0;branch=z9hG4bK-ack\r\n",
        "Call-ID: call-success@example.com\r\n",
        "CSeq: 1 ACK\r\n",
        "Content-Length: 0\r\n\r\n"
    ));
    event_tx.send(ServerTransactionEvent::Ack).await.unwrap();
    assert!(
        tokio::time::timeout(
            Duration::from_millis(60),
            client_socket.recv_from(&mut buffer)
        )
        .await
        .is_err(),
        "2xx retransmission must stop after ACK"
    );
}

fn request(raw: &str) -> sip_core::SipRequest {
    let sip_core::SipMessageBorrow::Request(request) = parse_message(raw.as_bytes()).unwrap()
    else {
        panic!("expected request");
    };
    request.into_owned()
}
