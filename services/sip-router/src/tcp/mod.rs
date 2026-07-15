mod framing;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use tokio::{
    io::AsyncWriteExt,
    net::{TcpListener, TcpStream},
};

use crate::{
    config::RouterConfig,
    discovery::SharedNodes,
    proxy::{add_router_via, header_value, remove_top_via, router_branch, top_via_branch},
    routes::DialogRouteStore,
};
use framing::SipFrameReader;

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
