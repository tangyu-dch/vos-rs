use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use stun::agent::TransactionId;
use stun::client::*;
use stun::message::*;
use stun::xoraddr::*;
use tokio::net::UdpSocket;
use tracing::{debug, info, warn};

pub struct StunClient {
    server_host: String,
    server_port: u16,
}

impl StunClient {
    pub fn new(server: &str) -> Option<Self> {
        let (host, port) = if let Some((h, p)) = server.rsplit_once(':') {
            let port: u16 = p.parse().ok()?;
            (h.to_string(), port)
        } else {
            (server.to_string(), stun::DEFAULT_PORT)
        };
        Some(Self {
            server_host: host,
            server_port: port,
        })
    }

    pub async fn discover_public_addr(&self) -> Option<SocketAddr> {
        self.discover_with_retry(3).await
    }

    async fn discover_with_retry(&self, max_retries: u32) -> Option<SocketAddr> {
        let server_addr =
            match tokio::net::lookup_host(&format!("{}:{}", self.server_host, self.server_port))
                .await
            {
                Ok(mut addrs) => addrs.next()?,
                Err(e) => {
                    warn!(error = %e, host = %self.server_host, "STUN DNS lookup failed");
                    return None;
                }
            };

        for attempt in 1..=max_retries {
            let result = self.try_binding(server_addr).await;
            if let Some(addr) = result {
                info!(
                    server = %self.server_host,
                    public_addr = %addr,
                    attempt,
                    "STUN binding: discovered public address"
                );
                return Some(addr);
            }
            if attempt < max_retries {
                let backoff = Duration::from_millis(500 * attempt as u64);
                debug!(
                    server = %self.server_host,
                    attempt,
                    backoff_ms = backoff.as_millis(),
                    "STUN retry after backoff"
                );
                tokio::time::sleep(backoff).await;
            }
        }
        warn!(server = %self.server_host, max_retries, "STUN: all retries exhausted");
        None
    }

    async fn try_binding(&self, server_addr: SocketAddr) -> Option<SocketAddr> {
        let conn = UdpSocket::bind("0.0.0.0:0").await.ok()?;
        conn.connect(server_addr).await.ok()?;

        let (handler_tx, mut handler_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut client = ClientBuilder::new()
            .with_conn(Arc::new(conn))
            .build()
            .ok()?;

        let mut msg = Message::new();
        msg.build(&[Box::<TransactionId>::default(), Box::new(BINDING_REQUEST)])
            .ok()?;

        client.send(&msg, Some(Arc::new(handler_tx))).await.ok()?;

        let result = tokio::time::timeout(Duration::from_secs(3), handler_rx.recv()).await;

        client.close().await.ok();

        match result {
            Ok(Some(event)) => {
                let resp = event.event_body.ok()?;
                let mut xor_addr = XorMappedAddress::default();
                xor_addr.get_from(&resp).ok()?;
                Some(SocketAddr::new(xor_addr.ip, xor_addr.port))
            }
            _ => None,
        }
    }
}

pub async fn discover_stun_addr(stun_servers: Option<&str>, fallback_addr: &str) -> String {
    let Some(servers_str) = stun_servers else {
        return fallback_addr.to_string();
    };

    let servers: Vec<&str> = servers_str
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();

    for server in &servers {
        if let Some(client) = StunClient::new(server) {
            if let Some(addr) = client.discover_public_addr().await {
                info!(public_addr = %addr, server = %server, "STUN: discovered public address for media relay");
                return addr.ip().to_string();
            }
        } else {
            warn!(server = %server, "STUN: invalid server address, skipping");
        }
    }

    if !fallback_addr.is_empty() {
        warn!("STUN: all servers failed, using fallback advertised address");
    }
    fallback_addr.to_string()
}
