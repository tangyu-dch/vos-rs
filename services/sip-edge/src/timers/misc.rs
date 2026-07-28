use crate::{config::EdgeConfig, edge_state::EdgeState};
use call_core::CallQualityMetrics;
use std::{
    sync::Arc,
    time::{Duration, SystemTime},
};
use tokio::net::UdpSocket;
use tracing::warn;

use crate::media;

pub(crate) fn spawn_nat_keepalive_loop(edge_state: Arc<EdgeState>, socket: Arc<UdpSocket>) {
    let scan_interval = if cfg!(test) {
        Duration::from_millis(50)
    } else {
        Duration::from_secs(30)
    };

    tokio::spawn(async move {
        let mut interval = tokio::time::interval(scan_interval);
        interval.tick().await;

        loop {
            interval.tick().await;

            let addrs = {
                let registrar = edge_state.registrar.read().await;
                registrar
                    .get_all_active_received_from(SystemTime::now(), None)
                    .await
            };

            for addr in addrs {
                edge_state.send_keepalive_probe(&addr, &socket).await;
            }
        }
    });
}

/// 周期性清理过期的 SUBSCRIBE/NOTIFY 订阅，并向已过期订阅发送 terminated NOTIFY。
///
/// 清理间隔：生产 60 秒，测试 50 毫秒。
pub(crate) fn spawn_subscription_prune_loop(
    edge_state: Arc<EdgeState>,
    socket: Arc<UdpSocket>,
    edge_config: Arc<EdgeConfig>,
) {
    let scan_interval = if cfg!(test) {
        Duration::from_millis(50)
    } else {
        Duration::from_secs(60)
    };

    tokio::spawn(async move {
        let mut interval = tokio::time::interval(scan_interval);
        interval.tick().await;

        loop {
            interval.tick().await;
            let expired = edge_state
                .subscription_store
                .prune_expired(SystemTime::now())
                .await;
            if expired.is_empty() {
                continue;
            }
            for subscription in expired {
                let notify = crate::sip::handlers::subscribe::build_notify(
                    &subscription,
                    "",
                    &crate::sip::subscription::SubscriptionState::Terminated {
                        reason: Some("timeout"),
                    },
                    &edge_config,
                );
                if let Err(error) = socket.send_to(&notify, subscription.peer).await {
                    warn!(%error, peer = %subscription.peer, "failed to send terminated NOTIFY");
                }
            }
        }
    });
}

pub(crate) fn calculate_mos_for_legs(
    caller_rtcp: Option<&media::RtcpQualitySnapshot>,
    gateway_rtcp: Option<&media::RtcpQualitySnapshot>,
) -> CallQualityMetrics {
    let mut metrics = CallQualityMetrics::default();

    let (caller_rtt, caller_loss, _caller_jitter) = if let Some(rtcp) = caller_rtcp {
        let rtt = rtcp.max_rtt_ms.or(rtcp.last_rtt_ms);
        let loss = rtcp
            .max_fraction_lost
            .or(rtcp.last_fraction_lost)
            .map(|f| (f64::from(f)) / 256.0 * 100.0);
        let jitter = rtcp
            .max_jitter
            .or(rtcp.last_jitter)
            .map(|j| (f64::from(j)) / 8.0);

        metrics.caller_rtt_ms = rtt;
        metrics.caller_loss_rate = loss;
        metrics.caller_jitter_ms = jitter;

        (rtt.unwrap_or(0), loss.unwrap_or(0.0), jitter.unwrap_or(0.0))
    } else {
        (0, 0.0, 0.0)
    };

    let (gateway_rtt, gateway_loss, _gateway_jitter) = if let Some(rtcp) = gateway_rtcp {
        let rtt = rtcp.max_rtt_ms.or(rtcp.last_rtt_ms);
        let loss = rtcp
            .max_fraction_lost
            .or(rtcp.last_fraction_lost)
            .map(|f| (f64::from(f)) / 256.0 * 100.0);
        let jitter = rtcp
            .max_jitter
            .or(rtcp.last_jitter)
            .map(|j| (f64::from(j)) / 8.0);

        metrics.gateway_rtt_ms = rtt;
        metrics.gateway_loss_rate = loss;
        metrics.gateway_jitter_ms = jitter;

        (rtt.unwrap_or(0), loss.unwrap_or(0.0), jitter.unwrap_or(0.0))
    } else {
        (0, 0.0, 0.0)
    };

    if caller_rtcp.is_none() && gateway_rtcp.is_none() {
        return metrics;
    }

    let d_caller = (f64::from(caller_rtt)) / 2.0;
    let d_gateway = (f64::from(gateway_rtt)) / 2.0;
    let d_total = d_caller + d_gateway;

    let i_d = if d_total < 177.3 {
        0.024 * d_total
    } else {
        0.024 * d_total + 0.11 * (d_total - 177.3)
    };

    let i_e_caller = 95.0 * (caller_loss / (caller_loss + 4.3));
    let i_e_gateway = 95.0 * (gateway_loss / (gateway_loss + 4.3));
    let i_e = i_e_caller + i_e_gateway;

    let r_factor = 93.2 - i_d - i_e;
    let r_factor = r_factor.clamp(0.0, 93.2);

    let mos = 1.0 + 0.035 * r_factor + 0.000007 * r_factor * (r_factor - 60.0) * (100.0 - r_factor);
    let mos = mos.clamp(1.0, 4.5);

    metrics.mos = Some(mos);
    metrics
}
