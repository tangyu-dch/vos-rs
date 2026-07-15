use std::sync::Arc;

use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

use crate::{
    config::RouterConfig,
    discovery::SharedNodes,
    proxy::{add_router_via, header_value, remove_top_via, router_branch, top_via_branch},
    routes::DialogRouteStore,
};

const MAX_SIP_MESSAGE_BYTES: usize = 1024 * 1024;

type BoxError = Box<dyn std::error::Error + Send + Sync>;

pub(crate) async fn run(
    config: RouterConfig,
    nodes: SharedNodes,
    routes: Arc<DialogRouteStore>,
) -> Result<(), BoxError> {
    let listener = TcpListener::bind(&config.tcp_bind).await?;
    tracing::info!(bind = %config.tcp_bind, "原生 SIP TCP 路由器已启动");
    loop {
        let (stream, peer) = listener.accept().await?;
        let config = config.clone();
        let nodes = Arc::clone(&nodes);
        let routes = Arc::clone(&routes);
        tokio::spawn(async move {
            if let Err(error) = handle_connection(stream, &config, &nodes, &routes).await {
                tracing::debug!(%peer, %error, "SIP TCP 路由连接已关闭");
            }
        });
    }
}

async fn handle_connection(
    client: TcpStream,
    config: &RouterConfig,
    nodes: &SharedNodes,
    routes: &DialogRouteStore,
) -> Result<(), BoxError> {
    let (client_read, mut client_write) = client.into_split();
    let mut client_reader = SipFrameReader::new(client_read);
    let first = client_reader
        .read_frame()
        .await?
        .ok_or("TCP 客户端未发送 SIP 消息")?;
    if is_response(&first) {
        return Err("TCP 连接首包不能是 SIP 响应".into());
    }
    let call_id = header_value(&first, &["call-id", "i"]).ok_or("SIP 请求缺少 Call-ID")?;
    let backend = {
        let snapshot = nodes.read().await;
        let node = routes.resolve(call_id, &snapshot).await?;
        (node.id, node.address)
    };
    let backend_stream = TcpStream::connect(backend.1).await?;
    let (backend_read, mut backend_write) = backend_stream.into_split();
    let mut backend_reader = SipFrameReader::new(backend_read);
    tracing::debug!(backend = %backend.0, "SIP TCP 连接已绑定后端节点");

    let first = add_tcp_via(&first, &config.advertised_addr)?;
    backend_write.write_all(&first).await?;

    let client_to_backend = async {
        while let Some(frame) = client_reader.read_frame().await? {
            let forwarded = if is_response(&frame) {
                frame
            } else {
                add_tcp_via(&frame, &config.advertised_addr)?
            };
            backend_write.write_all(&forwarded).await?;
        }
        Ok::<(), BoxError>(())
    };

    let backend_to_client = async {
        while let Some(frame) = backend_reader.read_frame().await? {
            let forwarded = if is_response(&frame)
                && top_via_branch(&frame)
                    .is_some_and(|branch| branch.starts_with("z9hG4bK-vosrs-tcp-"))
            {
                remove_top_via(&frame)?
            } else {
                frame
            };
            client_write.write_all(&forwarded).await?;
        }
        Ok::<(), BoxError>(())
    };

    tokio::try_join!(client_to_backend, backend_to_client)?;
    Ok(())
}

fn add_tcp_via(frame: &[u8], advertised_addr: &str) -> Result<Vec<u8>, &'static str> {
    let branch = router_branch(frame, "TCP")?.replacen("z9hG4bK-vosrs-", "z9hG4bK-vosrs-tcp-", 1);
    add_router_via(frame, advertised_addr, "TCP", &branch)
}

fn is_response(frame: &[u8]) -> bool {
    frame.starts_with(b"SIP/2.0 ")
}

struct SipFrameReader<R> {
    reader: R,
    buffer: Vec<u8>,
}

impl<R: AsyncRead + Unpin> SipFrameReader<R> {
    fn new(reader: R) -> Self {
        Self {
            reader,
            buffer: Vec::with_capacity(4096),
        }
    }

    async fn read_frame(&mut self) -> Result<Option<Vec<u8>>, BoxError> {
        loop {
            self.discard_keepalive_prefix();
            if let Some(frame_length) = complete_frame_length(&self.buffer)? {
                return Ok(Some(self.buffer.drain(..frame_length).collect()));
            }
            if self.buffer.len() >= MAX_SIP_MESSAGE_BYTES {
                return Err("SIP TCP 消息超过 1 MiB 限制".into());
            }
            let read = self.reader.read_buf(&mut self.buffer).await?;
            if read == 0 {
                if self.buffer.is_empty() {
                    return Ok(None);
                }
                return Err("SIP TCP 连接在完整消息前关闭".into());
            }
        }
    }

    fn discard_keepalive_prefix(&mut self) {
        while self.buffer.starts_with(b"\r\n") {
            self.buffer.drain(..2);
        }
    }
}

fn complete_frame_length(buffer: &[u8]) -> Result<Option<usize>, BoxError> {
    let Some((header_end, delimiter_length)) = find_header_end(buffer) else {
        return Ok(None);
    };
    let headers = std::str::from_utf8(&buffer[..header_end])?;
    let content_length = headers
        .lines()
        .skip(1)
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            (name.trim().eq_ignore_ascii_case("content-length")
                || name.trim().eq_ignore_ascii_case("l"))
            .then(|| value.trim())
        })
        .map(str::parse::<usize>)
        .transpose()?
        .unwrap_or(0);
    let frame_length = header_end
        .checked_add(delimiter_length)
        .and_then(|length| length.checked_add(content_length))
        .ok_or("SIP TCP 消息长度溢出")?;
    if frame_length > MAX_SIP_MESSAGE_BYTES {
        return Err("SIP TCP 消息超过 1 MiB 限制".into());
    }
    Ok((buffer.len() >= frame_length).then_some(frame_length))
}

fn find_header_end(buffer: &[u8]) -> Option<(usize, usize)> {
    buffer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|position| (position, 4))
        .or_else(|| {
            buffer
                .windows(2)
                .position(|window| window == b"\n\n")
                .map(|position| (position, 2))
        })
}

#[cfg(test)]
mod tests {
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
        let config = RouterConfig {
            udp_bind: router_addr.to_string(),
            tcp_bind: router_addr.to_string(),
            advertised_addr: "router.test:5070".to_string(),
            redis_url: "redis://127.0.0.1/0".to_string(),
            node_key_prefix: "test".to_string(),
            discovery_interval_secs: 1,
            transaction_ttl_secs: 64,
            dialog_route_ttl_secs: 60,
        };
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
}
