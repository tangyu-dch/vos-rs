use std::{
    net::SocketAddr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use dashmap::DashMap;
use tokio::net::UdpSocket;

use super::message::{remove_top_via, response_status, top_via_branch};
use crate::{metrics, routes::DialogRouteStore};

#[derive(Debug, Clone)]
pub(super) struct TransactionRoute {
    client: SocketAddr,
    call_id: String,
    method: String,
    expires_at: Instant,
    release_scheduled: Arc<AtomicBool>,
}

pub(super) type Transactions = DashMap<String, TransactionRoute>;

pub(super) fn store(
    transactions: &Transactions,
    branch: String,
    client: SocketAddr,
    call_id: &str,
    method: &str,
    ttl_secs: u64,
    max_transactions: usize,
) -> Result<(), &'static str> {
    if transactions.len() >= max_transactions && !transactions.contains_key(&branch) {
        return Err("SIP UDP 事务容量已满");
    }
    transactions.insert(
        branch,
        TransactionRoute {
            client,
            call_id: call_id.to_string(),
            method: method.to_string(),
            expires_at: Instant::now() + Duration::from_secs(ttl_secs),
            release_scheduled: Arc::new(AtomicBool::new(false)),
        },
    );
    metrics::active_transactions(transactions.len());
    Ok(())
}

pub(super) fn spawn_transaction_cleanup(transactions: Arc<Transactions>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(5));
        interval.tick().await;
        loop {
            interval.tick().await;
            let now = Instant::now();
            transactions.retain(|_, route| route.expires_at > now);
            metrics::active_transactions(transactions.len());
        }
    });
}

pub(super) async fn forward_backend_packet(
    socket: &UdpSocket,
    packet: &[u8],
    transactions: &Transactions,
    routes: &Arc<DialogRouteStore>,
    write_buf: &mut Vec<u8>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let branch = top_via_branch(packet).ok_or("SIP 响应缺少路由器 Via branch")?;
    let route = transactions.get(&branch).ok_or("SIP 响应事务路由已过期")?;
    remove_top_via(packet, write_buf)?;
    socket.send_to(write_buf, route.client).await?;
    if response_status(packet).is_some_and(|status| should_release(&route.method, status))
        && !route.release_scheduled.swap(true, Ordering::AcqRel)
    {
        let call_id = route.call_id.clone();
        let delay = route.expires_at.saturating_duration_since(Instant::now());
        drop(route);
        let routes = Arc::clone(routes);
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            routes.release(&call_id);
        });
    }
    Ok(())
}

pub(super) fn should_release(method: &str, status: u16) -> bool {
    if status < 200 {
        return false;
    }
    method.eq_ignore_ascii_case("BYE")
        || (method.eq_ignore_ascii_case("INVITE") && status >= 300)
        || ["OPTIONS", "REGISTER", "MESSAGE", "PUBLISH"]
            .iter()
            .any(|candidate| method.eq_ignore_ascii_case(candidate))
}
