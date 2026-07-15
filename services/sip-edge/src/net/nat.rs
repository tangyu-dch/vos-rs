use std::net::SocketAddr;
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
