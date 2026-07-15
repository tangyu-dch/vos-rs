use super::*;
use crate::discovery::SipNode;
use crate::proxy::select_node;
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
    let (read, write) = client.into_split();
    let response = SipFrameReader::new(read)
        .read_frame()
        .await
        .expect("read response")
        .expect("response");
    assert_eq!(
        header_value(&response, &["via"]),
        Some("SIP/2.0/TCP client.test:5090;branch=z9hG4bK-client")
    );
    drop(write);
    backend.await.expect("backend task");
    proxy.await.expect("proxy task");
}

#[tokio::test]
async fn test_one_tcp_client_routes_two_call_ids_to_different_nodes() {
    let backend_a = TcpListener::bind("127.0.0.1:0").await.expect("backend a");
    let backend_b = TcpListener::bind("127.0.0.1:0").await.expect("backend b");
    let nodes_list = vec![
        SipNode {
            id: "sip-a".to_string(),
            address: backend_a.local_addr().expect("backend a address"),
        },
        SipNode {
            id: "sip-b".to_string(),
            address: backend_b.local_addr().expect("backend b address"),
        },
    ];
    let call_a = call_id_for_node("sip-a", &nodes_list);
    let call_b = call_id_for_node("sip-b", &nodes_list);
    let task_a = tokio::spawn(echo_backend(backend_a));
    let task_b = tokio::spawn(echo_backend(backend_b));

    let router_listener = TcpListener::bind("127.0.0.1:0").await.expect("router");
    let router_addr = router_listener.local_addr().expect("router address");
    let nodes = Arc::new(RwLock::new(nodes_list));
    let config = test_config(router_addr);
    let routes = DialogRouteStore::without_redis_for_test(60);
    let proxy_nodes = Arc::clone(&nodes);
    let proxy = tokio::spawn(async move {
        let (stream, _) = router_listener.accept().await.expect("accept client");
        handle_connection(stream, &config, &proxy_nodes, &routes)
            .await
            .expect("proxy connection");
    });

    let stream = TcpStream::connect(router_addr)
        .await
        .expect("connect router");
    let (read, mut write) = stream.into_split();
    write
        .write_all(&[tcp_options(&call_a), tcp_options(&call_b)].concat())
        .await
        .expect("two requests");
    let mut reader = SipFrameReader::new(read);
    let first = reader
        .read_frame()
        .await
        .expect("first read")
        .expect("first");
    let second = reader
        .read_frame()
        .await
        .expect("second read")
        .expect("second");
    let responses = [
        header_value(&first, &["call-id"]).expect("first call-id"),
        header_value(&second, &["call-id"]).expect("second call-id"),
    ];
    assert!(responses.contains(&call_a.as_str()));
    assert!(responses.contains(&call_b.as_str()));

    drop(write);
    assert_eq!(task_a.await.expect("backend a"), call_a);
    assert_eq!(task_b.await.expect("backend b"), call_b);
    proxy.await.expect("proxy task");
}

async fn echo_backend(listener: TcpListener) -> String {
    let (stream, _) = listener.accept().await.expect("accept backend");
    let (read, mut write) = stream.into_split();
    let request = SipFrameReader::new(read)
        .read_frame()
        .await
        .expect("backend read")
        .expect("backend request");
    let call_id = header_value(&request, &["call-id"])
        .expect("call-id")
        .to_string();
    let router_via = header_value(&request, &["via"])
        .expect("router via")
        .to_string();
    let response = format!(
        "SIP/2.0 200 OK\r\nVia: {router_via}\r\nVia: SIP/2.0/TCP client.test:5090;branch=z9hG4bK-client\r\nCall-ID: {call_id}\r\nCSeq: 1 OPTIONS\r\nContent-Length: 0\r\n\r\n"
    );
    write
        .write_all(response.as_bytes())
        .await
        .expect("backend response");
    call_id
}

fn call_id_for_node(node_id: &str, nodes: &[SipNode]) -> String {
    (0..10_000)
        .map(|index| format!("tcp-multiplex-{index}"))
        .find(|call_id| select_node(call_id, nodes).is_some_and(|node| node.id == node_id))
        .expect("call-id for node")
}

fn tcp_options(call_id: &str) -> Vec<u8> {
    format!(
        "OPTIONS sip:test@example.com SIP/2.0\r\nVia: SIP/2.0/TCP client.test:5090;branch=z9hG4bK-{call_id}\r\nCall-ID: {call_id}\r\nCSeq: 1 OPTIONS\r\nContent-Length: 0\r\n\r\n"
    )
    .into_bytes()
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
        tcp_max_connections: 64,
        tcp_write_queue_capacity: 64,
        tcp_idle_timeout_secs: 30,
        tcp_connect_timeout_secs: 1,
    }
}
