//! Queue-related VCI instruction handlers: Queue, Conference.

use std::net::SocketAddr;
use std::sync::Arc;

use call_core::VciInstruction;
use tracing::{info, warn};

use crate::config::EdgeConfig;
use crate::edge_state::EdgeState;

use super::super::execute_instruction;
use super::super::webhook::answer_parked_call_if_needed;

pub(super) async fn execute_queue(
    call_id: &str,
    edge_state: &Arc<EdgeState>,
    edge_config: &EdgeConfig,
    moh_url: String,
) {
    info!(
        call_id,
        moh_url, "VCI Queue command received, putting call in queue loop"
    );
    let cmd = VciInstruction::Play {
        url: moh_url,
        loop_count: 9999,
    };
    execute_instruction(
        cmd,
        call_id.to_string(),
        Arc::clone(edge_state),
        edge_config.clone(),
    )
    .await;
}

pub(super) async fn execute_conference(
    call_id: &str,
    edge_state: &Arc<EdgeState>,
    edge_config: &EdgeConfig,
    room_id: String,
) {
    let port = if edge_state.parked_calls.contains_key(call_id) {
        answer_parked_call_if_needed(
            call_id,
            edge_state,
            edge_config,
            "conference",
            "vosrs-conference-tag",
        )
        .await
    } else {
        edge_state
            .inbound_transactions
            .get(call_id)
            .and_then(|tx| tx.caller_relay_rtp.as_ref().map(|ep| ep.port))
    };
    if let Some(port) = port {
        let target_addr_opt = if let Some(tx) = edge_state.inbound_transactions.get(call_id) {
            if let Some(ref caller_rtp) = tx.caller_rtp {
                format!("{}:{}", caller_rtp.address, caller_rtp.port)
                    .parse::<SocketAddr>()
                    .ok()
            } else {
                None
            }
        } else {
            None
        };

        if let Some(target_addr) = target_addr_opt {
            let codec = edge_state
                .media_relay
                .codecs
                .get(&port)
                .map(|c| *c.value())
                .unwrap_or(rtp_core::AudioCodec::Pcma);
            if let Some(socket) = edge_state
                .media_relay
                .active_sockets
                .get(&port)
                .map(|s| s.value().clone())
            {
                let _ = edge_state
                    .media_relay
                    .conference_manager
                    .join_conference(&room_id, port, codec, target_addr, socket)
                    .await;
                info!(call_id, room_id, port, %target_addr, "successfully joined conference room");
            } else {
                warn!(call_id, "active socket not found for conference port");
            }
        } else {
            warn!(
                call_id,
                "failed to get caller target address for conference"
            );
        }
    }
}
