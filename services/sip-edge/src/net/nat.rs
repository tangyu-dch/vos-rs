use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, info, warn};

use crate::cluster::MediaNodeType;

pub(crate) async fn run_stun_discovery(
    stun_server: &str,
    edge_config: &mut crate::config::EdgeConfig,
) {
    info!(server = %stun_server, "STUN discovery enabled");
    let Some(local_node) = edge_config
        .media_cluster
        .nodes
        .iter_mut()
        .find(|node| node.node_type == MediaNodeType::Local)
    else {
        warn!("STUN 仅适用于 local 媒体节点，当前配置将忽略 STUN");
        return;
    };
    let fallback = local_node.advertised_addr.clone();
    let public_ip = crate::net::stun_client::discover_stun_addr(Some(stun_server), &fallback).await;
    local_node.advertised_addr = public_ip.clone();
    edge_config.media.set_advertised_addr(public_ip);

    // Background STUN keepalive: reuse one socket for consistent NAT mapping
    let stun_server_clone = stun_server.to_string();
    tokio::spawn(async move {
        let server_addr = match tokio::net::lookup_host(&stun_server_clone).await {
            Ok(mut addrs) => match addrs.next() {
                Some(a) => a,
                None => {
                    warn!("STUN keepalive: DNS lookup failed, stopping");
                    return;
                }
            },
            Err(e) => {
                warn!(error = %e, "STUN keepalive: DNS lookup failed, stopping");
                return;
            }
        };
        let sock = match tokio::net::UdpSocket::bind("0.0.0.0:0").await {
            Ok(s) => s,
            Err(e) => {
                warn!(error = %e, "STUN keepalive: bind failed, stopping");
                return;
            }
        };
        let _ = sock.connect(server_addr).await;
        let mut interval = tokio::time::interval(Duration::from_secs(30));
        interval.tick().await;
        loop {
            interval.tick().await;
            // Minimal STUN Binding Request: 20 bytes
            let mut req = [0u8; 20];
            req[0] = 0x00;
            req[1] = 0x01; // BINDING
            req[2] = 0x00;
            req[3] = 0x08; // length = 8
            req[4] = 0x21;
            req[5] = 0x12;
            req[6] = 0xa4;
            req[7] = 0x42; // magic cookie
            let _ = sock.send(&req).await;
            let mut buf = [0u8; 1500];
            let _ = tokio::time::timeout(Duration::from_secs(3), sock.recv(&mut buf)).await;
            debug!("STUN keepalive sent");
        }
    });
}

pub(crate) fn run_upnp_port_mapping(bind_addr: &str, edge_config: &crate::config::EdgeConfig) {
    info!("UPnP port mapping enabled, discovering gateway...");
    if let Some(gw) = crate::net::upnp::discover_gateway() {
        if let Some(ext_ip) = crate::net::upnp::get_external_ip(&gw) {
            info!(external_ip = %ext_ip, "UPnP: router external IP");

            // Map SIP UDP port (5060)
            let sip_port: u16 = bind_addr
                .parse::<SocketAddr>()
                .map(|a| a.port())
                .unwrap_or(5060);
            crate::net::upnp::add_port_mapping(
                &gw,
                sip_port,
                sip_port,
                "UDP",
                "sip-edge SIP UDP",
                3600,
            );
            crate::net::upnp::add_port_mapping(
                &gw,
                sip_port,
                sip_port,
                "TCP",
                "sip-edge SIP TCP",
                3600,
            );

            // Map RTP port range
            let Some(local_node) = edge_config
                .media_cluster
                .nodes
                .iter()
                .find(|node| node.node_type == MediaNodeType::Local)
            else {
                warn!("UPnP RTP 映射仅适用于 local 媒体节点");
                return;
            };
            let rtp_min = local_node.port_min;
            let rtp_max = local_node.port_max;
            for port in (rtp_min..=rtp_max).step_by(2) {
                crate::net::upnp::add_port_mapping(&gw, port, port, "UDP", "sip-edge RTP", 3600);
            }

            // Periodic UPnP renewal (every 30 minutes, lease is 3600s = 1h)
            let gw_clone = crate::net::upnp::UpnpGateway {
                control_url: gw.control_url.clone(),
                local_ip: gw.local_ip.clone(),
                service_type: gw.service_type.clone(),
            };
            let sip_port_renew = sip_port;
            let rtp_min_renew = rtp_min;
            let rtp_max_renew = rtp_max;
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(Duration::from_secs(1800));
                interval.tick().await;
                loop {
                    interval.tick().await;
                    crate::net::upnp::add_port_mapping(
                        &gw_clone,
                        sip_port_renew,
                        sip_port_renew,
                        "UDP",
                        "sip-edge SIP UDP",
                        3600,
                    );
                    crate::net::upnp::add_port_mapping(
                        &gw_clone,
                        sip_port_renew,
                        sip_port_renew,
                        "TCP",
                        "sip-edge SIP TCP",
                        3600,
                    );
                    for port in (rtp_min_renew..=rtp_min_renew.min(rtp_max_renew)).step_by(2) {
                        crate::net::upnp::add_port_mapping(
                            &gw_clone,
                            port,
                            port,
                            "UDP",
                            "sip-edge RTP",
                            3600,
                        );
                    }
                    debug!("UPnP: port mappings renewed");
                }
            });
        }
    } else {
        warn!("UPnP: no gateway found on network, port mapping disabled");
    }
}

/// 启动 TURN 中继客户端：解析配置、创建 ALLOCATE、启动后台 REFRESH 续约。
///
/// 返回 `Some(TurnClient)` 表示成功分配 relayed 地址；`None` 表示未配置或分配失败。
/// 调用方应将结果注入 `EdgeState` 供媒体路径使用。
pub(crate) async fn run_turn_bootstrap(
    edge_config: &crate::config::EdgeConfig,
) -> Option<Arc<crate::net::turn_client::TurnClient>> {
    let server_str = edge_config.turn_server.as_ref()?;
    let cleaned = server_str
        .trim_start_matches("turn:")
        .trim_start_matches("turns:");
    let server_addr: SocketAddr = match cleaned.parse() {
        Ok(addr) => addr,
        Err(_) => {
            // 尝试解析为 host:port
            let parts: Vec<&str> = cleaned.rsplitn(2, ':').collect();
            if parts.len() == 2 {
                match format!("{}:{}", parts[1], parts[0]).parse::<SocketAddr>() {
                    Ok(addr) => addr,
                    Err(_) => {
                        warn!(server = %server_str, "TURN: invalid server address format");
                        return None;
                    }
                }
            } else {
                warn!(server = %server_str, "TURN: server address missing port");
                return None;
            }
        }
    };

    let username = edge_config.turn_username.clone().unwrap_or_default();
    let password = edge_config.turn_password.clone().unwrap_or_default();
    let realm = edge_config.turn_realm.clone().unwrap_or_default();

    if username.is_empty() || password.is_empty() {
        warn!(
            server = %server_addr,
            "TURN: username or password missing, skipping allocation"
        );
        return None;
    }

    info!(server = %server_addr, realm = %realm, "TURN bootstrap enabled");

    let config = crate::net::turn_client::TurnClientConfig {
        server_addr,
        username,
        password,
        realm,
    };

    match crate::net::turn_client::TurnClient::new(config).await {
        Ok(client) => match client.allocate().await {
            Ok(allocation) => {
                info!(
                    relayed = %allocation.relayed_address,
                    mapped = %allocation.mapped_address,
                    lifetime = allocation.lifetime_secs,
                    "TURN allocation established"
                );
                Some(Arc::new(client))
            }
            Err(e) => {
                warn!(error = %e, server = %server_addr, "TURN allocation failed");
                None
            }
        },
        Err(e) => {
            warn!(error = %e, server = %server_addr, "TURN client creation failed");
            None
        }
    }
}
