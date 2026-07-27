//! VCI instruction dispatcher and simple instruction handlers.

mod media;
mod queue;

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use call_core::{CallEvent, VciInstruction, WebhookEvent, WEBHOOK_SCHEMA_VERSION};
use tracing::info;

use crate::config::EdgeConfig;
use crate::edge_state::EdgeState;

use super::{execute_instruction, post_webhook_event};

/// Dispatch a VciInstruction to the appropriate handler.
pub(super) async fn dispatch_instruction(
    instruction: VciInstruction,
    call_id: &str,
    edge_state: &Arc<EdgeState>,
    edge_config: &EdgeConfig,
) {
    match instruction {
        VciInstruction::Dial {
            targets,
            sim_ring: _,
            caller_id,
            timeout_secs,
            record_call: _,
        } => {
            execute_dial(
                call_id,
                edge_state,
                edge_config,
                targets,
                caller_id,
                timeout_secs,
            )
            .await;
        }
        VciInstruction::Play { url, loop_count } => {
            media::execute_play(call_id, edge_state, edge_config, url, loop_count).await;
        }
        VciInstruction::Gather {
            play_url,
            max_digits,
            timeout_ms,
            inter_digit_timeout_ms: _,
            finish_on_key: _,
            barge_in: _,
        } => {
            media::execute_gather(
                call_id,
                edge_state,
                edge_config,
                play_url,
                max_digits,
                timeout_ms,
            )
            .await;
        }
        VciInstruction::Hangup {
            reason_code: _,
            sip_cause,
        } => {
            execute_hangup(call_id, edge_state, edge_config, sip_cause).await;
        }
        VciInstruction::Stream {
            websocket_url,
            format,
            barge_in,
        } => {
            media::execute_stream(
                call_id,
                edge_state,
                edge_config,
                websocket_url,
                format,
                barge_in,
            )
            .await;
        }
        VciInstruction::Record {
            max_length_secs,
            play_beep,
            trim_silence: _,
            silence_threshold_db: _,
        } => {
            media::execute_record(call_id, edge_state, edge_config, max_length_secs, play_beep)
                .await;
        }
        VciInstruction::Say {
            text,
            voice,
            speed: _,
            pitch: _,
        } => {
            media::execute_say(call_id, edge_state, edge_config, text, voice).await;
        }
        VciInstruction::Queue {
            queue_id: _,
            moh_url,
            priority: _,
        } => {
            queue::execute_queue(call_id, edge_state, edge_config, moh_url).await;
        }
        VciInstruction::Conference {
            room_id,
            start_muted: _,
            end_on_exit: _,
            max_participants: _,
        } => {
            queue::execute_conference(call_id, edge_state, edge_config, room_id).await;
        }
        VciInstruction::Redirect { url } => {
            execute_redirect(call_id, edge_state, edge_config, url).await;
        }
        VciInstruction::Pause { duration_ms } => {
            execute_pause(call_id, edge_state, edge_config, duration_ms).await;
        }
        VciInstruction::PlayDigits {
            digits,
            duration_ms: _,
        } => {
            media::execute_play_digits(call_id, edge_state, edge_config, digits).await;
        }
    }
}

async fn execute_dial(
    call_id: &str,
    edge_state: &Arc<EdgeState>,
    edge_config: &EdgeConfig,
    targets: Vec<String>,
    caller_id: Option<String>,
    timeout_secs: Option<u32>,
) {
    let target_uri = targets.first().cloned();
    let cmd = crate::sip::handlers::command_listener::CallCommand {
        call_id: call_id.to_string(),
        action: crate::sip::handlers::command_listener::CommandAction::Dial {
            params: crate::sip::handlers::command_listener::DialParams {
                target_gateway: None,
                target_uri,
                caller_id,
                timeout_secs,
            },
        },
    };
    crate::sip::handlers::command_listener::handle_command(cmd, edge_state, edge_config).await;
}

async fn execute_hangup(
    call_id: &str,
    edge_state: &Arc<EdgeState>,
    edge_config: &EdgeConfig,
    sip_cause: Option<u16>,
) {
    let cmd = crate::sip::handlers::command_listener::CallCommand {
        call_id: call_id.to_string(),
        action: crate::sip::handlers::command_listener::CommandAction::Hangup {
            params: crate::sip::handlers::command_listener::HangupParams { sip_cause },
        },
    };
    crate::sip::handlers::command_listener::handle_command(cmd, edge_state, edge_config).await;
}

async fn execute_redirect(
    call_id: &str,
    edge_state: &Arc<EdgeState>,
    edge_config: &EdgeConfig,
    url: String,
) {
    info!(call_id, url, "VCI Redirect command received");
    let edge_state_clone = Arc::clone(edge_state);
    let mut edge_config_clone = edge_config.clone();

    if url.starts_with("nats://") || url.starts_with("vos_rs.") {
        edge_config_clone.webhooks.control_mode = "nats".to_string();
        edge_config_clone.webhooks.control_incoming_subject = url.clone();
    } else {
        edge_config_clone.webhooks.control_mode = "http".to_string();
        edge_config_clone.webhooks.endpoint_url = url.clone();
    }

    let cid = call_id.to_string();
    tokio::spawn(async move {
        let event = WebhookEvent {
            event_id: uuid::Uuid::new_v4().to_string(),
            schema_version: WEBHOOK_SCHEMA_VERSION.to_string(),
            call_id: cid.clone(),
            sequence: 1,
            occurred_at_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as i64,
            event: CallEvent::CallInitiated {
                caller: None,
                callee: None,
                direction: "inbound".to_string(),
                leg: "a_leg".to_string(),
            },
        };
        if let Some(next_inst) =
            post_webhook_event(&edge_state_clone, &edge_config_clone, &event).await
        {
            execute_instruction(
                next_inst,
                cid.clone(),
                Arc::clone(&edge_state_clone),
                edge_config_clone,
            )
            .await;
        }
    });
}

async fn execute_pause(
    call_id: &str,
    edge_state: &Arc<EdgeState>,
    edge_config: &EdgeConfig,
    duration_ms: u64,
) {
    let edge_state_clone = Arc::clone(edge_state);
    let edge_config_clone = edge_config.clone();
    let cid = call_id.to_string();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(duration_ms)).await;
        if !edge_state_clone.inbound_transactions.contains_key(&cid) {
            return;
        }
        let event = WebhookEvent {
            event_id: uuid::Uuid::new_v4().to_string(),
            schema_version: WEBHOOK_SCHEMA_VERSION.to_string(),
            call_id: cid.clone(),
            sequence: 4,
            occurred_at_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as i64,
            event: CallEvent::CallAnswered {
                sip_status: 200,
                leg: "a_leg".to_string(),
            },
        };
        if let Some(next_inst) =
            post_webhook_event(&edge_state_clone, &edge_config_clone, &event).await
        {
            execute_instruction(
                next_inst,
                cid.clone(),
                Arc::clone(&edge_state_clone),
                edge_config_clone.clone(),
            )
            .await;
        }
    });
}
