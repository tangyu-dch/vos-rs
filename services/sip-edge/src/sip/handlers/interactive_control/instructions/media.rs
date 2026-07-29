//! Media-related VCI instruction handlers: Play, Gather, Stream, Record, Say, PlayDigits.

use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use call_core::{CallEvent, WebhookEvent, WEBHOOK_SCHEMA_VERSION};
use tracing::{info, warn};

use crate::config::EdgeConfig;
use crate::edge_state::{EdgeState, PendingDatagram};

use super::super::webhook::answer_parked_call_if_needed;
use super::super::{execute_instruction, post_webhook_event};

pub(super) async fn execute_play(
    call_id: &str,
    edge_state: &Arc<EdgeState>,
    edge_config: &EdgeConfig,
    url: String,
    loop_count: u32,
) {
    let port = if edge_state.parked_calls.contains_key(call_id) {
        answer_parked_call_if_needed(
            call_id,
            edge_state,
            edge_config,
            "playback",
            "vosrs-playback-tag",
        )
        .await
    } else {
        edge_state
            .inbound_transactions
            .get(call_id)
            .and_then(|tx| tx.caller_relay_rtp.as_ref().map(|ep| ep.port))
    };

    if let Some(port) = port {
        let file_path = url.clone();
        let loop_playback = loop_count > 1;
        let _ = edge_state
            .media_relay
            .start_playback(
                port,
                std::path::PathBuf::from(file_path),
                crate::media::relay::PlaybackMode::Exclusive,
                loop_playback,
            )
            .await;

        // Spawn loop to monitor playback completion
        let edge_state_clone = Arc::clone(edge_state);
        let edge_config_clone = edge_config.clone();
        let cid = call_id.to_string();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(500));
            loop {
                interval.tick().await;
                if !edge_state_clone.inbound_transactions.contains_key(&cid) {
                    break;
                }
                if !edge_state_clone.media_relay.playbacks.contains_key(&port) {
                    info!(call_id = %cid, "playback completed, triggering callback");
                    let event = WebhookEvent {
                        event_id: uuid::Uuid::new_v4().to_string(),
                        schema_version: WEBHOOK_SCHEMA_VERSION.to_string(),
                        call_id: cid.clone(),
                        sequence: 2,
                        occurred_at_ms: SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as i64,
                        event: CallEvent::CallAnswered {
                            sip_status: 200,
                            leg: "b_leg".to_string(),
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
                    break;
                }
            }
        });
    }
}

pub(super) async fn execute_gather(
    call_id: &str,
    edge_state: &Arc<EdgeState>,
    edge_config: &EdgeConfig,
    play_url: Option<String>,
    max_digits: usize,
    timeout_ms: u64,
) {
    let port = if edge_state.parked_calls.contains_key(call_id) {
        answer_parked_call_if_needed(
            call_id,
            edge_state,
            edge_config,
            "gather",
            "vosrs-gather-tag",
        )
        .await
    } else {
        edge_state
            .inbound_transactions
            .get(call_id)
            .and_then(|tx| tx.caller_relay_rtp.as_ref().map(|ep| ep.port))
    };

    if let Some(port) = port {
        let Some(media_session_id) = edge_state
            .inbound_transactions
            .get(call_id)
            .map(|transaction| transaction.session_id.clone())
        else {
            warn!(call_id, "gather session disappeared before DTMF setup");
            return;
        };
        edge_state
            .media_relay
            .register_port_dtmf_tracking(&media_session_id, port, 101);

        if let Some(ref play_url) = play_url {
            let _ = edge_state
                .media_relay
                .start_playback(
                    port,
                    std::path::PathBuf::from(play_url.clone()),
                    crate::media::relay::PlaybackMode::Exclusive,
                    false,
                )
                .await;
        }

        // Spawn loop to monitor DTMF digits and timeout
        let edge_state_clone = Arc::clone(edge_state);
        let edge_config_clone = edge_config.clone();
        let cid = call_id.to_string();
        tokio::spawn(async move {
            let start = Instant::now();
            let interval = Duration::from_millis(100);
            let mut gathered = String::new();

            while start.elapsed().as_millis() < timeout_ms as u128 {
                tokio::time::sleep(interval).await;
                if !edge_state_clone.inbound_transactions.contains_key(&cid) {
                    return;
                }
                if let Some(digits) = edge_state_clone
                    .media_relay
                    .get_dtmf_digits(&media_session_id)
                {
                    gathered = digits.clone();
                    if gathered.len() >= max_digits {
                        break;
                    }
                }
            }

            edge_state_clone.media_relay.stop_playback(port);

            info!(call_id = %cid, digits = %gathered, "gather completed, posting callback");
            let event = WebhookEvent {
                event_id: uuid::Uuid::new_v4().to_string(),
                schema_version: WEBHOOK_SCHEMA_VERSION.to_string(),
                call_id: cid.clone(),
                sequence: 3,
                occurred_at_ms: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as i64,
                event: CallEvent::DtmfReceived {
                    digits: gathered,
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
}

pub(super) async fn execute_stream(
    call_id: &str,
    edge_state: &Arc<EdgeState>,
    edge_config: &EdgeConfig,
    websocket_url: String,
    format: String,
    barge_in: bool,
) {
    let cmd = crate::sip::handlers::command_listener::CallCommand {
        call_id: call_id.to_string(),
        action: crate::sip::handlers::command_listener::CommandAction::Stream {
            params: crate::sip::handlers::command_listener::StreamParams {
                websocket_url,
                format,
                barge_in,
            },
        },
    };
    crate::sip::handlers::command_listener::handle_command(cmd, edge_state, edge_config).await;
}

pub(super) async fn execute_record(
    call_id: &str,
    edge_state: &Arc<EdgeState>,
    edge_config: &EdgeConfig,
    max_length_secs: u32,
    play_beep: bool,
) {
    let tx_opt = edge_state.inbound_transactions.get(call_id).map(|tx| {
        (
            tx.caller_relay_rtp.as_ref().map(|ep| ep.port),
            tx.gateway_relay_rtp.as_ref().map(|ep| ep.port),
            tx.session_id.clone(),
        )
    });
    if let Some((Some(caller_port), Some(gateway_port), session_id)) = tx_opt {
        if play_beep {
            let _ = edge_state
                .media_relay
                .start_playback(
                    caller_port,
                    std::path::PathBuf::from("/audio/beep.wav"),
                    crate::media::relay::PlaybackMode::Exclusive,
                    false,
                )
                .await;
            tokio::time::sleep(Duration::from_millis(500)).await;
        }

        let media_config = edge_state.recording_media_config(&edge_config.media);
        let _ = edge_state.media_relay.start_call_recording(
            &session_id,
            caller_port,
            gateway_port,
            &media_config,
        );

        let edge_state_clone = Arc::clone(edge_state);
        let cid = call_id.to_string();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(max_length_secs as u64)).await;
            edge_state_clone.media_relay.recordings.remove(&caller_port);
            edge_state_clone
                .media_relay
                .recordings
                .remove(&gateway_port);
            info!(call_id = %cid, "call recording stopped after reaching max_length_secs");
        });
    } else {
        warn!(
            call_id,
            "call must be active and bridged to start recording"
        );
    }
}

pub(super) async fn execute_say(
    call_id: &str,
    edge_state: &Arc<EdgeState>,
    edge_config: &EdgeConfig,
    text: String,
    voice: String,
) {
    info!(call_id, text, voice, "VCI Say command received (TTS)");
    let port = if edge_state.parked_calls.contains_key(call_id) {
        answer_parked_call_if_needed(call_id, edge_state, edge_config, "tts", "vosrs-tts-tag").await
    } else {
        edge_state
            .inbound_transactions
            .get(call_id)
            .and_then(|tx| tx.caller_relay_rtp.as_ref().map(|ep| ep.port))
    };
    if let Some(_port) = port {
        let edge_state_clone = Arc::clone(edge_state);
        let edge_config_clone = edge_config.clone();
        let cid = call_id.to_string();
        tokio::spawn(async move {
            let event = WebhookEvent {
                event_id: uuid::Uuid::new_v4().to_string(),
                schema_version: WEBHOOK_SCHEMA_VERSION.to_string(),
                call_id: cid.clone(),
                sequence: 5,
                occurred_at_ms: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as i64,
                event: CallEvent::CallAnswered {
                    sip_status: 200,
                    leg: "b_leg".to_string(),
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
}

pub(super) async fn execute_play_digits(
    call_id: &str,
    edge_state: &Arc<EdgeState>,
    edge_config: &EdgeConfig,
    digits: String,
) {
    info!(
        call_id,
        digits, "VCI PlayDigits command received, sending SIP INFO"
    );
    if let Some(mut tx) = edge_state.inbound_transactions.get_mut(call_id) {
        if let Some(socket) = edge_state.get_socket() {
            if let Some(orig_req) = tx.original_request.clone() {
                let mut dummy_req = (*orig_req.as_ref()).clone();
                dummy_req.method = sip_core::Method::Info;
                for digit in digits.chars() {
                    let info_body = format!("Signal={}\r\nDuration=160\r\n", digit);
                    dummy_req.headers.insert(
                        sip_core::HeaderName::new("content-type").unwrap(),
                        sip_core::HeaderValue::new_owned("application/dtmf-relay".to_string()),
                    );
                    dummy_req.body = std::borrow::Cow::Owned(info_body.into_bytes());
                    tx.dialogs.gateway.local_cseq = tx.dialogs.gateway.local_cseq.saturating_add(1);
                    let gateway = &tx.dialogs.gateway;
                    let info_bytes = crate::sip::outbound::build_b2bua_in_dialog_request(
                        &dummy_req,
                        &gateway.remote_target,
                        &edge_config.advertised_addr,
                        &gateway.route_set,
                        &gateway.call_id,
                        &gateway.local_uri,
                        &gateway.local_tag,
                        &gateway.remote_uri,
                        gateway.remote_tag.as_deref(),
                        gateway.local_cseq,
                        &dummy_req.body,
                    );

                    let target_peer = gateway
                        .route_set
                        .first()
                        .map(|route| crate::sip::outbound::target_addr_for_str(route))
                        .or_else(|| gateway.peer.clone())
                        .unwrap_or_else(|| {
                            crate::sip::outbound::target_addr_for(&gateway.remote_target)
                        });
                    let dg = PendingDatagram::new(target_peer, info_bytes);
                    let _ = edge_state.send_sip_datagram(dg, &socket, edge_config).await;
                }
            }
        }
    }
}
