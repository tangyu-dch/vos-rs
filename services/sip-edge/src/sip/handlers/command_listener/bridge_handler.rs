//! VCI Bridge 命令处理。
//!
//! 将两通已建立的呼叫在媒体层两两配对，让 RTP 双向直通。

use std::sync::Arc;

use tracing::{info, warn};

use crate::config::EdgeConfig;
use crate::edge_state::EdgeState;

/// 处理 Bridge 命令：将两个已建立呼叫的 RTP 端口两两配对，实现媒体直通。
pub(super) async fn handle_bridge(
    call_id: &str,
    call_id_a: String,
    call_id_b: String,
    edge_state: &Arc<EdgeState>,
    _edge_config: &EdgeConfig,
) {
    info!(
        call_id,
        "Executing Bridge command for {} and {}", call_id_a, call_id_b
    );

    let (port_a, rtp_a) = {
        if let Some(tx_a) = edge_state.inbound_transactions.get(&call_id_a) {
            let port = tx_a.caller_relay_rtp.as_ref().map(|ep| ep.port);
            let rtp = tx_a.caller_rtp.clone();
            (port, rtp)
        } else {
            (None, None)
        }
    };

    let (port_b, rtp_b) = {
        if let Some(tx_b) = edge_state.inbound_transactions.get(&call_id_b) {
            let port = tx_b.caller_relay_rtp.as_ref().map(|ep| ep.port);
            let rtp = tx_b.caller_rtp.clone();
            (port, rtp)
        } else {
            (None, None)
        }
    };

    match (port_a, port_b) {
        (Some(pa), Some(pb)) => {
            edge_state.media_relay.pair_ports(pa, pb);
            info!("Successfully paired ports: {} <-> {}", pa, pb);

            if let (Some(dest_b), Some(ep_a)) = (rtp_b, {
                edge_state
                    .inbound_transactions
                    .get(&call_id_a)
                    .and_then(|tx| tx.caller_relay_rtp.clone())
            }) {
                let _ = edge_state.media_relay.set_target(&ep_a, &dest_b);
            }
            if let (Some(dest_a), Some(ep_b)) = (rtp_a, {
                edge_state
                    .inbound_transactions
                    .get(&call_id_b)
                    .and_then(|tx| tx.caller_relay_rtp.clone())
            }) {
                let _ = edge_state.media_relay.set_target(&ep_b, &dest_a);
            }
        }
        _ => {
            warn!(
                "Failed to find caller relay ports for bridging: a_port={:?}, b_port={:?}",
                port_a, port_b
            );
        }
    }
}
