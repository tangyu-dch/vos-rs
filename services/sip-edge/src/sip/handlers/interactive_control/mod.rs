//! Interactive Webhook Call Control (VCI) for HTTP control mode.

mod instructions;
mod webhook;

#[cfg(test)]
mod tests;

pub(crate) use webhook::{handle_interactive_webhook_call, post_webhook_event};

use call_core::VciInstruction;
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

use crate::config::EdgeConfig;
use crate::edge_state::EdgeState;

pub(crate) fn get_http_client() -> &'static reqwest::Client {
    static HTTP_CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    HTTP_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap()
    })
}

/// Execute a VciInstruction.
pub(crate) fn execute_instruction(
    instruction: VciInstruction,
    call_id: String,
    edge_state: Arc<EdgeState>,
    edge_config: EdgeConfig,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
    Box::pin(async move {
        let call_id = &call_id;
        let edge_state = &edge_state;
        let edge_config = &edge_config;
        info!(call_id, ?instruction, "executing VciInstruction");

        // Check if the call is still parked or active.
        let is_parked = edge_state.parked_calls.contains_key(call_id);
        let is_active = edge_state.inbound_transactions.contains_key(call_id);

        if !is_parked && !is_active {
            warn!(
                call_id,
                "call not found in parked or active calls, ignoring instruction"
            );
            return;
        }

        instructions::dispatch_instruction(instruction, call_id, edge_state, edge_config).await;
    })
}
