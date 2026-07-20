use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use serde::Deserialize;
use sip_core::SipUri;
use tracing::{error, info, warn};

use crate::edge_state::{EdgeState, PendingDatagram};
use crate::sip::handlers::{prepare_rewritten_sdp, response_for_media_error};
use crate::sip::{outbound, response};

#[derive(Debug, Deserialize)]
pub struct DialParams {
    pub target_gateway: Option<String>,
    pub target_uri: Option<String>,
    pub caller_id: Option<String>,
    pub timeout_secs: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct PlayParams {
    pub url: String,
    pub loop_count: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct GatherParams {
    pub play_url: Option<String>,
    pub max_digits: usize,
    pub timeout_ms: u64,
}

#[derive(Debug, Deserialize)]
pub struct HangupParams {
    pub sip_cause: Option<u16>,
}

#[derive(Debug, Deserialize)]
pub struct StreamParams {
    pub websocket_url: String,
    pub format: String,
    pub barge_in: bool,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum CommandAction {
    Dial {
        #[serde(flatten)]
        params: DialParams,
    },
    Play {
        #[serde(flatten)]
        params: PlayParams,
    },
    Gather {
        #[serde(flatten)]
        params: GatherParams,
    },
    Hangup {
        #[serde(flatten)]
        params: HangupParams,
    },
    Stream {
        #[serde(flatten)]
        params: StreamParams,
    },
    Record {
        max_length_secs: u32,
        play_beep: bool,
    },
    Say {
        text: String,
        voice: String,
    },
    Queue {
        queue_id: String,
        moh_url: String,
    },
    Conference {
        room_id: String,
        start_muted: bool,
    },
    Redirect {
        url: String,
    },
    Pause {
        duration_ms: u64,
    },
    PlayDigits {
        digits: String,
    },
    Originate {
        target_uri: String,
        caller_id: String,
    },
    Bridge {
        call_id_a: String,
        call_id_b: String,
    },
}

#[derive(Debug, Deserialize)]
pub struct CallCommand {
    pub call_id: String,
    #[serde(flatten)]
    pub action: CommandAction,
}

const PARKED_CALL_TTL: Duration = Duration::from_secs(120);

fn finalize_vci_hangup(edge_state: &EdgeState, call_id: &str, termination_reason: &str) {
    let call_id_value = call_core::CallId::new(call_id.to_string());
    if edge_state
        .call_manager
        .try_terminate_call_with_reason(call_id, termination_reason)
    {
        crate::billing_settlement::settle_completed_call(edge_state, &call_id_value);
    } else {
        crate::resource_lease::release(edge_state, &call_id_value);
    }
}

fn clear_call_id_mapping(edge_state: &EdgeState, internal_call_id: &str) {
    if let Some((_, external_call_id)) = edge_state
        .internal_to_external_call_ids
        .remove(internal_call_id)
    {
        edge_state
            .external_to_internal_call_ids
            .remove(&external_call_id);
    }
}

pub async fn start_command_listener(
    edge_state: Arc<EdgeState>,
    edge_config: Arc<crate::config::EdgeConfig>,
    nats: async_nats::Client,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let subject = edge_config.webhooks.control_command_subject.clone();
    info!(subject = %subject, "Starting NATS VCI command listener");

    let parked_cleanup_state = Arc::clone(&edge_state);
    let parked_cleanup_config = Arc::clone(&edge_config);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(10));
        interval.tick().await;
        loop {
            interval.tick().await;
            let now = std::time::Instant::now();
            let mut expired = Vec::new();
            for entry in parked_cleanup_state.parked_calls.iter() {
                if now.duration_since(entry.value().created_at) > PARKED_CALL_TTL {
                    expired.push(entry.key().clone());
                }
            }
            for call_id in expired {
                if let Some((_, parked)) = parked_cleanup_state.parked_calls.remove(&call_id) {
                    info!(call_id = %call_id, "expired parked call cleaned up");
                    parked_cleanup_state
                        .media_relay
                        .clear_target(parked.caller_relay_port);
                    if let Some(socket) = parked_cleanup_state.get_socket() {
                        let timeout_resp = response::build_response_with_owned_headers(
                            &parked.invite_request,
                            408,
                            "Request Timeout",
                            &[],
                            "",
                        );
                        let dg = PendingDatagram::new(parked.peer_addr.to_string(), timeout_resp);
                        let _ = parked_cleanup_state
                            .send_sip_datagram(dg, &socket, &parked_cleanup_config)
                            .await;
                    }
                }
            }
        }
    });

    let mut subscriber = nats.subscribe(subject).await?;

    while let Some(message) = subscriber.next().await {
        let edge_state_clone = Arc::clone(&edge_state);
        let edge_config_clone = Arc::clone(&edge_config);

        tokio::spawn(async move {
            if let Ok(command) = serde_json::from_slice::<CallCommand>(&message.payload) {
                handle_command(command, &edge_state_clone, &edge_config_clone).await;
            } else {
                warn!("failed to deserialize CallCommand from NATS payload");
            }
        });
    }

    Ok(())
}

pub async fn handle_command(
    command: CallCommand,
    edge_state: &Arc<EdgeState>,
    edge_config: &crate::config::EdgeConfig,
) {
    let call_id = command.call_id;
    let socket = match edge_state.get_socket() {
        Some(s) => s,
        None => {
            error!(
                call_id,
                "UdpSocket not initialized in EdgeState, cannot send SIP response"
            );
            return;
        }
    };

    match command.action {
        CommandAction::Dial { params } => {
            info!(call_id, "VCI Dial command execution started");

            let parked = match edge_state.parked_calls.remove(&call_id) {
                Some((_, p)) => p,
                None => {
                    warn!(call_id, "parked call not found for dial command");
                    return;
                }
            };

            let callee = parked
                .invite_request
                .uri
                .user
                .as_deref()
                .unwrap_or("")
                .to_string();
            let outbound_uri = if let Some(ref uri_str) = params.target_uri {
                SipUri::from_str(uri_str).unwrap_or_else(|_| parked.invite_request.uri.clone())
            } else if let Some(ref gw_id) = params.target_gateway {
                let gw_addr = edge_state
                    .gateway_target(gw_id)
                    .unwrap_or_else(|| edge_config.default_gateway.clone());
                SipUri::from_str(&format!("sip:{}@{}", callee, gw_addr))
                    .unwrap_or_else(|_| parked.invite_request.uri.clone())
            } else {
                parked.invite_request.uri.clone()
            };

            let rewritten_sdp = match prepare_rewritten_sdp(
                &parked.invite_request.headers,
                &parked.invite_request.body,
                &edge_state.media_relay,
                &edge_config.media,
                "inbound INVITE offer via VCI dial",
                &call_id,
            ) {
                Ok(sdp) => sdp,
                Err(error) => {
                    warn!(call_id, %error, "failed to rewrite SDP for VCI dial");
                    let err_resp = response_for_media_error(&parked.invite_request, &error);
                    let datagram = PendingDatagram::new(parked.peer_addr.to_string(), err_resp);
                    let _ = edge_state
                        .send_sip_datagram(datagram, &socket, edge_config)
                        .await;
                    return;
                }
            };

            if let Some(ref rewritten_sdp) = rewritten_sdp {
                if let Some(caller_rtp) = &rewritten_sdp.original_endpoint {
                    crate::sip::handlers::register_relay_target(
                        &edge_state.media_relay,
                        &rewritten_sdp.relay_endpoint,
                        caller_rtp,
                        "gateway-to-caller RTP via VCI",
                    );
                }
            }

            let gateway_id = params.target_gateway.unwrap_or_default();

            edge_state.remember_inbound_invite(
                &parked.invite_request,
                parked.peer_addr,
                outbound_uri.clone(),
                rewritten_sdp
                    .as_ref()
                    .and_then(|sdp| sdp.original_endpoint.clone()),
                rewritten_sdp.as_ref().map(|sdp| sdp.relay_endpoint.clone()),
                false,
                params.timeout_secs,
            );

            let external_call_id = uuid::Uuid::new_v4().to_string();
            edge_state.register_call_id_mapping(&call_id, &external_call_id);

            let target_addr = if outbound_uri.port.is_some() {
                format!("{}:{}", outbound_uri.host, outbound_uri.port.unwrap())
            } else {
                format!("{}:5060", outbound_uri.host)
            };

            let caller_identity = params
                .caller_id
                .as_ref()
                .map(|num| call_core::CallerIdentity {
                    original_number: num.clone(),
                    presented_number: num.clone(),
                    owner_gateway_id: call_core::GatewayId::new(gateway_id.clone()),
                    mode: call_core::CallerIdentityMode::Fixed,
                    max_concurrent: 0,
                });

            let outbound_invite_bytes =
                outbound::build_outbound_invite_with_session_timer_call_id_and_caller(
                    &parked.invite_request,
                    &outbound_uri,
                    &edge_config.advertised_addr,
                    rewritten_sdp
                        .as_ref()
                        .map(|sdp| sdp.body.as_slice())
                        .unwrap_or(parked.invite_request.body.as_ref()),
                    edge_config.session_expires_gateway,
                    &[],
                    &external_call_id,
                    caller_identity.as_ref(),
                );

            let datagram = PendingDatagram::new(target_addr, outbound_invite_bytes);
            if let Err(e) = edge_state
                .send_sip_datagram(datagram, &socket, edge_config)
                .await
            {
                error!(
                    call_id,
                    error = %e,
                    "failed to send outbound INVITE datagram for VCI Dial"
                );
            }

            if !gateway_id.is_empty() {
                edge_state.gateway_health.increment_active(&gateway_id);
                let status = edge_state.gateway_health.get_gateway_status(&gateway_id);
                crate::timers::persist_gateway_health(edge_state, gateway_id, status);
            }
        }

        CommandAction::Hangup { params } => {
            info!(call_id, "VCI Hangup command execution started");

            let sip_cause = params.sip_cause.unwrap_or(603);
            let termination_reason = format!("VCI Hangup ({sip_cause})");

            if let Some((_, parked)) = edge_state.parked_calls.remove(&call_id) {
                let code = sip_cause;
                let reason = match code {
                    486 => "Busy Here",
                    480 => "Temporarily Unavailable",
                    488 => "Not Acceptable Here",
                    503 => "Service Unavailable",
                    _ => "Decline",
                };
                let resp = response::build_response_with_owned_headers(
                    &parked.invite_request,
                    code,
                    reason,
                    &[],
                    "",
                );
                let dg = PendingDatagram::new(parked.peer_addr.to_string(), resp);
                let _ = edge_state.send_sip_datagram(dg, &socket, edge_config).await;
                edge_state
                    .media_relay
                    .clear_target(parked.caller_relay_port);
            } else if let Some((_, tx)) = edge_state.inbound_transactions.remove(&call_id) {
                if let Some(ref gw_relay) = tx.gateway_relay_rtp {
                    edge_state.media_relay.clear_target(gw_relay.port);
                }
                if let Some(ref caller_relay) = tx.caller_relay_rtp {
                    edge_state.media_relay.clear_target(caller_relay.port);
                }

                if let Some(ref gw_peer) = tx.outbound_peer {
                    if let Ok(gw_addr) = gw_peer.parse::<SocketAddr>() {
                        let route_set = tx.outbound_route_set.as_slice();
                        let bye_bytes = outbound::build_outbound_in_dialog_request(
                            tx.original_request
                                .as_ref()
                                .unwrap_or(&tx.original_request.clone().unwrap()),
                            &tx.outbound_uri,
                            &edge_config.advertised_addr,
                            route_set,
                        );
                        let _ = edge_state
                            .send_sip_datagram(
                                PendingDatagram::new(gw_addr.to_string(), bye_bytes),
                                &socket,
                                edge_config,
                            )
                            .await;
                    }
                }

                if let Ok(caller_addr) = tx.peer.parse::<SocketAddr>() {
                    let bye_resp = response::ok_for_request(
                        tx.original_request
                            .as_ref()
                            .unwrap_or(&tx.original_request.clone().unwrap()),
                    );
                    let _ = edge_state
                        .send_sip_datagram(
                            PendingDatagram::new(caller_addr.to_string(), bye_resp),
                            &socket,
                            edge_config,
                        )
                        .await;
                }

                if let Some(gw_id) = edge_state.call_manager.current_gateway_id(&call_id) {
                    edge_state.gateway_health.decrement_active(&gw_id);
                    let status = edge_state.gateway_health.get_gateway_status(&gw_id);
                    crate::timers::persist_gateway_health(edge_state, gw_id, status);
                }

                if let Some(username) = EdgeState::username_from_request(
                    tx.original_request
                        .as_ref()
                        .unwrap_or(&tx.original_request.clone().unwrap()),
                ) {
                    edge_state.decrement_user_concurrency(&username);
                }

                clear_call_id_mapping(edge_state, &call_id);
            }

            finalize_vci_hangup(edge_state, &call_id, &termination_reason);
        }

        CommandAction::Play { params } => {
            info!(call_id, "VCI Play command execution started");

            let parked = match edge_state.parked_calls.get(&call_id) {
                Some(p) => p.value().clone(),
                None => {
                    warn!(call_id, "parked call not found for play command");
                    return;
                }
            };

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
                 s=vos-rs-playback\r\n\
                 c=IN IP4 {addr}\r\n\
                 t=0 0\r\n\
                 m=audio {port} RTP/AVP {pt}\r\n\
                 a=rtpmap:{pt} {codec_name}/8000\r\n\
                 a=sendrecv\r\n",
                addr = edge_config.media.advertised_addr,
                port = parked.caller_relay_port,
            );

            let resp = response::build_response_with_owned_headers(
                &parked.invite_request,
                200,
                "OK",
                &[
                    ("Content-Type".to_string(), "application/sdp".to_string()),
                    (
                        "Contact".to_string(),
                        format!("<sip:vosrs-playback@{}>", edge_config.advertised_addr),
                    ),
                ],
                &sdp_answer,
            );

            let dg = PendingDatagram::new(parked.peer_addr.to_string(), resp);
            let _ = edge_state.send_sip_datagram(dg, &socket, edge_config).await;

            edge_state.parked_calls.remove(&call_id);

            edge_state
                .media_relay
                .register_port_codec(parked.caller_relay_port, codec);

            let file_path = params.url.clone();
            let loop_playback = params.loop_count.unwrap_or(1) > 1;
            let _ = edge_state
                .media_relay
                .start_playback(
                    parked.caller_relay_port,
                    std::path::PathBuf::from(file_path),
                    crate::media::relay::PlaybackMode::Exclusive,
                    loop_playback,
                )
                .await;
        }

        CommandAction::Gather { params } => {
            info!(call_id, "VCI Gather command execution started");

            let parked = match edge_state.parked_calls.get(&call_id) {
                Some(p) => p.value().clone(),
                None => {
                    warn!(call_id, "parked call not found for gather command");
                    return;
                }
            };

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
                 s=vos-rs-gather\r\n\
                 c=IN IP4 {addr}\r\n\
                 t=0 0\r\n\
                 m=audio {port} RTP/AVP {pt} 101\r\n\
                 a=rtpmap:{pt} {codec_name}/8000\r\n\
                 a=rtpmap:101 telephone-event/8000\r\n\
                 a=fmtp:101 0-15\r\n\
                 a=sendrecv\r\n",
                addr = edge_config.media.advertised_addr,
                port = parked.caller_relay_port,
            );

            let resp = response::build_response_with_owned_headers(
                &parked.invite_request,
                200,
                "OK",
                &[
                    ("Content-Type".to_string(), "application/sdp".to_string()),
                    (
                        "Contact".to_string(),
                        format!("<sip:vosrs-gather@{}>", edge_config.advertised_addr),
                    ),
                ],
                &sdp_answer,
            );

            let dg = PendingDatagram::new(parked.peer_addr.to_string(), resp);
            let _ = edge_state.send_sip_datagram(dg, &socket, edge_config).await;

            edge_state.parked_calls.remove(&call_id);

            edge_state
                .media_relay
                .register_port_codec(parked.caller_relay_port, codec);

            edge_state.media_relay.register_port_dtmf_tracking(
                &call_id,
                parked.caller_relay_port,
                101,
            );

            if let Some(ref play_url) = params.play_url {
                let _ = edge_state
                    .media_relay
                    .start_playback(
                        parked.caller_relay_port,
                        std::path::PathBuf::from(play_url.clone()),
                        crate::media::relay::PlaybackMode::Exclusive,
                        false,
                    )
                    .await;
            }

            let edge_state_clone = edge_state.clone();
            let caller_relay_port = parked.caller_relay_port;
            let max_digits = params.max_digits;
            let timeout_ms = params.timeout_ms;
            let dtmf_subject = edge_config.webhooks.control_dtmf_subject.clone();

            tokio::spawn(async move {
                let start = std::time::Instant::now();
                let interval = std::time::Duration::from_millis(100);
                let mut gathered = String::new();

                while start.elapsed().as_millis() < timeout_ms as u128 {
                    tokio::time::sleep(interval).await;
                    if let Some(digits) = edge_state_clone.media_relay.get_dtmf_digits(&call_id) {
                        gathered = digits.clone();
                        if gathered.len() >= max_digits {
                            break;
                        }
                    }
                }

                edge_state_clone
                    .media_relay
                    .stop_playback(caller_relay_port);

                if let Some(nats) = edge_state_clone.nats_connection() {
                    let dtmf_event = serde_json::json!({
                        "call_id": call_id,
                        "digits": gathered,
                        "status": "success"
                    });
                    let payload = serde_json::to_vec(&dtmf_event).unwrap_or_default();
                    if let Err(e) = nats.publish(dtmf_subject, payload.into()).await {
                        warn!("failed to publish DTMF digits back to NATS: {:?}", e);
                    }
                }
            });
        }
        CommandAction::Stream { params } => {
            info!(call_id, "VCI Stream command execution started");

            // 1. Check if the call is parked (first command after INVITE)
            let port = if let Some(parked) = edge_state
                .parked_calls
                .get(&call_id)
                .map(|p| p.value().clone())
            {
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
                     s=vos-rs-stream\r\n\
                     c=IN IP4 {addr}\r\n\
                     t=0 0\r\n\
                     m=audio {port} RTP/AVP {pt}\r\n\
                     a=rtpmap:{pt} {codec_name}/8000\r\n\
                     a=sendrecv\r\n",
                    addr = edge_config.media.advertised_addr,
                    port = parked.caller_relay_port,
                );

                let resp = response::build_response_with_owned_headers(
                    &parked.invite_request,
                    200,
                    "OK",
                    &[
                        ("Content-Type".to_string(), "application/sdp".to_string()),
                        (
                            "Contact".to_string(),
                            format!("<sip:vosrs-stream@{}>", edge_config.advertised_addr),
                        ),
                    ],
                    &sdp_answer,
                );

                let dg = PendingDatagram::new(parked.peer_addr.to_string(), resp);
                let _ = edge_state.send_sip_datagram(dg, &socket, edge_config).await;

                edge_state.parked_calls.remove(&call_id);
                edge_state
                    .media_relay
                    .register_port_codec(parked.caller_relay_port, codec);

                // Register transaction so BYEs work
                let client_ep =
                    crate::media::sdp::parse_sdp_rtp_endpoint(&parked.invite_request.body).ok();
                let local_ep = sdp_core::RtpEndpoint {
                    address: edge_config.media.advertised_addr.clone(),
                    port: parked.caller_relay_port,
                };
                edge_state.remember_inbound_invite(
                    &parked.invite_request,
                    parked.peer_addr,
                    sip_core::SipUri::from_str(&format!(
                        "sip:vosrs-stream@{}",
                        edge_config.advertised_addr
                    ))
                    .unwrap(),
                    client_ep,
                    Some(local_ep.clone()),
                    false,
                    None,
                );
                if let Some(mut tx) = edge_state.inbound_transactions.get_mut(&call_id) {
                    tx.caller_relay_rtp = Some(local_ep);
                    tx.inbound_to_tag = Some("vosrs-stream-tag".to_string());
                }

                parked.caller_relay_port
            } else if let Some(tx) = edge_state.inbound_transactions.get(&call_id) {
                // If the call is already active, find its media port
                if let Some(ref local_ep) = tx.caller_relay_rtp {
                    local_ep.port
                } else {
                    warn!(
                        call_id,
                        "active call has no caller relay port, cannot start stream"
                    );
                    return;
                }
            } else {
                warn!(call_id, "call not found for stream command");
                return;
            };

            // 2. Start WebSocket audio stream
            let _ = edge_state
                .media_relay
                .start_stream(port, params.websocket_url, params.format, params.barge_in)
                .await;
        }
        CommandAction::Record {
            max_length_secs,
            play_beep,
        } => {
            let inst = call_core::VciInstruction::Record {
                max_length_secs,
                play_beep,
                trim_silence: false,
                silence_threshold_db: None,
            };
            crate::sip::handlers::interactive_control::execute_instruction(
                inst,
                call_id,
                edge_state.clone(),
                edge_config.clone(),
            )
            .await;
        }
        CommandAction::Say { text, voice } => {
            let inst = call_core::VciInstruction::Say {
                text,
                voice,
                speed: 1.0,
                pitch: 0,
            };
            crate::sip::handlers::interactive_control::execute_instruction(
                inst,
                call_id,
                edge_state.clone(),
                edge_config.clone(),
            )
            .await;
        }
        CommandAction::Queue { queue_id, moh_url } => {
            let inst = call_core::VciInstruction::Queue {
                queue_id,
                moh_url,
                priority: 1,
            };
            crate::sip::handlers::interactive_control::execute_instruction(
                inst,
                call_id,
                edge_state.clone(),
                edge_config.clone(),
            )
            .await;
        }
        CommandAction::Conference {
            room_id,
            start_muted,
        } => {
            let inst = call_core::VciInstruction::Conference {
                room_id,
                start_muted,
                end_on_exit: true,
                max_participants: 20,
            };
            crate::sip::handlers::interactive_control::execute_instruction(
                inst,
                call_id,
                edge_state.clone(),
                edge_config.clone(),
            )
            .await;
        }
        CommandAction::Redirect { url } => {
            let inst = call_core::VciInstruction::Redirect { url };
            crate::sip::handlers::interactive_control::execute_instruction(
                inst,
                call_id,
                edge_state.clone(),
                edge_config.clone(),
            )
            .await;
        }
        CommandAction::Pause { duration_ms } => {
            let inst = call_core::VciInstruction::Pause { duration_ms };
            crate::sip::handlers::interactive_control::execute_instruction(
                inst,
                call_id,
                edge_state.clone(),
                edge_config.clone(),
            )
            .await;
        }
        CommandAction::PlayDigits { digits } => {
            let inst = call_core::VciInstruction::PlayDigits {
                digits,
                duration_ms: 100,
            };
            crate::sip::handlers::interactive_control::execute_instruction(
                inst,
                call_id,
                edge_state.clone(),
                edge_config.clone(),
            )
            .await;
        }
        CommandAction::Originate {
            target_uri,
            caller_id,
        } => {
            info!(call_id, "Executing Originate command");

            let caller_relay_rtp = match edge_state
                .media_relay
                .allocate_endpoint_for_call(&edge_config.media, &call_id)
            {
                Ok(ep) => ep,
                Err(e) => {
                    warn!(call_id, error = %e, "Failed to allocate media relay endpoint for originate");
                    return;
                }
            };

            let sdp_offer = format!(
                "v=0\r\n\
                 o=vos-rs 123456 123456 IN IP4 {addr}\r\n\
                 s=vos-rs-originate\r\n\
                 c=IN IP4 {addr}\r\n\
                 t=0 0\r\n\
                 m=audio {port} RTP/AVP 8\r\n\
                 a=rtpmap:8 PCMA/8000\r\n\
                 a=sendrecv\r\n",
                addr = edge_config.media.advertised_addr,
                port = caller_relay_rtp.port,
            );

            let outbound_uri = match SipUri::from_str(&target_uri) {
                Ok(uri) => uri,
                Err(_) => {
                    warn!(call_id, "Invalid target URI for originate");
                    return;
                }
            };

            let tx = crate::edge_state::InboundTransaction {
                peer: "local-originate".to_string(),
                outbound_peer: Some(target_uri.clone()),
                vias: vec![format!(
                    "SIP/2.0/UDP {adv};branch=z9hG4bK-originate-{cid}",
                    adv = edge_config.advertised_addr,
                    cid = call_id
                )],
                outbound_uri,
                inbound_from_tag: Some(format!("originate-{}", call_id)),
                inbound_to_tag: None,
                last_inbound_cseq: Some(1),
                last_outbound_cseq: Some(1),
                caller_rtp: None,
                gateway_relay_rtp: None,
                gateway_rtp: None,
                caller_relay_rtp: Some(caller_relay_rtp),
                original_request: None,
                inbound_route_set: Vec::new(),
                outbound_route_set: Vec::new(),
                caller_contact: None,
                callee_contact: None,
                session_expires: None,
                session_refresher: None,
                last_session_refresh: None,
                prack_rseq: 1,
                gateway_100rel: false,
                refer_subscription: None,
                transfer_from_header: None,
                transfer_to_header: None,
                transfer_call_id: None,
                transfer_contact: None,
                transfer_peer: None,
                transferee_is_caller: true,
                callee_behind_nat: false,
                active_forks: Vec::new(),
                max_duration_secs: None,
                established_at: Some(std::time::Instant::now()),
                invite_response_order: Arc::new(tokio::sync::Mutex::new(
                    crate::edge_state::InviteResponseOrder::default(),
                )),
            };

            edge_state.inbound_transactions.insert(call_id.clone(), tx);

            let branch = format!("z9hG4bK-originate-{}", call_id);
            let sdp_len = sdp_offer.len();
            let invite_str = format!(
                "INVITE {target_uri} SIP/2.0\r\n\
                 Via: SIP/2.0/UDP {adv};branch={branch}\r\n\
                 Max-Forwards: 70\r\n\
                 From: <sip:{caller_id}@{adv}>;tag=originate-{call_id}\r\n\
                 To: <{target_uri}>\r\n\
                 Call-ID: {call_id}\r\n\
                 CSeq: 1 INVITE\r\n\
                 Contact: <sip:vosrs-originate@{adv}>\r\n\
                 Content-Type: application/sdp\r\n\
                 Content-Length: {sdp_len}\r\n\r\n\
                 {sdp_offer}",
                adv = edge_config.advertised_addr,
                target_uri = target_uri,
                branch = branch,
                caller_id = caller_id,
                call_id = call_id,
                sdp_len = sdp_len,
                sdp_offer = sdp_offer,
            );

            if let Some(socket) = edge_state.get_socket() {
                let target_peer = outbound::target_addr_for_str(&target_uri);
                let dg = PendingDatagram::new(target_peer, invite_str.into_bytes());
                let _ = edge_state.send_sip_datagram(dg, &socket, edge_config).await;
            }
        }
        CommandAction::Bridge {
            call_id_a,
            call_id_b,
        } => {
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::EdgeConfig;
    use crate::edge_state::ParkedCall;
    use call_core::{CallId, CallManager, CallState, CdrStatus, RouteTable};
    use sip_core::{parse_message, SipMessage};
    use std::net::SocketAddr;
    use std::sync::Arc;

    async fn make_test_state() -> Arc<EdgeState> {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let cm = CallManager::new(RouteTable::default(), tx);
        let config = EdgeConfig::default();
        let state = EdgeState::with_media_relay_and_db(
            cm,
            crate::media::MediaRelayState::new(),
            None,
            &config,
        );
        let socket = std::sync::Arc::new(tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap());
        state.set_socket(socket);
        Arc::new(state)
    }

    fn make_parked_request() -> sip_core::SipRequest {
        sip_core::SipRequest {
            method: sip_core::Method::Invite,
            uri: SipUri {
                secure: false,
                user: Some("1001".into()),
                host: "192.168.1.1".into(),
                port: Some(5060),
                params: Vec::new(),
            },
            version: std::borrow::Cow::Borrowed("SIP/2.0"),
            headers: sip_core::HeaderMap::new(),
            body: std::borrow::Cow::Borrowed(&[]),
        }
    }

    #[tokio::test]
    async fn test_hangup_removes_parked_call_and_sends_response() {
        let state = make_test_state().await;
        let config = Arc::new(EdgeConfig::default());

        let request = make_parked_request();
        let peer: SocketAddr = "192.168.1.100:5060".parse().unwrap();

        state.parked_calls.insert(
            "test-call-1".to_string(),
            ParkedCall {
                invite_request: request,
                peer_addr: peer,
                caller_relay_port: 40000,
                created_at: std::time::Instant::now(),
            },
        );

        assert!(state.parked_calls.contains_key("test-call-1"));

        let cmd = CallCommand {
            call_id: "test-call-1".to_string(),
            action: CommandAction::Hangup {
                params: HangupParams {
                    sip_cause: Some(603),
                },
            },
        };

        handle_command(cmd, &state, &config).await;

        assert!(!state.parked_calls.contains_key("test-call-1"));
    }

    #[tokio::test]
    async fn test_hangup_parked_call_with_default_cause() {
        let state = make_test_state().await;
        let config = Arc::new(EdgeConfig::default());

        let request = make_parked_request();
        let peer: SocketAddr = "192.168.1.101:5060".parse().unwrap();

        state.parked_calls.insert(
            "test-call-2".to_string(),
            ParkedCall {
                invite_request: request,
                peer_addr: peer,
                caller_relay_port: 40001,
                created_at: std::time::Instant::now(),
            },
        );

        let cmd = CallCommand {
            call_id: "test-call-2".to_string(),
            action: CommandAction::Hangup {
                params: HangupParams { sip_cause: None },
            },
        };

        handle_command(cmd, &state, &config).await;

        assert!(!state.parked_calls.contains_key("test-call-2"));
    }

    #[test]
    fn test_hangup_finalizes_managed_call_only_once() {
        let (cdr_tx, mut cdr_rx) = tokio::sync::mpsc::unbounded_channel();
        let manager = CallManager::new(RouteTable::default(), cdr_tx);
        let config = Arc::new(EdgeConfig::default());
        let state = EdgeState::with_media_relay_and_db(
            manager,
            crate::media::MediaRelayState::new(),
            None,
            &config,
        );
        let state = Arc::new(state);

        let raw_invite = b"INVITE sip:1002@example.com SIP/2.0\r\n\
Via: SIP/2.0/UDP 127.0.0.1:5060;branch=z9hG4bK-vci-finalize\r\n\
From: <sip:1001@example.com>;tag=from-vci\r\n\
To: <sip:1002@example.com>\r\n\
Call-ID: vci-finalize@example.com\r\n\
CSeq: 1 INVITE\r\n\
Content-Length: 0\r\n\r\n";
        let SipMessage::Request(invite) = parse_message(raw_invite).unwrap() else {
            panic!("expected INVITE request");
        };
        state
            .call_manager
            .handle_inbound_invite_to_uri(
                &invite,
                SipUri::from_str("sip:1002@gateway.example.com:5060").unwrap(),
            )
            .unwrap();

        finalize_vci_hangup(&state, "vci-finalize@example.com", "VCI Hangup (487)");

        let call_id = CallId::new("vci-finalize@example.com");
        assert_eq!(
            state.call_manager.get(&call_id).map(|call| call.state),
            Some(CallState::Failed)
        );
        let cdr = cdr_rx.try_recv().expect("VCI Hangup should emit a CDR");
        assert_eq!(cdr.status, CdrStatus::Failed);
        assert_eq!(
            cdr.failure_cause
                .as_ref()
                .map(|cause| cause.reason.as_str()),
            Some("VCI Hangup (487)")
        );

        finalize_vci_hangup(&state, "vci-finalize@example.com", "VCI Hangup (487)");
        assert!(cdr_rx.try_recv().is_err(), "duplicate Hangup emitted a CDR");
    }

    #[test]
    fn test_hangup_clears_both_call_id_mapping_directions() {
        let (cdr_tx, _cdr_rx) = tokio::sync::mpsc::unbounded_channel();
        let manager = CallManager::new(RouteTable::default(), cdr_tx);
        let config = EdgeConfig::default();
        let state = EdgeState::with_media_relay_and_db(
            manager,
            crate::media::MediaRelayState::new(),
            None,
            &config,
        );
        state.register_call_id_mapping("internal-call", "external-call");

        clear_call_id_mapping(&state, "internal-call");

        assert!(state.get_external_call_id("internal-call").is_none());
        assert!(state.get_internal_call_id("external-call").is_none());
    }

    #[tokio::test]
    async fn test_dial_missing_parked_call_returns_early() {
        let state = make_test_state().await;
        let config = Arc::new(EdgeConfig::default());

        let cmd = CallCommand {
            call_id: "nonexistent".to_string(),
            action: CommandAction::Dial {
                params: DialParams {
                    target_gateway: None,
                    target_uri: None,
                    caller_id: None,
                    timeout_secs: None,
                },
            },
        };

        handle_command(cmd, &state, &config).await;

        assert!(!state.parked_calls.contains_key("nonexistent"));
    }

    #[test]
    fn test_call_command_deserialize_dial() {
        let json = r#"{"call_id":"abc","action":"dial","target_gateway":"gw1","caller_id":"1001"}"#;
        let cmd: CallCommand = serde_json::from_str(json).unwrap();
        assert_eq!(cmd.call_id, "abc");
        match cmd.action {
            CommandAction::Dial { params } => {
                assert_eq!(params.target_gateway.as_deref(), Some("gw1"));
                assert_eq!(params.caller_id.as_deref(), Some("1001"));
            }
            _ => panic!("expected Dial"),
        }
    }

    #[test]
    fn test_call_command_deserialize_hangup() {
        let json = r#"{"call_id":"xyz","action":"hangup","sip_cause":486}"#;
        let cmd: CallCommand = serde_json::from_str(json).unwrap();
        assert_eq!(cmd.call_id, "xyz");
        match cmd.action {
            CommandAction::Hangup { params } => {
                assert_eq!(params.sip_cause, Some(486));
            }
            _ => panic!("expected Hangup"),
        }
    }

    #[test]
    fn test_call_command_deserialize_play() {
        let json = r#"{"call_id":"p1","action":"play","url":"/audio/welcome.wav","loop_count":2}"#;
        let cmd: CallCommand = serde_json::from_str(json).unwrap();
        match cmd.action {
            CommandAction::Play { params } => {
                assert_eq!(params.url, "/audio/welcome.wav");
                assert_eq!(params.loop_count, Some(2));
            }
            _ => panic!("expected Play"),
        }
    }

    #[test]
    fn test_call_command_deserialize_gather() {
        let json = r#"{"call_id":"g1","action":"gather","play_url":"/audio/prompt.wav","max_digits":4,"timeout_ms":5000}"#;
        let cmd: CallCommand = serde_json::from_str(json).unwrap();
        match cmd.action {
            CommandAction::Gather { params } => {
                assert_eq!(params.max_digits, 4);
                assert_eq!(params.timeout_ms, 5000);
                assert_eq!(params.play_url.as_deref(), Some("/audio/prompt.wav"));
            }
            _ => panic!("expected Gather"),
        }
    }
}
