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

pub(crate) struct BufferPool {
    pool: std::sync::Mutex<Vec<Vec<u8>>>,
    buf_size: usize,
}

impl BufferPool {
    pub fn new(capacity: usize, buf_size: usize) -> Self {
        let mut pool = Vec::with_capacity(capacity);
        for _ in 0..capacity {
            pool.push(vec![0; buf_size]);
        }
        Self {
            pool: std::sync::Mutex::new(pool),
            buf_size,
        }
    }

    pub fn acquire(&self) -> Vec<u8> {
        if let Ok(mut pool) = self.pool.lock() {
            if let Some(buf) = pool.pop() {
                return buf;
            }
        }
        vec![0; self.buf_size]
    }

    pub fn release(&self, mut buf: Vec<u8>) {
        if buf.capacity() < self.buf_size {
            buf.resize(self.buf_size, 0);
        }
        if let Ok(mut pool) = self.pool.lock() {
            if pool.len() < pool.capacity() {
                pool.push(buf);
            }
        }
    }
}

pub(crate) struct PooledBuffer {
    data: Vec<u8>,
    pool: Arc<BufferPool>,
}

impl PooledBuffer {
    pub fn new(data: Vec<u8>, pool: Arc<BufferPool>) -> Self {
        Self { data, pool }
    }
}

impl std::ops::Deref for PooledBuffer {
    type Target = [u8];
    fn deref(&self) -> &Self::Target {
        &self.data
    }
}

impl std::ops::DerefMut for PooledBuffer {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.data
    }
}

impl Drop for PooledBuffer {
    fn drop(&mut self) {
        let mut buf = std::mem::take(&mut self.data);
        buf.resize(self.pool.buf_size, 0);
        self.pool.release(buf);
    }
}

struct Datagram {
    bytes: PooledBuffer,
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
    let pool_capacity = (config.udp_workers * config.udp_queue_capacity).min(2048) + 128;
    let buffer_pool = Arc::new(BufferPool::new(pool_capacity, MAX_DATAGRAM_BYTES));

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

    loop {
        let mut raw_buf = buffer_pool.acquire();
        match socket.recv_from(&mut raw_buf).await {
            Ok((length, source)) => {
                metrics::udp_received();
                let snapshot = crate::discovery::snapshot(&nodes);
                let trusted_backend = snapshot.iter().any(|node| node.address == source);
                if !guard.allow(source.ip(), trusted_backend) {
                    metrics::udp_dropped();
                    buffer_pool.release(raw_buf);
                    continue;
                }
                raw_buf.truncate(length);
                let bytes = PooledBuffer::new(raw_buf, Arc::clone(&buffer_pool));
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
            Err(error) => {
                buffer_pool.release(raw_buf);
                return Err(error.into());
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
                let mut write_buf = Vec::with_capacity(MAX_DATAGRAM_BYTES);
                while let Some(datagram) = receiver.recv().await {
                    write_buf.clear();
                    if let Err(error) =
                        process_packet(&socket, &datagram, &config, &routes, &transactions, &mut write_buf).await
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
    write_buf: &mut Vec<u8>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if datagram.trusted_backend {
        forward_backend_packet(socket, &datagram.bytes, transactions, routes, write_buf).await
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
            write_buf,
        )
        .await
    }
}

#[allow(clippy::too_many_arguments)]
async fn forward_client_packet(
    socket: &UdpSocket,
    packet: &[u8],
    source: SocketAddr,
    config: &RouterConfig,
    nodes: &[crate::discovery::SipNode],
    routes: &DialogRouteStore,
    transactions: &Transactions,
    write_buf: &mut Vec<u8>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let call_id = header_value(packet, &["call-id", "i"]).ok_or("SIP 请求缺少 Call-ID")?;
    let method = request_method(packet).ok_or("SIP 请求起始行无效")?;
    let backend = routes.resolve(call_id, nodes).await?;
    let branch = router_branch(packet, "UDP")?;
    add_router_via(packet, &config.advertised_addr, "UDP", &branch, write_buf)?;
    store(
        transactions,
        branch,
        source,
        call_id,
        method,
        config.transaction_ttl_secs,
        config.max_transactions,
    )?;
    socket.send_to(write_buf, backend.address).await?;
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
