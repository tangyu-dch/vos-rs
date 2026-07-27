//! Webhook-related handlers for interactive call control.

use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use call_core::{CallEvent, VciInstruction, WebhookEvent, WEBHOOK_SCHEMA_VERSION};
use sip_core::SipRequest;
use tracing::{error, info, warn};

use crate::config::EdgeConfig;
use crate::edge_state::{EdgeState, ParkedCall, PendingDatagram};
use crate::sip::handlers::response_for_media_error;

use super::{execute_instruction, get_http_client};

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
    let session_id = uuid::Uuid::new_v4().to_string();

    info!(call_id = %call_id, "intercepting call via HTTP interactive Webhook control");

    // 1. Allocate media endpoint
    let local_ep = match edge_state
        .media_relay
        .allocate_endpoint_for_call(&edge_config.media, &session_id)
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
        session_id,
        invite_request: request.clone(),
        peer_addr: peer,
        caller_relay_port: local_ep.port,
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
pub(crate) async fn answer_parked_call_if_needed(
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
        parked.session_id,
        &parked.invite_request,
        parked.peer_addr,
        sip_core::SipUri::from_str(&format!(
            "sip:{}@{}",
            service_name, edge_config.advertised_addr
        ))
        .unwrap(),
        client_ep.clone(),
        Some(local_ep.clone()),
        None,
    );

    if let Some(mut tx) = edge_state.inbound_transactions.get_mut(call_id) {
        tx.caller_relay_rtp = Some(local_ep);
        tx.dialogs.caller.local_tag = to_tag.to_string();
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
