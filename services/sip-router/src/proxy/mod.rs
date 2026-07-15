mod message;
mod transaction;

#[cfg(test)]
mod tests;

use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    net::SocketAddr,
    sync::Arc,
};

use tokio::net::UdpSocket;

use crate::{config::RouterConfig, discovery::SharedNodes, routes::DialogRouteStore};

use message::request_method;
pub(crate) use message::{
    add_router_via, header_value, remove_top_via, router_branch, top_via_branch,
};
use transaction::{forward_backend_packet, spawn_transaction_cleanup, store, Transactions};

const MAX_DATAGRAM_BYTES: usize = 65_535;

pub(crate) async fn run(
    config: RouterConfig,
    nodes: SharedNodes,
    routes: Arc<DialogRouteStore>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let socket = Arc::new(UdpSocket::bind(&config.udp_bind).await?);
    let transactions = Arc::new(Transactions::new());
    spawn_transaction_cleanup(Arc::clone(&transactions));
    tracing::info!(bind = %config.udp_bind, "原生 SIP UDP 路由器已启动");
    let mut buffer = vec![0_u8; MAX_DATAGRAM_BYTES];

    loop {
        let (length, source) = socket.recv_from(&mut buffer).await?;
        let packet = &buffer[..length];
        let result = if is_backend(source, &nodes).await {
            forward_backend_packet(&socket, packet, &transactions, Arc::clone(&routes)).await
        } else {
            forward_client_packet(
                &socket,
                packet,
                source,
                &config,
                &nodes,
                &routes,
                &transactions,
            )
            .await
        };
        if let Err(error) = result {
            tracing::warn!(%source, %error, "丢弃无法路由的 SIP UDP 数据报");
        }
    }
}

async fn forward_client_packet(
    socket: &UdpSocket,
    packet: &[u8],
    source: SocketAddr,
    config: &RouterConfig,
    nodes: &SharedNodes,
    routes: &DialogRouteStore,
    transactions: &Transactions,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let call_id = header_value(packet, &["call-id", "i"]).ok_or("SIP 请求缺少 Call-ID")?;
    let method = request_method(packet).ok_or("SIP 请求起始行无效")?;
    let snapshot = nodes.read().await;
    let backend = routes.resolve(call_id, &snapshot).await?;
    let branch = router_branch(packet, "UDP")?;
    let forwarded = add_router_via(packet, &config.advertised_addr, "UDP", &branch)?;
    store(
        transactions,
        branch,
        source,
        call_id,
        method,
        config.transaction_ttl_secs,
    );
    socket.send_to(&forwarded, backend.address).await?;
    Ok(())
}

async fn is_backend(source: SocketAddr, nodes: &SharedNodes) -> bool {
    nodes.read().await.iter().any(|node| node.address == source)
}

pub(crate) fn select_node<'a>(
    call_id: &str,
    nodes: &'a [crate::discovery::SipNode],
) -> Option<&'a crate::discovery::SipNode> {
    nodes.iter().max_by_key(|node| {
        let mut hasher = DefaultHasher::new();
        call_id.hash(&mut hasher);
        node.id.hash(&mut hasher);
        hasher.finish()
    })
}
