use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    net::SocketAddr,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use dashmap::DashMap;
use tokio::net::UdpSocket;

use crate::{config::RouterConfig, discovery::SharedNodes, routes::DialogRouteStore};

const MAX_DATAGRAM_BYTES: usize = 65_535;
static NEXT_BRANCH_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone)]
struct TransactionRoute {
    client: SocketAddr,
    expires_at: Instant,
}

pub(crate) async fn run(
    config: RouterConfig,
    nodes: SharedNodes,
    routes: Arc<DialogRouteStore>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let socket = Arc::new(UdpSocket::bind(&config.udp_bind).await?);
    let transactions = Arc::new(DashMap::<String, TransactionRoute>::new());
    spawn_transaction_cleanup(Arc::clone(&transactions));
    tracing::info!(bind = %config.udp_bind, "原生 SIP UDP 路由器已启动");
    let mut buffer = vec![0_u8; MAX_DATAGRAM_BYTES];

    loop {
        let (length, source) = socket.recv_from(&mut buffer).await?;
        let packet = &buffer[..length];
        let result = if is_backend(source, &nodes).await {
            forward_backend_packet(&socket, packet, &transactions).await
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

fn spawn_transaction_cleanup(transactions: Arc<DashMap<String, TransactionRoute>>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(5));
        interval.tick().await;
        loop {
            interval.tick().await;
            let now = Instant::now();
            transactions.retain(|_, route| route.expires_at > now);
        }
    });
}

async fn forward_client_packet(
    socket: &UdpSocket,
    packet: &[u8],
    source: SocketAddr,
    config: &RouterConfig,
    nodes: &SharedNodes,
    routes: &DialogRouteStore,
    transactions: &DashMap<String, TransactionRoute>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let call_id = header_value(packet, &["call-id", "i"]).ok_or("SIP 请求缺少 Call-ID")?;
    let snapshot = nodes.read().await;
    let backend = routes.resolve(call_id, &snapshot).await?;
    let branch = format!(
        "z9hG4bK-vosrs-{:016x}",
        NEXT_BRANCH_ID.fetch_add(1, Ordering::Relaxed)
    );
    let forwarded = add_router_via(packet, &config.advertised_addr, "UDP", &branch)?;
    transactions.insert(
        branch,
        TransactionRoute {
            client: source,
            expires_at: Instant::now() + Duration::from_secs(config.transaction_ttl_secs),
        },
    );
    socket.send_to(&forwarded, backend.address).await?;
    Ok(())
}

async fn forward_backend_packet(
    socket: &UdpSocket,
    packet: &[u8],
    transactions: &DashMap<String, TransactionRoute>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let branch = top_via_branch(packet).ok_or("SIP 响应缺少路由器 Via branch")?;
    let route = transactions.get(&branch).ok_or("SIP 响应事务路由已过期")?;
    let forwarded = remove_top_via(packet)?;
    socket.send_to(&forwarded, route.client).await?;
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

pub(crate) fn add_router_via(
    packet: &[u8],
    advertised_addr: &str,
    transport: &str,
    branch: &str,
) -> Result<Vec<u8>, &'static str> {
    let split = packet
        .iter()
        .position(|byte| *byte == b'\n')
        .ok_or("SIP 起始行不完整")?
        + 1;
    let via = format!("Via: SIP/2.0/{transport} {advertised_addr};branch={branch};rport\r\n");
    let mut output = Vec::with_capacity(packet.len() + via.len());
    output.extend_from_slice(&packet[..split]);
    output.extend_from_slice(via.as_bytes());
    output.extend_from_slice(&packet[split..]);
    Ok(output)
}

pub(crate) fn top_via_branch(packet: &[u8]) -> Option<String> {
    header_value(packet, &["via", "v"]).and_then(|via| parameter(via, "branch"))
}

pub(crate) fn header_value<'a>(packet: &'a [u8], accepted_names: &[&str]) -> Option<&'a str> {
    let text = std::str::from_utf8(packet).ok()?;
    text.lines().skip(1).find_map(|line| {
        if line.trim().is_empty() {
            return None;
        }
        let (name, value) = line.split_once(':')?;
        accepted_names
            .iter()
            .any(|accepted| name.trim().eq_ignore_ascii_case(accepted))
            .then(|| value.trim())
    })
}

fn parameter(value: &str, name: &str) -> Option<String> {
    value.split(';').skip(1).find_map(|part| {
        let (key, value) = part.trim().split_once('=')?;
        key.eq_ignore_ascii_case(name)
            .then(|| value.trim().to_string())
    })
}

pub(crate) fn remove_top_via(packet: &[u8]) -> Result<Vec<u8>, &'static str> {
    let text = std::str::from_utf8(packet).map_err(|_| "SIP 响应不是 UTF-8")?;
    let line_start = text.find('\n').ok_or("SIP 起始行不完整")? + 1;
    let relative_end = text[line_start..].find('\n').ok_or("Via 行不完整")? + 1;
    let line_end = line_start + relative_end;
    let first_header = text[line_start..line_end].trim_start();
    if !first_header.to_ascii_lowercase().starts_with("via:")
        && !first_header.to_ascii_lowercase().starts_with("v:")
    {
        return Err("路由器 Via 不是首个响应头");
    }
    let mut output = Vec::with_capacity(packet.len() - (line_end - line_start));
    output.extend_from_slice(&packet[..line_start]);
    output.extend_from_slice(&packet[line_end..]);
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discovery::SipNode;

    const REQUEST: &[u8] = b"OPTIONS sip:test@example.com SIP/2.0\r\nVia: SIP/2.0/UDP client;branch=z9hG4bK-client\r\nCall-ID: call-1\r\nCSeq: 1 OPTIONS\r\nContent-Length: 0\r\n\r\n";

    #[test]
    fn test_add_and_remove_router_via_round_trips_packet() {
        let forwarded =
            add_router_via(REQUEST, "router:5060", "UDP", "z9hG4bK-router").expect("add Via");
        assert_eq!(
            top_via_branch(&forwarded).as_deref(),
            Some("z9hG4bK-router")
        );
        assert_eq!(remove_top_via(&forwarded).expect("remove Via"), REQUEST);
    }

    #[test]
    fn test_select_node_is_stable_for_call_id() {
        let nodes = vec![
            SipNode {
                id: "a".to_string(),
                address: "127.0.0.1:5061".parse().expect("address"),
            },
            SipNode {
                id: "b".to_string(),
                address: "127.0.0.1:5062".parse().expect("address"),
            },
        ];
        assert_eq!(select_node("call-1", &nodes), select_node("call-1", &nodes));
    }
}
