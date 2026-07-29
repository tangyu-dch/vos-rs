//! RWI (Real-Time WebSocket Interface) Gateway Handler
//!
//! Provides WebSocket endpoint `/rwi/v1/ws` for receiving call events
//! and executing control commands.

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::IntoResponse,
    Extension,
};
use call_core::rwi::{RwiCommand, RwiEvent, RwiMessage, RwiPayload};
use futures::{SinkExt, StreamExt};
use uuid::Uuid;

use crate::{system::auth::Claims, AppState};

/// Handler for `/rwi/v1/ws` WebSocket upgrade.
pub async fn rwi_ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    Extension(claims): Extension<Claims>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_rwi_socket(socket, state, claims))
}

/// Handles upgraded WebSocket connection.
async fn handle_rwi_socket(socket: WebSocket, state: AppState, claims: Claims) {
    let (mut sender, mut receiver) = socket.split();
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Message>(64);

    let send_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if sender.send(msg).await.is_err() {
                break;
            }
        }
    });

    let event_tx = tx.clone();
    let nats_client = state.nats_client.clone();
    let event_task = tokio::spawn(async move {
        if let Some(nats) = nats_client {
            subscribe_and_forward_events(nats, event_tx).await;
        }
    });

    while let Some(result) = receiver.next().await {
        let msg = match result {
            Ok(m) => m,
            Err(_) => break,
        };
        if let Err(e) = process_ws_message(&state, &claims, msg, &tx).await {
            tracing::warn!(error = %e, "Error processing RWI WS message");
        }
    }

    send_task.abort();
    event_task.abort();
}

/// Subscribes to NATS call events and forwards them over WebSocket.
async fn subscribe_and_forward_events(
    nats: async_nats::Client,
    tx: tokio::sync::mpsc::Sender<Message>,
) {
    let mut sub = match nats.subscribe("vos_rs.call.>").await {
        Ok(s) => s,
        Err(_) => return,
    };

    while let Some(msg) = sub.next().await {
        if let Ok(event) = parse_rwi_event(&msg.payload) {
            let rwi_msg = RwiMessage {
                id: Uuid::new_v4().to_string(),
                version: "1.0".to_string(),
                payload: RwiPayload::Event(event),
            };
            if let Ok(json) = serde_json::to_string(&rwi_msg) {
                if tx.send(Message::Text(json)).await.is_err() {
                    break;
                }
            }
        }
    }
}

/// Parses raw bytes into `RwiEvent`.
fn parse_rwi_event(payload: &[u8]) -> Result<RwiEvent, serde_json::Error> {
    if let Ok(event) = serde_json::from_slice::<RwiEvent>(payload) {
        return Ok(event);
    }
    let val: serde_json::Value = serde_json::from_slice(payload)?;
    parse_rwi_event_from_value(&val)
}

/// Converts generic JSON event values into structured `RwiEvent`.
fn parse_rwi_event_from_value(val: &serde_json::Value) -> Result<RwiEvent, serde_json::Error> {
    let event_type = val.get("event_type").and_then(|v| v.as_str()).unwrap_or("");
    let call_id = val
        .get("call_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let timestamp_ms = val
        .get("occurred_at_ms")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let data = val.get("data");

    match event_type {
        "call_started" | "call_initiated" => Ok(RwiEvent::CallStarted {
            call_id,
            caller: data
                .and_then(|d| d.get("caller"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            callee: data
                .and_then(|d| d.get("callee"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            direction: data
                .and_then(|d| d.get("direction"))
                .and_then(|v| v.as_str())
                .unwrap_or("inbound")
                .to_string(),
            timestamp_ms,
        }),
        "call_ringing" => Ok(RwiEvent::CallRinging {
            call_id,
            sip_status: data
                .and_then(|d| d.get("sip_status"))
                .and_then(|v| v.as_u64())
                .unwrap_or(180) as u16,
            leg: data
                .and_then(|d| d.get("leg"))
                .and_then(|v| v.as_str())
                .unwrap_or("b_leg")
                .to_string(),
            timestamp_ms,
        }),
        "call_answered" => Ok(RwiEvent::CallAnswered {
            call_id,
            sip_status: data
                .and_then(|d| d.get("sip_status"))
                .and_then(|v| v.as_u64())
                .unwrap_or(200) as u16,
            leg: data
                .and_then(|d| d.get("leg"))
                .and_then(|v| v.as_str())
                .unwrap_or("b_leg")
                .to_string(),
            timestamp_ms,
        }),
        "call_ended" | "call_finished" => Ok(RwiEvent::CallEnded {
            call_id,
            duration_secs: data
                .and_then(|d| d.get("duration_secs"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0),
            reason: data
                .and_then(|d| d.get("reason"))
                .and_then(|v| v.as_str())
                .unwrap_or("normal")
                .to_string(),
            sip_status: data
                .and_then(|d| d.get("sip_status"))
                .and_then(|v| v.as_u64())
                .map(|v| v as u16),
            timestamp_ms,
        }),
        "media_event" | "dtmf_received" => Ok(RwiEvent::MediaEvent {
            call_id,
            event_type: event_type.to_string(),
            payload: data.map(|d| d.to_string()).unwrap_or_default(),
            timestamp_ms,
        }),
        _ => Err(serde::de::Error::custom("Unknown RwiEvent type")),
    }
}

/// Processes an incoming WebSocket message frame (Text or Binary).
async fn process_ws_message(
    state: &AppState,
    claims: &Claims,
    msg: Message,
    tx: &tokio::sync::mpsc::Sender<Message>,
) -> Result<(), String> {
    let (rwi_msg, is_binary) = match msg {
        Message::Text(text) => (
            serde_json::from_str::<RwiMessage>(&text).map_err(|e| e.to_string())?,
            false,
        ),
        Message::Binary(bin) => (
            serde_json::from_slice::<RwiMessage>(&bin).map_err(|e| e.to_string())?,
            true,
        ),
        Message::Ping(p) => {
            let _ = tx.send(Message::Pong(p)).await;
            return Ok(());
        }
        _ => return Ok(()),
    };

    if let RwiPayload::Command(cmd) = rwi_msg.payload {
        authorize_rwi_command(state, claims, &cmd).await?;
        execute_rwi_command(state, &cmd).await?;
        let ack = RwiMessage {
            id: rwi_msg.id,
            version: rwi_msg.version,
            payload: RwiPayload::Command(cmd),
        };
        let response_msg = if is_binary {
            let bytes = serde_json::to_vec(&ack).map_err(|e| e.to_string())?;
            Message::Binary(bytes)
        } else {
            let text = serde_json::to_string(&ack).map_err(|e| e.to_string())?;
            Message::Text(text)
        };
        let _ = tx.send(response_msg).await;
    }
    Ok(())
}

/// 对升级后的每一条实时控制命令重新检查数据库权限快照。
///
/// 这同时保证角色权限或用户权限版本在连接存续期间发生变化后，旧连接不能继续操作。
async fn authorize_rwi_command(
    state: &AppState,
    claims: &Claims,
    command: &RwiCommand,
) -> Result<(), String> {
    let permission = rwi_command_permission(command);
    let allowed = state.access_snapshot.read().await.allows(
        &claims.sub,
        &claims.role,
        claims.auth_version,
        permission,
    );
    if allowed {
        Ok(())
    } else {
        Err(format!("越权访问：缺少权限 {permission}"))
    }
}

/// 返回实时控制命令对应的最小按钮级权限点。
fn rwi_command_permission(command: &RwiCommand) -> &'static str {
    match command {
        RwiCommand::BargeIn { .. } => "calls.barge",
        RwiCommand::Speak { .. } => "calls.play",
        RwiCommand::Listen { .. } => "calls.monitor",
        RwiCommand::Transfer { .. } => "calls.transfer",
        RwiCommand::Hangup { .. } => "calls.terminate",
    }
}

/// Executes control command (`BargeIn`, `Speak`, `Listen`, `Transfer`, `Hangup`).
pub async fn execute_rwi_command(state: &AppState, cmd: &RwiCommand) -> Result<(), String> {
    if let Some(nats) = &state.nats_client {
        if let Ok(bytes) = serde_json::to_vec(cmd) {
            let _ = nats.publish("vos_rs.call.commands", bytes.into()).await;
        }
    }
    relay_rwi_command(state, cmd).await
}

/// Relays command to sip-edge HTTP management API.
async fn relay_rwi_command(state: &AppState, cmd: &RwiCommand) -> Result<(), String> {
    let call_id = cmd.call_id();
    let (action, payload) = match cmd {
        RwiCommand::BargeIn {
            mode, target_leg, ..
        } => (
            "barge-in",
            serde_json::json!({ "mode": mode, "target_leg": target_leg }),
        ),
        RwiCommand::Speak {
            text, voice, speed, ..
        } => (
            "play",
            serde_json::json!({ "text": text, "voice": voice, "speed": speed }),
        ),
        RwiCommand::Listen {
            stream_url, format, ..
        } => (
            "stream",
            serde_json::json!({ "stream_url": stream_url, "format": format }),
        ),
        RwiCommand::Transfer {
            target,
            transfer_type,
            ..
        } => (
            "transfer",
            serde_json::json!({ "target": target, "transfer_type": transfer_type }),
        ),
        RwiCommand::Hangup { reason_code, .. } => (
            "terminate",
            serde_json::json!({ "reason_code": reason_code }),
        ),
    };

    let encoded_call_id = urlencode(call_id);
    let url = format!(
        "{}/manage/calls/{}/{}",
        state.sip_manage_base, encoded_call_id, action
    );
    let token = &state.internal_secret;
    if token.is_empty() {
        return Err("internal_secret missing".to_string());
    }

    let req = state
        .internal_client
        .post(url)
        .header("X-VOS-Token", token)
        .json(&payload);

    let res = req.send().await.map_err(|e| e.to_string())?;
    if res.status().is_success() {
        Ok(())
    } else {
        Err(format!("HTTP error status: {}", res.status()))
    }
}

fn urlencode(s: &str) -> String {
    s.as_bytes()
        .iter()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
                (*byte as char).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use call_core::rwi::RwiEvent;

    #[test]
    fn test_parse_rwi_event_from_json() -> Result<(), Box<dyn std::error::Error>> {
        let json_str = r#"{
            "event_type": "call_started",
            "call_id": "test-call-1",
            "occurred_at_ms": 1720000000000,
            "data": {
                "caller": "1001",
                "callee": "1002",
                "direction": "inbound"
            }
        }"#;
        let event = parse_rwi_event(json_str.as_bytes())?;
        if let RwiEvent::CallStarted {
            call_id,
            caller,
            callee,
            ..
        } = event
        {
            assert_eq!(call_id, "test-call-1");
            assert_eq!(caller, "1001");
            assert_eq!(callee, "1002");
        } else {
            return Err("Unexpected event variant".into());
        }
        Ok(())
    }

    #[test]
    fn rwi_commands_have_independent_permissions() {
        let commands = [
            (
                RwiCommand::BargeIn {
                    call_id: "call-1".to_string(),
                    mode: "listen".to_string(),
                    target_leg: None,
                },
                "calls.barge",
            ),
            (
                RwiCommand::Speak {
                    call_id: "call-1".to_string(),
                    text: "测试".to_string(),
                    voice: None,
                    speed: None,
                },
                "calls.play",
            ),
            (
                RwiCommand::Listen {
                    call_id: "call-1".to_string(),
                    stream_url: "wss://example.invalid/audio".to_string(),
                    format: None,
                },
                "calls.monitor",
            ),
            (
                RwiCommand::Transfer {
                    call_id: "call-1".to_string(),
                    target: "1002".to_string(),
                    transfer_type: None,
                },
                "calls.transfer",
            ),
            (
                RwiCommand::Hangup {
                    call_id: "call-1".to_string(),
                    reason_code: None,
                },
                "calls.terminate",
            ),
        ];

        for (command, permission) in commands {
            assert_eq!(rwi_command_permission(&command), permission);
        }
    }
}
