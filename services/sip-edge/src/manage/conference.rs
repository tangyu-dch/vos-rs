//! 会议管理端点：加入/离开/状态查询/参与者静音。

use axum::{extract::State, http::StatusCode, Json};
use serde::Deserialize;
use std::net::SocketAddr;
use std::sync::Arc;

use crate::EdgeState;

#[derive(Deserialize)]
pub(super) struct JoinConferenceRequest {
    conference_id: String,
    port: u16,
    codec: String,
    target_ip: String,
    target_port: u16,
}

pub(super) async fn join_conference(
    State(state): State<Arc<EdgeState>>,
    Json(payload): Json<JoinConferenceRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let socket = match state.media_relay.active_sockets.get(&payload.port) {
        Some(s) => s.value().clone(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": format!("No active socket found for port {}", payload.port)
                })),
            );
        }
    };

    let target_addr =
        match format!("{}:{}", payload.target_ip, payload.target_port).parse::<SocketAddr>() {
            Ok(addr) => addr,
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(serde_json::json!({
                        "error": format!("Invalid target address: {}", e)
                    })),
                );
            }
        };

    let codec = match payload.codec.to_lowercase().as_str() {
        "pcmu" => rtp_core::AudioCodec::Pcmu,
        _ => rtp_core::AudioCodec::Pcma,
    };

    state
        .media_relay
        .conference_manager
        .join_conference(
            &payload.conference_id,
            payload.port,
            codec,
            target_addr,
            socket,
        )
        .await;
    state.media_relay.mark_relay_features_changed(payload.port);

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "message": "Successfully joined conference",
            "conference_id": payload.conference_id,
            "port": payload.port,
        })),
    )
}

#[derive(Deserialize)]
pub(super) struct LeaveConferenceRequest {
    port: u16,
}

pub(super) async fn leave_conference(
    State(state): State<Arc<EdgeState>>,
    Json(payload): Json<LeaveConferenceRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    state
        .media_relay
        .conference_manager
        .leave_conference(payload.port)
        .await;
    state.media_relay.mark_relay_features_changed(payload.port);

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "message": "Successfully left conference",
            "port": payload.port,
        })),
    )
}

pub(super) async fn conference_status(
    State(state): State<Arc<EdgeState>>,
) -> Json<serde_json::Value> {
    let mut list = Vec::new();
    for entry in state.media_relay.conference_manager.conferences.iter() {
        let conf = entry.value().lock().await;
        let mut participants = Vec::new();
        for p in conf.participants.values() {
            participants.push(serde_json::json!({
                "port": p.port,
                "codec": format!("{:?}", p.codec).to_lowercase(),
                "target_addr": p.target_addr.to_string(),
                "ssrc": p.ssrc,
                "sequence": p.sequence,
                "timestamp": p.timestamp,
                "buffered_pcm_samples": p.pcm_buffer.len(),
                "muted": p.muted,
            }));
        }
        list.push(serde_json::json!({
            "conference_id": conf.id,
            "participants": participants,
        }));
    }
    Json(serde_json::json!({
        "conferences": list
    }))
}

#[derive(Deserialize)]
pub(super) struct MuteConferenceParticipantRequest {
    conference_id: String,
    port: u16,
    mute: bool,
}

pub(super) async fn mute_conference_participant(
    State(state): State<Arc<EdgeState>>,
    Json(payload): Json<MuteConferenceParticipantRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let success = state
        .media_relay
        .conference_manager
        .set_participant_mute(&payload.conference_id, payload.port, payload.mute)
        .await;

    if success {
        (
            StatusCode::OK,
            Json(serde_json::json!({
                "status": "success",
                "message": format!("Participant port {} mute status set to {}", payload.port, payload.mute)
            })),
        )
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "status": "error",
                "message": "Conference or participant not found"
            })),
        )
    }
}
