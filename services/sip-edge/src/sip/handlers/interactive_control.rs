//! Interactive Webhook Call Control (VCI) for HTTP control mode.

use crate::config::EdgeConfig;
use crate::edge_state::{EdgeState, ParkedCall, PendingDatagram};
use crate::sip::handlers::response_for_media_error;
use call_core::{CallEvent, VciInstruction, WebhookEvent, WEBHOOK_SCHEMA_VERSION};
use sip_core::SipRequest;
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tracing::{error, info, warn};

pub(crate) fn get_http_client() -> &'static reqwest::Client {
    static HTTP_CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    HTTP_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap()
    })
}

/// Main entry point when an INVITE request comes in and control_mode is "http".
pub(crate) async fn handle_interactive_webhook_call(
    request: SipRequest,
    peer: SocketAddr,
    edge_state: &Arc<EdgeState>,
    edge_config: &EdgeConfig,
) -> Vec<PendingDatagram> {
    let call_id = request
        .headers
        .get("call-id")
        .map(|v| v.as_str().to_string())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    info!(call_id = %call_id, "intercepting call via HTTP interactive Webhook control");

    // 1. Allocate media endpoint
    let local_ep = match edge_state
        .media_relay
        .allocate_endpoint_for_call(&edge_config.media, &call_id)
    {
        Ok(ep) => ep,
        Err(e) => {
            warn!(error = %e, "failed to allocate endpoint for interactive webhook call");
            return vec![PendingDatagram::new(
                peer.to_string(),
                response_for_media_error(&request, &e),
            )];
        }
    };

    // 2. Parse client SDP and register codec, bind target
    let _client_ep = match crate::media::sdp::parse_sdp_rtp_endpoint(&request.body) {
        Ok(ep) => {
            let codec = crate::media::sdp::negotiated_audio_codec(&request.body)
                .unwrap_or(rtp_core::AudioCodec::Pcma);
            edge_state
                .media_relay
                .register_port_codec(local_ep.port, codec);
            let _ = edge_state.media_relay.set_target(&local_ep, &ep);
            Some(ep)
        }
        Err(e) => {
            warn!(error = %e, "failed to parse client SDP");
            edge_state.media_relay.clear_target(local_ep.port);
            return vec![PendingDatagram::new(
                peer.to_string(),
                response_for_media_error(&request, &e),
            )];
        }
    };

    // 3. Park the call
    let parked = ParkedCall {
        invite_request: request.clone(),
        peer_addr: peer,
        caller_relay_port: local_ep.port,
        created_at: Instant::now(),
    };
    edge_state.parked_calls.insert(call_id.clone(), parked);

    // 4. Send CallInitiated event to Webhook endpoint
    let caller = EdgeState::username_from_request(&request);
    let callee = request.uri.user.as_deref().unwrap_or("").to_string();

    let event = WebhookEvent {
        event_id: uuid::Uuid::new_v4().to_string(),
        schema_version: WEBHOOK_SCHEMA_VERSION.to_string(),
        call_id: call_id.clone(),
        sequence: 1,
        occurred_at_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64,
        event: CallEvent::CallInitiated {
            caller,
            callee: Some(callee),
            direction: "inbound".to_string(),
            leg: "a_leg".to_string(),
        },
    };
    // 4.5 Send 100 Trying immediately to prevent INVITE retransmissions
    if let Some(socket) = edge_state.get_socket() {
        let trying_resp = crate::sip::response::response_100_trying(&request);
        let dg = PendingDatagram::new(peer.to_string(), trying_resp);
        let _ = edge_state.send_sip_datagram(dg, &socket, edge_config).await;
    }

    // Send the webhook and handle response
    if let Some(instruction) = post_webhook_event(edge_state, edge_config, &event).await {
        execute_instruction(
            instruction,
            call_id.clone(),
            Arc::clone(edge_state),
            edge_config.clone(),
        )
        .await;
    } else {
        // Fallback to Hangup if Webhook fails
        let hangup_cmd = crate::sip::handlers::command_listener::CallCommand {
            call_id: call_id.clone(),
            action: crate::sip::handlers::command_listener::CommandAction::Hangup {
                params: crate::sip::handlers::command_listener::HangupParams {
                    sip_cause: Some(500),
                },
            },
        };
        crate::sip::handlers::command_listener::handle_command(hangup_cmd, edge_state, edge_config)
            .await;
    }

    // Since command_listener handle_command sends SIP packets directly, we return empty list
    Vec::new()
}

/// Helper to answer a parked call. This prepares media relay and returns caller port.
async fn answer_parked_call_if_needed(
    call_id: &str,
    edge_state: &Arc<EdgeState>,
    edge_config: &EdgeConfig,
    service_name: &str,
    to_tag: &str,
) -> Option<u16> {
    let parked = edge_state.parked_calls.remove(call_id)?.1;
    let socket = edge_state.get_socket()?;

    let client_ep = crate::media::sdp::parse_sdp_rtp_endpoint(&parked.invite_request.body).ok();
    let local_ep = sdp_core::RtpEndpoint {
        address: edge_config.media.advertised_addr.clone(),
        port: parked.caller_relay_port,
    };

    edge_state.remember_inbound_invite(
        &parked.invite_request,
        parked.peer_addr,
        sip_core::SipUri::from_str(&format!(
            "sip:{}@{}",
            service_name, edge_config.advertised_addr
        ))
        .unwrap(),
        client_ep.clone(),
        Some(local_ep.clone()),
        false,
        None,
    );

    if let Some(mut tx) = edge_state.inbound_transactions.get_mut(call_id) {
        tx.caller_relay_rtp = Some(local_ep);
        tx.inbound_to_tag = Some(to_tag.to_string());
    }

    let codec = crate::media::sdp::negotiated_audio_codec(&parked.invite_request.body)
        .unwrap_or(rtp_core::AudioCodec::Pcma);
    let pt = codec.static_payload_type().unwrap_or(8);
    let codec_name = match codec {
        rtp_core::AudioCodec::Pcmu => "PCMU",
        _ => "PCMA",
    };

    let sdp_answer = format!(
        "v=0\r\n\
         o=vos-rs 123456 123456 IN IP4 {addr}\r\n\
         s=vos-rs-{service_name}\r\n\
         c=IN IP4 {addr}\r\n\
         t=0 0\r\n\
         m=audio {port} RTP/AVP {pt}\r\n\
         a=rtpmap:{pt} {codec_name}/8000\r\n\
         a=sendrecv\r\n",
        addr = edge_config.media.advertised_addr,
        port = parked.caller_relay_port,
    );

    let resp = crate::sip::response::build_response_with_owned_headers(
        &parked.invite_request,
        200,
        "OK",
        &[
            ("Content-Type".to_string(), "application/sdp".to_string()),
            (
                "Contact".to_string(),
                format!("<sip:{}@{}>", service_name, edge_config.advertised_addr),
            ),
        ],
        &sdp_answer,
    );

    let dg = PendingDatagram::new(parked.peer_addr.to_string(), resp);
    let _ = edge_state.send_sip_datagram(dg, &socket, edge_config).await;

    edge_state
        .media_relay
        .register_port_codec(parked.caller_relay_port, codec);
    if let Some(ref client_rtp) = client_ep {
        let _ = edge_state.media_relay.set_target(
            &sdp_core::RtpEndpoint {
                address: edge_config.media.advertised_addr.clone(),
                port: parked.caller_relay_port,
            },
            client_rtp,
        );
    }

    Some(parked.caller_relay_port)
}

/// Helper to serialize, sign/deliver event via HTTP or NATS Request-Reply.
pub(crate) async fn post_webhook_event(
    edge_state: &Arc<EdgeState>,
    edge_config: &EdgeConfig,
    event: &WebhookEvent,
) -> Option<VciInstruction> {
    if edge_config.webhooks.control_mode == "nats" {
        let nats = match edge_state.nats_connection() {
            Some(n) => n,
            None => {
                error!("NATS client not initialized for call control");
                return None;
            }
        };

        let payload = match serde_json::to_vec(event) {
            Ok(p) => p,
            Err(e) => {
                error!("failed to serialize webhook event for NATS: {:?}", e);
                return None;
            }
        };

        let subject = edge_config.webhooks.control_incoming_subject.clone();
        info!(subject = %subject, call_id = %event.call_id, "sending webhook event over NATS request-reply");

        let request_future = nats.request(subject, payload.into());
        let response = tokio::time::timeout(Duration::from_secs(5), request_future).await;

        match response {
            Ok(Ok(reply)) => match serde_json::from_slice::<VciInstruction>(&reply.payload) {
                Ok(instruction) => Some(instruction),
                Err(e) => {
                    warn!(
                        "failed to deserialize VciInstruction from NATS reply: {:?}",
                        e
                    );
                    None
                }
            },
            Ok(Err(e)) => {
                warn!("NATS request failed: {:?}", e);
                None
            }
            Err(_) => {
                warn!("NATS request timed out");
                None
            }
        }
    } else {
        let endpoint = &edge_config.webhooks.endpoint_url;
        if endpoint.trim().is_empty() {
            return None;
        }
        let body = match serde_json::to_vec(event) {
            Ok(b) => b,
            Err(e) => {
                error!(error = %e, "failed to serialize webhook event");
                return None;
            }
        };
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            .to_string();
        let signature = crate::webhook_delivery::sign_payload(
            &edge_config.webhooks.signing_secret,
            &timestamp,
            &body,
        )
        .ok()?;

        let client = get_http_client();
        let response = client
            .post(endpoint)
            .header("content-type", "application/json")
            .header("x-vos-webhook-id", &event.event_id)
            .header("x-vos-webhook-timestamp", &timestamp)
            .header("x-vos-webhook-signature", format!("v1={}", signature))
            .body(body)
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => match resp.json::<VciInstruction>().await {
                Ok(inst) => Some(inst),
                Err(e) => {
                    warn!(
                        "failed to deserialize VciInstruction from Webhook response: {:?}",
                        e
                    );
                    None
                }
            },
            Ok(resp) => {
                warn!("Webhook returned HTTP error status: {}", resp.status());
                None
            }
            Err(e) => {
                warn!("failed to send webhook: {:?}", e);
                None
            }
        }
    }
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

        match instruction {
            VciInstruction::Dial {
                targets,
                sim_ring: _,
                caller_id,
                timeout_secs,
                record_call: _,
            } => {
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
                crate::sip::handlers::command_listener::handle_command(
                    cmd,
                    edge_state,
                    edge_config,
                )
                .await;
            }

            VciInstruction::Play { url, loop_count } => {
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
                                        .as_millis()
                                        as i64,
                                    event: CallEvent::CallAnswered {
                                        sip_status: 200,
                                        leg: "b_leg".to_string(),
                                    },
                                };
                                if let Some(next_inst) = post_webhook_event(
                                    &edge_state_clone,
                                    &edge_config_clone,
                                    &event,
                                )
                                .await
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

            VciInstruction::Gather {
                play_url,
                max_digits,
                timeout_ms,
                inter_digit_timeout_ms: _,
                finish_on_key: _,
                barge_in: _,
            } => {
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
                    edge_state
                        .media_relay
                        .register_port_dtmf_tracking(call_id, port, 101);

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
                            if let Some(digits) = edge_state_clone.media_relay.get_dtmf_digits(&cid)
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

            VciInstruction::Hangup {
                reason_code: _,
                sip_cause,
            } => {
                let cmd = crate::sip::handlers::command_listener::CallCommand {
                    call_id: call_id.to_string(),
                    action: crate::sip::handlers::command_listener::CommandAction::Hangup {
                        params: crate::sip::handlers::command_listener::HangupParams { sip_cause },
                    },
                };
                crate::sip::handlers::command_listener::handle_command(
                    cmd,
                    edge_state,
                    edge_config,
                )
                .await;
            }

            VciInstruction::Stream {
                websocket_url,
                format,
                barge_in,
            } => {
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
                crate::sip::handlers::command_listener::handle_command(
                    cmd,
                    edge_state,
                    edge_config,
                )
                .await;
            }

            VciInstruction::Record {
                max_length_secs,
                play_beep,
                trim_silence: _,
                silence_threshold_db: _,
            } => {
                let tx_opt = edge_state.inbound_transactions.get(call_id).map(|tx| {
                    (
                        tx.caller_relay_rtp.as_ref().map(|ep| ep.port),
                        tx.gateway_relay_rtp.as_ref().map(|ep| ep.port),
                    )
                });
                if let Some((Some(caller_port), Some(gateway_port))) = tx_opt {
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

                    let _ = edge_state.media_relay.start_call_recording(
                        call_id,
                        caller_port,
                        gateway_port,
                        &edge_config.media,
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

            VciInstruction::Say {
                text,
                voice,
                speed: _,
                pitch: _,
            } => {
                info!(call_id, text, voice, "VCI Say command received (TTS)");
                let port = if edge_state.parked_calls.contains_key(call_id) {
                    answer_parked_call_if_needed(
                        call_id,
                        edge_state,
                        edge_config,
                        "tts",
                        "vosrs-tts-tag",
                    )
                    .await
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

            VciInstruction::Queue {
                queue_id: _,
                moh_url,
                priority: _,
            } => {
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
                    call_id.clone(),
                    Arc::clone(edge_state),
                    edge_config.clone(),
                )
                .await;
            }

            VciInstruction::Conference {
                room_id,
                start_muted: _,
                end_on_exit: _,
                max_participants: _,
            } => {
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
                    let target_addr_opt =
                        if let Some(tx) = edge_state.inbound_transactions.get(call_id) {
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

            VciInstruction::Redirect { url } => {
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

            VciInstruction::Pause { duration_ms } => {
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

            VciInstruction::PlayDigits {
                digits,
                duration_ms: _,
            } => {
                info!(
                    call_id,
                    digits, "VCI PlayDigits command received, sending SIP INFO"
                );
                if let Some(tx) = edge_state.inbound_transactions.get(call_id) {
                    if let Some(socket) = edge_state.get_socket() {
                        if let Some(ref orig_req) = tx.original_request {
                            let mut dummy_req = (*orig_req.as_ref()).clone();
                            dummy_req.method = sip_core::Method::Info;
                            for digit in digits.chars() {
                                let info_body = format!("Signal={}\r\nDuration=160\r\n", digit);
                                dummy_req.headers.insert(
                                    sip_core::HeaderName::new("content-type").unwrap(),
                                    sip_core::HeaderValue::new_owned(
                                        "application/dtmf-relay".to_string(),
                                    ),
                                );
                                dummy_req.body = std::borrow::Cow::Owned(info_body.into_bytes());

                                let info_bytes =
                                    crate::sip::outbound::build_outbound_in_dialog_request(
                                        &dummy_req,
                                        &tx.outbound_uri,
                                        &edge_config.advertised_addr,
                                        tx.outbound_route_set.as_slice(),
                                    );

                                let target_peer =
                                    tx.outbound_peer.clone().unwrap_or_else(|| tx.peer.clone());
                                let dg = PendingDatagram::new(target_peer, info_bytes);
                                let _ =
                                    edge_state.send_sip_datagram(dg, &socket, edge_config).await;
                            }
                        }
                    }
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use call_core::VciInstruction;

    #[test]
    fn test_vci_instruction_parsing() {
        let say_json = r#"{
            "action": "say",
            "text": "Hello world",
            "voice": "zh-CN",
            "speed": 1.0,
            "pitch": 0
        }"#;
        let inst: VciInstruction = serde_json::from_str(say_json).unwrap();
        assert!(matches!(
            inst,
            VciInstruction::Say { text, voice, speed: _, pitch: _ } if text == "Hello world" && voice == "zh-CN"
        ));

        let hangup_json = r#"{
            "action": "hangup",
            "reason_code": 16,
            "sip_cause": 200
        }"#;
        let inst: VciInstruction = serde_json::from_str(hangup_json).unwrap();
        assert!(matches!(
            inst,
            VciInstruction::Hangup { reason_code: 16, sip_cause: Some(200) }
        ));

        let redirect_json = r#"{
            "action": "redirect",
            "url": "http://new-url/webhook"
        }"#;
        let inst: VciInstruction = serde_json::from_str(redirect_json).unwrap();
        assert!(matches!(
            inst,
            VciInstruction::Redirect { url } if url == "http://new-url/webhook"
        ));

        let stream_json = r#"{
            "action": "stream",
            "websocket_url": "wss://audio-server/stream",
            "format": "pcm",
            "barge_in": false
        }"#;
        let inst: VciInstruction = serde_json::from_str(stream_json).unwrap();
        assert!(matches!(
            inst,
            VciInstruction::Stream { websocket_url, format, barge_in: false } if websocket_url == "wss://audio-server/stream" && format == "pcm"
        ));
    }
}
