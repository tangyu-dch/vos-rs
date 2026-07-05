use std::net::SocketAddr;
use std::sync::Arc;
use stun::agent::TransactionId;
use stun::client::*;
use stun::message::*;
use stun::xoraddr::*;
use tokio::net::UdpSocket;
use tracing::{info, warn};

pub struct StunClient {
    server_host: String,
    server_port: u16,
    refresh_interval_secs: u64,
}

impl StunClient {
    pub fn new(server: &str, refresh_interval_secs: u64) -> Option<Self> {
        let (host, port) = if let Some((h, p)) = server.rsplit_once(':') {
            let port: u16 = p.parse().ok()?;
            (h.to_string(), port)
        } else {
            (server.to_string(), stun::DEFAULT_PORT)
        };
        Some(Self {
            server_host: host,
            server_port: port,
            refresh_interval_secs,
        })
    }

    pub fn refresh_interval(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.refresh_interval_secs)
    }

    pub async fn discover_public_addr(&self) -> Option<SocketAddr> {
        let server_addr = match tokio::net::lookup_host(&format!("{}:{}", self.server_host, self.server_port)).await {
            Ok(mut addrs) => addrs.next()?,
            Err(e) => {
                warn!(error = %e, host = %self.server_host, "STUN DNS lookup failed");
                return None;
            }
        };

        let conn = UdpSocket::bind("0.0.0.0:0").await.ok()?;
        conn.connect(server_addr).await.ok()?;

        let (handler_tx, mut handler_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut client = ClientBuilder::new()
            .with_conn(Arc::new(conn))
            .build()
            .ok()?;

        let mut msg = Message::new();
        msg.build(&[
            Box::<TransactionId>::default(),
            Box::new(BINDING_REQUEST),
        ])
        .ok()?;

        client
            .send(&msg, Some(Arc::new(handler_tx)))
            .await
            .ok()?;

        let result = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            handler_rx.recv(),
        )
        .await;

        client.close().await.ok();

        match result {
            Ok(Some(event)) => {
                let resp = event.event_body.ok()?;
                let mut xor_addr = XorMappedAddress::default();
                xor_addr.get_from(&resp).ok()?;
                let public_addr = SocketAddr::new(xor_addr.ip, xor_addr.port);
                info!(
                    server = %self.server_host,
                    public_addr = %public_addr,
                    "STUN binding: discovered public address"
                );
                Some(public_addr)
            }
            _ => {
                warn!(server = %self.server_host, "STUN timeout or no response");
                None
            }
        }
    }
}

pub async fn discover_stun_addr(stun_server: Option<&str>, fallback_addr: &str) -> String {
    let Some(server) = stun_server else {
        return fallback_addr.to_string();
    };

    let client = match StunClient::new(server, 30) {
        Some(c) => c,
        None => {
            warn!(server = %server, "invalid STUN server address, using fallback");
            return fallback_addr.to_string();
        }
    };

    match client.discover_public_addr().await {
        Some(addr) => {
            info!(public_addr = %addr, "STUN: discovered public address for media relay");
            addr.ip().to_string()
        }
        None => {
            warn!("STUN discovery failed, using fallback advertised address");
            fallback_addr.to_string()
        }
    }
}
