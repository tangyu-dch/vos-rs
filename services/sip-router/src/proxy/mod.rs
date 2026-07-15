mod message;
mod transaction;

#[cfg(test)]
mod tests;

use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    net::SocketAddr,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

use tokio::{net::UdpSocket, sync::mpsc};

use crate::{
    config::RouterConfig, discovery::SharedNodes, metrics, routes::DialogRouteStore,
    security::RouterGuard,
};

use message::request_method;
pub(crate) use message::{
    add_router_via, header_value, remove_top_via, router_branch, top_via_branch,
};
use transaction::{forward_backend_packet, spawn_transaction_cleanup, store, Transactions};

const MAX_DATAGRAM_BYTES: usize = 65_535;
static UDP_QUEUE_DROPS: AtomicU64 = AtomicU64::new(0);

struct Datagram {
    bytes: Vec<u8>,
    source: SocketAddr,
    trusted_backend: bool,
    nodes: Option<Arc<[crate::discovery::SipNode]>>,
}

pub(crate) async fn run(
    config: RouterConfig,
    nodes: SharedNodes,
    routes: Arc<DialogRouteStore>,
    guard: Arc<RouterGuard>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let socket = Arc::new(UdpSocket::bind(&config.udp_bind).await?);
    let transactions = Arc::new(Transactions::new());
    spawn_transaction_cleanup(Arc::clone(&transactions));
    let workers = spawn_workers(
        &config,
        Arc::clone(&socket),
        Arc::clone(&routes),
        Arc::clone(&transactions),
    );
    tracing::info!(
        bind = %config.udp_bind,
        workers = config.udp_workers,
        queue_capacity = config.udp_queue_capacity,
        "原生 SIP UDP 路由器已启动"
    );
    let mut buffer = vec![0_u8; MAX_DATAGRAM_BYTES];

    loop {
        let (length, source) = socket.recv_from(&mut buffer).await?;
        metrics::udp_received();
        let snapshot = crate::discovery::snapshot(&nodes);
        let trusted_backend = snapshot.iter().any(|node| node.address == source);
        if !guard.allow(source.ip(), trusted_backend) {
            metrics::udp_dropped();
            continue;
        }
        let bytes = buffer[..length].to_vec();
        let index = worker_index(&bytes, source, workers.len());
        if workers[index]
            .try_send(Datagram {
                bytes,
                source,
                trusted_backend,
                nodes: (!trusted_backend).then_some(snapshot),
            })
            .is_err()
        {
            let dropped = UDP_QUEUE_DROPS.fetch_add(1, Ordering::Relaxed) + 1;
            metrics::udp_dropped();
            if dropped == 1 || dropped.is_multiple_of(1000) {
                tracing::warn!(
                    worker = index,
                    %source,
                    dropped,
                    "SIP UDP worker 队列已满，丢弃数据报"
                );
            }
        }
    }
}

fn spawn_workers(
    config: &RouterConfig,
    socket: Arc<UdpSocket>,
    routes: Arc<DialogRouteStore>,
    transactions: Arc<Transactions>,
) -> Vec<mpsc::Sender<Datagram>> {
    (0..config.udp_workers)
        .map(|worker| {
            let (sender, mut receiver) = mpsc::channel::<Datagram>(config.udp_queue_capacity);
            let socket = Arc::clone(&socket);
            let routes = Arc::clone(&routes);
            let transactions = Arc::clone(&transactions);
            let config = config.clone();
            tokio::spawn(async move {
                while let Some(datagram) = receiver.recv().await {
                    if let Err(error) =
                        process_packet(&socket, &datagram, &config, &routes, &transactions).await
                    {
                        metrics::udp_error();
                        tracing::warn!(
                            worker,
                            source = %datagram.source,
                            %error,
                            "丢弃无法路由的 SIP UDP 数据报"
                        );
                    }
                }
            });
            sender
        })
        .collect()
}

async fn process_packet(
    socket: &UdpSocket,
    datagram: &Datagram,
    config: &RouterConfig,
    routes: &Arc<DialogRouteStore>,
    transactions: &Transactions,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if datagram.trusted_backend {
        forward_backend_packet(socket, &datagram.bytes, transactions, routes).await
    } else {
        let snapshot = datagram
            .nodes
            .as_deref()
            .ok_or("SIP 客户端数据报缺少节点快照")?;
        forward_client_packet(
            socket,
            &datagram.bytes,
            datagram.source,
            config,
            snapshot,
            routes,
            transactions,
        )
        .await
    }
}

async fn forward_client_packet(
    socket: &UdpSocket,
    packet: &[u8],
    source: SocketAddr,
    config: &RouterConfig,
    nodes: &[crate::discovery::SipNode],
    routes: &DialogRouteStore,
    transactions: &Transactions,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let call_id = header_value(packet, &["call-id", "i"]).ok_or("SIP 请求缺少 Call-ID")?;
    let method = request_method(packet).ok_or("SIP 请求起始行无效")?;
    let backend = routes.resolve(call_id, nodes).await?;
    let branch = router_branch(packet, "UDP")?;
    let forwarded = add_router_via(packet, &config.advertised_addr, "UDP", &branch)?;
    store(
        transactions,
        branch,
        source,
        call_id,
        method,
        config.transaction_ttl_secs,
        config.max_transactions,
    )?;
    socket.send_to(&forwarded, backend.address).await?;
    metrics::udp_routed();
    Ok(())
}

fn worker_index(packet: &[u8], source: SocketAddr, workers: usize) -> usize {
    let mut hasher = DefaultHasher::new();
    if let Some(call_id) = header_value(packet, &["call-id", "i"]) {
        call_id.hash(&mut hasher);
    } else {
        source.hash(&mut hasher);
    }
    hasher.finish() as usize % workers.max(1)
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
