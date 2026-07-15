use super::*;
use crate::discovery::SipNode;
use tokio::sync::RwLock;

#[tokio::test]
async fn test_tcp_framer_reads_multiple_messages_and_body() {
    let (mut writer, reader) = tokio::io::duplex(1024);
    let first = b"MESSAGE sip:a@b SIP/2.0\r\nCall-ID: one\r\nContent-Length: 4\r\n\r\ntest";
    let second = b"OPTIONS sip:a@b SIP/2.0\r\nCall-ID: two\r\nContent-Length: 0\r\n\r\n";
    writer
        .write_all(&[first.as_slice(), second.as_slice()].concat())
        .await
        .expect("write");
    drop(writer);
    let mut framed = SipFrameReader::new(reader);

    assert_eq!(
        framed.read_frame().await.expect("first"),
        Some(first.to_vec())
    );
    assert_eq!(
        framed.read_frame().await.expect("second"),
        Some(second.to_vec())
    );
    assert_eq!(framed.read_frame().await.expect("eof"), None);
}

#[tokio::test]
async fn test_tcp_proxy_adds_and_removes_own_via() {
    let backend_listener = TcpListener::bind("127.0.0.1:0").await.expect("backend");
    let backend_addr = backend_listener.local_addr().expect("backend address");
    let backend = tokio::spawn(async move {
        let (stream, _) = backend_listener.accept().await.expect("accept backend");
        let (read, mut write) = stream.into_split();
        let request = SipFrameReader::new(read)
            .read_frame()
            .await
            .expect("read request")
            .expect("request");
        let router_via = header_value(&request, &["via"]).expect("router Via");
        assert!(router_via.starts_with("SIP/2.0/TCP router.test:5070"));
        let response = format!(
            "SIP/2.0 200 OK\r\nVia: {router_via}\r\nVia: SIP/2.0/TCP client.test:5090;branch=z9hG4bK-client\r\nCall-ID: tcp-call\r\nCSeq: 1 OPTIONS\r\nContent-Length: 0\r\n\r\n"
        );
        write
            .write_all(response.as_bytes())
            .await
            .expect("response");
    });

    let router_listener = TcpListener::bind("127.0.0.1:0").await.expect("router");
    let router_addr = router_listener.local_addr().expect("router address");
    let nodes = Arc::new(RwLock::new(vec![SipNode {
        id: "sip-a".to_string(),
        address: backend_addr,
    }]));
    let config = test_config(router_addr);
    let proxy_nodes = Arc::clone(&nodes);
    let routes = DialogRouteStore::without_redis_for_test(60);
    let proxy = tokio::spawn(async move {
        let (stream, _) = router_listener.accept().await.expect("accept client");
        handle_connection(stream, &config, &proxy_nodes, &routes)
            .await
            .expect("proxy connection");
    });

    let mut client = TcpStream::connect(router_addr)
        .await
        .expect("connect router");
    client
        .write_all(b"OPTIONS sip:test@example.com SIP/2.0\r\nVia: SIP/2.0/TCP client.test:5090;branch=z9hG4bK-client\r\nCall-ID: tcp-call\r\nCSeq: 1 OPTIONS\r\nContent-Length: 0\r\n\r\n")
        .await
        .expect("request");
    let (read, _) = client.into_split();
    let response = SipFrameReader::new(read)
        .read_frame()
        .await
        .expect("read response")
        .expect("response");
    assert_eq!(
        header_value(&response, &["via"]),
        Some("SIP/2.0/TCP client.test:5090;branch=z9hG4bK-client")
    );
    backend.await.expect("backend task");
    proxy.await.expect("proxy task");
}

fn test_config(router_addr: std::net::SocketAddr) -> RouterConfig {
    RouterConfig {
        udp_bind: router_addr.to_string(),
        tcp_bind: router_addr.to_string(),
        advertised_addr: "router.test:5070".to_string(),
        redis_url: "redis://127.0.0.1/0".to_string(),
        node_key_prefix: "test".to_string(),
        discovery_interval_secs: 1,
        transaction_ttl_secs: 64,
        dialog_route_ttl_secs: 60,
        udp_workers: 1,
        udp_queue_capacity: 64,
        max_transactions: 1024,
    }
}
