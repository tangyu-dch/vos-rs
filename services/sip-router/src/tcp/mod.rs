mod framing;
mod pool;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use tokio::{
    io::AsyncWriteExt,
    net::{TcpListener, TcpStream},
    sync::{mpsc, Semaphore},
};

use crate::{
    config::RouterConfig,
    discovery::SharedNodes,
    metrics,
    proxy::{add_router_via, header_value, router_branch},
    routes::DialogRouteStore,
    security::RouterGuard,
};
use framing::SipFrameReader;
use pool::BackendPool;

type BoxError = Box<dyn std::error::Error + Send + Sync>;

pub(crate) async fn run(
    config: RouterConfig,
    nodes: SharedNodes,
    routes: Arc<DialogRouteStore>,
    guard: Arc<RouterGuard>,
) -> Result<(), BoxError> {
    let listener = TcpListener::bind(&config.tcp_bind).await?;
    let connection_limit = Arc::new(Semaphore::new(config.tcp_max_connections));
    tracing::info!(
        bind = %config.tcp_bind,
        max_connections = config.tcp_max_connections,
        "原生 SIP TCP 路由器已启动"
    );
    loop {
        let (stream, peer) = listener.accept().await?;
        if !guard.allow(peer.ip(), false) {
            metrics::tcp_rejected();
            drop(stream);
            continue;
        }
        let Ok(permit) = Arc::clone(&connection_limit).try_acquire_owned() else {
            tracing::warn!(%peer, "SIP TCP 连接数达到上限，拒绝新连接");
            metrics::tcp_rejected();
            drop(stream);
            continue;
        };
        let config = config.clone();
        let nodes = Arc::clone(&nodes);
        let routes = Arc::clone(&routes);
        tokio::spawn(async move {
            let _permit = permit;
            metrics::tcp_opened();
            if let Err(error) = handle_connection(stream, &config, &nodes, &routes).await {
                tracing::debug!(%peer, %error, "SIP TCP 路由连接已关闭");
            }
            metrics::tcp_closed();
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
    let (client_sender, mut client_receiver) = mpsc::channel(config.tcp_write_queue_capacity);
    let idle_timeout = std::time::Duration::from_secs(config.tcp_idle_timeout_secs);
    let connect_timeout = std::time::Duration::from_secs(config.tcp_connect_timeout_secs);
    let routing = async move {
        let mut pool = BackendPool::new(
            client_sender,
            config.tcp_write_queue_capacity,
            connect_timeout,
        );
        loop {
            let frame = tokio::time::timeout(idle_timeout, client_reader.read_frame())
                .await
                .map_err(|_| "SIP TCP 客户端连接空闲超时")??;
            let Some(frame) = frame else {
                break;
            };
            metrics::tcp_frame();
            let call_id =
                header_value(&frame, &["call-id", "i"]).ok_or("SIP TCP 消息缺少 Call-ID")?;
            let snapshot = crate::discovery::snapshot(nodes);
            let backend = routes.resolve(call_id, snapshot.as_ref()).await?;
            let forwarded = if is_response(&frame) {
                frame
            } else {
                add_tcp_via(&frame, &config.advertised_addr)?
            };
            pool.send(&backend, forwarded).await?;
        }
        Ok::<(), BoxError>(())
    };
    let writer = async move {
        while let Some(frame) = client_receiver.recv().await {
            client_write.write_all(&frame).await?;
        }
        Ok::<(), BoxError>(())
    };
    tokio::try_join!(routing, writer)?;
    Ok(())
}

fn add_tcp_via(frame: &[u8], advertised_addr: &str) -> Result<Vec<u8>, &'static str> {
    let branch = router_branch(frame, "TCP")?.replacen("z9hG4bK-vosrs-", "z9hG4bK-vosrs-tcp-", 1);
    let mut output = Vec::new();
    add_router_via(frame, advertised_addr, "TCP", &branch, &mut output)?;
    Ok(output)
}

fn is_response(frame: &[u8]) -> bool {
    frame.starts_with(b"SIP/2.0 ")
}
