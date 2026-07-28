mod config;
mod handlers;
mod media;

use axum::{
    extract::State,
    http::{Request, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

use crate::media::config::MediaConfig;
use crate::media::relay::ai_plugin::AiVoicePluginProxy;
use crate::media::relay::io_uring::IoUringUdpSocket;
use crate::media::relay::XdpMediaEngine;
use crate::media::MediaRelayState;

struct AppState {
    media_relay: MediaRelayState,
    control_token: String,
    /// 已加载的 XDP 内核旁路引擎（按网卡名索引）。
    xdp_engines: tokio::sync::Mutex<HashMap<String, XdpMediaEngine>>,
    /// 活跃的 AI 语音插件代理会话（按会话 ID 索引）。
    ai_proxies: tokio::sync::Mutex<HashMap<String, AiVoicePluginProxy>>,
    /// 已初始化的 io_uring 零拷贝 UDP 通道（按绑定地址索引）。
    io_uring_sockets: tokio::sync::Mutex<HashMap<SocketAddr, IoUringUdpSocket>>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let service_config = config::MediaEdgeServiceConfig::load()?;
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new(config_logging_filter("media_edge=info")))
        .init();

    info!("Starting VOS-RS Next-Gen Media Edge service...");

    let media_relay = MediaRelayState::with_recording_pool(
        service_config.recording_workers,
        service_config.recording_queue_capacity,
    );

    let state = Arc::new(AppState {
        media_relay,
        control_token: service_config.control_token,
        xdp_engines: tokio::sync::Mutex::new(HashMap::new()),
        ai_proxies: tokio::sync::Mutex::new(HashMap::new()),
        io_uring_sockets: tokio::sync::Mutex::new(HashMap::new()),
    });

    let uds_path = service_config.uds_path;
    tokio::spawn(start_uds_server(Arc::clone(&state), uds_path));

    let control_routes = Router::new()
        .route("/allocate_endpoint", post(allocate_endpoint))
        .route("/pair_ports", post(pair_ports))
        .route("/set_target", post(set_target))
        .route("/register_webrtc_session", post(register_webrtc_session))
        .route(
            "/unregister_webrtc_session",
            post(unregister_webrtc_session),
        )
        .route(
            "/set_remote_ice_credentials",
            post(set_remote_ice_credentials),
        )
        .route("/add_remote_candidate", post(add_remote_candidate))
        .route("/clear_target", post(clear_target))
        .route("/start_call_recording", post(start_call_recording))
        .route("/start_monitoring", post(start_monitoring))
        .route("/stop_monitoring", post(stop_monitoring))
        .route("/clear_monitors", post(clear_monitors))
        .route("/register_srtp_session", post(register_srtp_session))
        .route("/register_srtp_offer", post(register_srtp_offer))
        .route("/register_port_codec", post(register_port_codec))
        .route("/start_playback", post(start_playback))
        .route("/stop_playback", post(stop_playback))
        .route("/start_relay_listeners", post(start_relay_listeners))
        .route("/metrics_for_port", post(metrics_for_port))
        .route("/metrics_totals", post(metrics_totals))
        .route(
            "/register_port_dtmf_tracking",
            post(register_port_dtmf_tracking),
        )
        .route("/get_dtmf_digits", post(get_dtmf_digits))
        .route("/clear_dtmf_digits", post(clear_dtmf_digits))
        .route("/take_dtmf_events", post(take_dtmf_events))
        .route("/clear_dtmf_events", post(clear_dtmf_events))
        // 扩展控制端点：WebRTC 诊断、会议管理、eBPF/XDP、AI 插件、io_uring、SIP INFO DTMF
        .route("/webrtc_diagnostics", post(handlers::webrtc_diagnostics))
        .route(
            "/webrtc_diagnostics_all",
            get(handlers::webrtc_diagnostics_all),
        )
        .route("/join_conference", post(handlers::join_conference))
        .route("/leave_conference", post(handlers::leave_conference))
        .route(
            "/set_participant_mute",
            post(handlers::set_participant_mute),
        )
        .route("/list_conferences", get(handlers::list_conferences))
        .route("/conference_for_port", post(handlers::conference_for_port))
        .route("/init_xdp", post(handlers::init_xdp))
        .route("/register_xdp_rule", post(handlers::register_xdp_rule))
        .route("/unregister_xdp_rule", post(handlers::unregister_xdp_rule))
        .route("/xdp_status", post(handlers::xdp_status))
        .route("/start_ai_plugin", post(handlers::start_ai_plugin))
        .route("/send_ai_upstream", post(handlers::send_ai_upstream))
        .route(
            "/try_recv_ai_downstream",
            post(handlers::try_recv_ai_downstream),
        )
        .route("/init_io_uring", post(handlers::init_io_uring))
        .route("/poll_io_uring", post(handlers::poll_io_uring))
        .route(
            "/register_info_dtmf_digit",
            post(handlers::register_info_dtmf_digit),
        )
        .route_layer(middleware::from_fn_with_state(
            Arc::clone(&state),
            authorize_control,
        ));
    let app = Router::new()
        .route("/health", get(health))
        .merge(control_routes)
        .with_state(state);

    let addr = service_config.control_bind;
    info!(%addr, "Media Edge Web API listening");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

fn config_logging_filter(default: &str) -> String {
    let path = std::env::var("VOS_RS_CONFIG_FILE").unwrap_or_else(|_| "config.yaml".to_string());
    std::fs::read_to_string(path)
        .ok()
        .and_then(|content| serde_yaml::from_str::<serde_yaml::Value>(&content).ok())
        .and_then(|root| {
            root.get("logging")?
                .get("filter")?
                .as_str()
                .map(str::to_owned)
        })
        .filter(|filter| !filter.trim().is_empty())
        .unwrap_or_else(|| default.to_string())
}

async fn health() -> &'static str {
    "ok"
}

async fn authorize_control(
    State(state): State<Arc<AppState>>,
    request: Request<axum::body::Body>,
    next: Next,
) -> Response {
    if state.control_token.is_empty() {
        return next.run(request).await;
    }
    let supplied = request
        .headers()
        .get("x-vos-media-token")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if constant_time_eq(supplied.as_bytes(), state.control_token.as_bytes()) {
        next.run(request).await
    } else {
        StatusCode::UNAUTHORIZED.into_response()
    }
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

#[cfg(test)]
mod control_auth_tests {
    use super::constant_time_eq;

    #[test]
    fn test_control_token_comparison_rejects_wrong_values() {
        assert!(constant_time_eq(b"cluster-secret", b"cluster-secret"));
        assert!(!constant_time_eq(b"cluster-secret", b"cluster-secrex"));
        assert!(!constant_time_eq(b"short", b"cluster-secret"));
    }
}

#[derive(serde::Deserialize)]
struct AllocateEndpointReq {
    config: MediaConfig,
}

#[derive(serde::Serialize)]
struct AllocateEndpointResp {
    port: u16,
}

async fn allocate_endpoint(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    Json(payload): Json<AllocateEndpointReq>,
) -> Json<Result<AllocateEndpointResp, String>> {
    match state.media_relay.allocate_endpoint(&payload.config) {
        Ok(ep) => {
            info!(port = ep.port, "allocated media relay endpoint");
            Json(Ok(AllocateEndpointResp { port: ep.port }))
        }
        Err(e) => {
            error!(%e, "Failed to allocate endpoint");
            Json(Err(e.to_string()))
        }
    }
}

#[derive(serde::Deserialize)]
struct PairPortsReq {
    port_a: u16,
    port_b: u16,
}

async fn pair_ports(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    Json(payload): Json<PairPortsReq>,
) -> Json<bool> {
    state.media_relay.pair_ports(payload.port_a, payload.port_b);
    Json(true)
}

#[derive(serde::Deserialize)]
struct SetTargetReq {
    local_port: u16,
    target: SocketAddr,
}

async fn set_target(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    Json(payload): Json<SetTargetReq>,
) -> Json<Result<bool, String>> {
    let local_ep = sdp_core::RtpEndpoint::new("0.0.0.0".to_string(), payload.local_port);
    let target_ep =
        sdp_core::RtpEndpoint::new(payload.target.ip().to_string(), payload.target.port());
    match state.media_relay.set_target(&local_ep, &target_ep) {
        Ok(_) => Json(Ok(true)),
        Err(e) => Json(Err(e.to_string())),
    }
}

#[derive(serde::Deserialize)]
struct WebRtcSessionReq {
    port: u16,
}

async fn register_webrtc_session(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    Json(payload): Json<WebRtcSessionReq>,
) -> Json<Result<crate::media::relay::webrtc::WebRtcSessionDescription, String>> {
    Json(
        state
            .media_relay
            .register_webrtc_session(payload.port)
            .map_err(|error| error.to_string()),
    )
}

async fn unregister_webrtc_session(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    Json(payload): Json<WebRtcSessionReq>,
) -> Json<bool> {
    state.media_relay.unregister_webrtc_session(payload.port);
    Json(true)
}

#[derive(serde::Deserialize)]
struct ClearTargetReq {
    port: u16,
}

async fn clear_target(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    Json(payload): Json<ClearTargetReq>,
) -> Json<bool> {
    state.media_relay.clear_target(payload.port);
    Json(true)
}

#[derive(serde::Deserialize)]
struct StartCallRecordingReq {
    port_a: u16,
    port_b: u16,
    wav_path: std::path::PathBuf,
    min_free_bytes: u64,
    max_file_bytes: u64,
    max_duration_secs: u64,
    format_str: String,
}

async fn start_call_recording(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    Json(payload): Json<StartCallRecordingReq>,
) -> Json<Result<bool, String>> {
    let mut config = MediaConfig::new_with_symmetric_learning("127.0.0.1", 10000, 65000, true);
    config.recording_enabled = true;
    config.recording_dir = payload
        .wav_path
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .to_path_buf();
    config.recording_min_free_bytes = payload.min_free_bytes;
    config.recording_max_file_bytes = payload.max_file_bytes;
    config.recording_max_duration_secs = payload.max_duration_secs;
    config.recording_format = payload.format_str;

    match state.media_relay.start_call_recording(
        "remote_call",
        payload.port_a,
        payload.port_b,
        &config,
    ) {
        Ok(_) => Json(Ok(true)),
        Err(e) => Json(Err(e.to_string())),
    }
}

#[derive(serde::Deserialize)]
struct ClearMonitorsReq {
    port: u16,
}

async fn clear_monitors(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    Json(payload): Json<ClearMonitorsReq>,
) -> Json<bool> {
    state.media_relay.clear_monitors(payload.port);
    Json(true)
}

#[derive(serde::Deserialize)]
struct MetricsForPortReq {
    port: u16,
}

async fn metrics_for_port(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    Json(payload): Json<MetricsForPortReq>,
) -> Json<Option<crate::media::metrics::MediaRelayMetrics>> {
    Json(Some(state.media_relay.metrics_for_port(payload.port)))
}

async fn metrics_totals(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
) -> Json<crate::media::metrics::MediaRelayMetrics> {
    Json(state.media_relay.metrics_totals())
}

async fn start_uds_server(state: Arc<AppState>, uds_path: String) {
    let _ = std::fs::remove_file(&uds_path);
    let listener = match tokio::net::UnixListener::bind(&uds_path) {
        Ok(l) => l,
        Err(e) => {
            error!(%uds_path, %e, "Failed to bind UDS listener");
            return;
        }
    };
    info!(%uds_path, "Media Edge UDS Control plane listening");

    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let state_clone = Arc::clone(&state);
                tokio::spawn(async move {
                    if let Err(e) = handle_uds_client(state_clone, stream).await {
                        tracing::debug!(%e, "UDS client connection error");
                    }
                });
            }
            Err(e) => {
                error!(%e, "Failed to accept UDS stream");
            }
        }
    }
}

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

#[derive(serde::Deserialize)]
struct UdsRequest {
    method: String,
    params: serde_json::Value,
}

#[derive(serde::Serialize)]
struct UdsResponse {
    result: Option<serde_json::Value>,
    error: Option<String>,
}

async fn handle_uds_client(
    state: Arc<AppState>,
    mut stream: tokio::net::UnixStream,
) -> Result<(), Box<dyn std::error::Error>> {
    let (reader, mut writer) = stream.split();
    let mut reader = BufReader::new(reader);
    let mut line = String::new();
    while reader.read_line(&mut line).await? > 0 {
        let req: UdsRequest = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                let resp = UdsResponse {
                    result: None,
                    error: Some(format!("Invalid JSON: {e}")),
                };
                let resp_str = serde_json::to_string(&resp)? + "\n";
                writer.write_all(resp_str.as_bytes()).await?;
                line.clear();
                continue;
            }
        };

        let resp = match req.method.as_str() {
            "allocate_endpoint" => {
                match serde_json::from_value::<AllocateEndpointReq>(req.params) {
                    Ok(payload) => match state.media_relay.allocate_endpoint(&payload.config) {
                        Ok(ep) => UdsResponse {
                            result: Some(serde_json::json!({ "port": ep.port })),
                            error: None,
                        },
                        Err(e) => UdsResponse {
                            result: None,
                            error: Some(e.to_string()),
                        },
                    },
                    Err(e) => UdsResponse {
                        result: None,
                        error: Some(e.to_string()),
                    },
                }
            }
            "pair_ports" => match serde_json::from_value::<PairPortsReq>(req.params) {
                Ok(payload) => {
                    state.media_relay.pair_ports(payload.port_a, payload.port_b);
                    UdsResponse {
                        result: Some(serde_json::json!(true)),
                        error: None,
                    }
                }
                Err(e) => UdsResponse {
                    result: None,
                    error: Some(e.to_string()),
                },
            },
            "set_target" => match serde_json::from_value::<SetTargetReq>(req.params) {
                Ok(payload) => {
                    let local_ep =
                        sdp_core::RtpEndpoint::new("0.0.0.0".to_string(), payload.local_port);
                    let target_ep = sdp_core::RtpEndpoint::new(
                        payload.target.ip().to_string(),
                        payload.target.port(),
                    );
                    match state.media_relay.set_target(&local_ep, &target_ep) {
                        Ok(_) => UdsResponse {
                            result: Some(serde_json::json!(true)),
                            error: None,
                        },
                        Err(e) => UdsResponse {
                            result: None,
                            error: Some(e.to_string()),
                        },
                    }
                }
                Err(e) => UdsResponse {
                    result: None,
                    error: Some(e.to_string()),
                },
            },
            "register_webrtc_session" => {
                match serde_json::from_value::<WebRtcSessionReq>(req.params) {
                    Ok(payload) => match state.media_relay.register_webrtc_session(payload.port) {
                        Ok(description) => UdsResponse {
                            result: Some(serde_json::to_value(description)?),
                            error: None,
                        },
                        Err(error) => UdsResponse {
                            result: None,
                            error: Some(error.to_string()),
                        },
                    },
                    Err(error) => UdsResponse {
                        result: None,
                        error: Some(error.to_string()),
                    },
                }
            }
            "unregister_webrtc_session" => {
                match serde_json::from_value::<WebRtcSessionReq>(req.params) {
                    Ok(payload) => {
                        state.media_relay.unregister_webrtc_session(payload.port);
                        UdsResponse {
                            result: Some(serde_json::json!(true)),
                            error: None,
                        }
                    }
                    Err(error) => UdsResponse {
                        result: None,
                        error: Some(error.to_string()),
                    },
                }
            }
            "clear_target" => match serde_json::from_value::<ClearTargetReq>(req.params) {
                Ok(payload) => {
                    state.media_relay.clear_target(payload.port);
                    UdsResponse {
                        result: Some(serde_json::json!(true)),
                        error: None,
                    }
                }
                Err(e) => UdsResponse {
                    result: None,
                    error: Some(e.to_string()),
                },
            },
            "start_call_recording" => {
                match serde_json::from_value::<StartCallRecordingReq>(req.params) {
                    Ok(payload) => {
                        let mut config = MediaConfig::new_with_symmetric_learning(
                            "127.0.0.1",
                            10000,
                            65000,
                            true,
                        );
                        config.recording_enabled = true;
                        config.recording_dir = payload
                            .wav_path
                            .parent()
                            .unwrap_or(std::path::Path::new("."))
                            .to_path_buf();
                        config.recording_min_free_bytes = payload.min_free_bytes;
                        config.recording_max_file_bytes = payload.max_file_bytes;
                        config.recording_max_duration_secs = payload.max_duration_secs;
                        config.recording_format = payload.format_str;

                        match state.media_relay.start_call_recording(
                            "remote_call",
                            payload.port_a,
                            payload.port_b,
                            &config,
                        ) {
                            Ok(_) => UdsResponse {
                                result: Some(serde_json::json!(true)),
                                error: None,
                            },
                            Err(e) => UdsResponse {
                                result: None,
                                error: Some(e.to_string()),
                            },
                        }
                    }
                    Err(e) => UdsResponse {
                        result: None,
                        error: Some(e.to_string()),
                    },
                }
            }
            "clear_monitors" => match serde_json::from_value::<ClearMonitorsReq>(req.params) {
                Ok(payload) => {
                    state.media_relay.clear_monitors(payload.port);
                    UdsResponse {
                        result: Some(serde_json::json!(true)),
                        error: None,
                    }
                }
                Err(e) => UdsResponse {
                    result: None,
                    error: Some(e.to_string()),
                },
            },
            "metrics_for_port" => match serde_json::from_value::<MetricsForPortReq>(req.params) {
                Ok(payload) => UdsResponse {
                    result: Some(serde_json::to_value(
                        state.media_relay.metrics_for_port(payload.port),
                    )?),
                    error: None,
                },
                Err(e) => UdsResponse {
                    result: None,
                    error: Some(e.to_string()),
                },
            },
            "metrics_totals" => match serde_json::to_value(state.media_relay.metrics_totals()) {
                Ok(v) => UdsResponse {
                    result: Some(v),
                    error: None,
                },
                Err(e) => UdsResponse {
                    result: None,
                    error: Some(e.to_string()),
                },
            },
            "register_port_dtmf_tracking" => {
                match serde_json::from_value::<RegisterPortDtmfTrackingReq>(req.params) {
                    Ok(payload) => {
                        state.media_relay.register_port_dtmf_tracking(
                            &payload.call_id,
                            payload.port,
                            payload.payload_type,
                        );
                        UdsResponse {
                            result: Some(serde_json::json!(true)),
                            error: None,
                        }
                    }
                    Err(e) => UdsResponse {
                        result: None,
                        error: Some(e.to_string()),
                    },
                }
            }
            "get_dtmf_digits" => match serde_json::from_value::<CallIdReq>(req.params) {
                Ok(payload) => UdsResponse {
                    result: Some(serde_json::to_value(
                        state.media_relay.get_dtmf_digits(&payload.call_id),
                    )?),
                    error: None,
                },
                Err(e) => UdsResponse {
                    result: None,
                    error: Some(e.to_string()),
                },
            },
            "clear_dtmf_digits" => match serde_json::from_value::<CallIdReq>(req.params) {
                Ok(payload) => {
                    state.media_relay.clear_dtmf_digits(&payload.call_id);
                    UdsResponse {
                        result: Some(serde_json::json!(true)),
                        error: None,
                    }
                }
                Err(e) => UdsResponse {
                    result: None,
                    error: Some(e.to_string()),
                },
            },
            "take_dtmf_events" => match serde_json::from_value::<CallIdReq>(req.params) {
                Ok(payload) => UdsResponse {
                    result: Some(serde_json::to_value(
                        state.media_relay.take_dtmf_events(&payload.call_id),
                    )?),
                    error: None,
                },
                Err(e) => UdsResponse {
                    result: None,
                    error: Some(e.to_string()),
                },
            },
            "clear_dtmf_events" => match serde_json::from_value::<CallIdReq>(req.params) {
                Ok(payload) => {
                    state.media_relay.clear_dtmf_events(&payload.call_id);
                    UdsResponse {
                        result: Some(serde_json::json!(true)),
                        error: None,
                    }
                }
                Err(e) => UdsResponse {
                    result: None,
                    error: Some(e.to_string()),
                },
            },
            _ => UdsResponse {
                result: None,
                error: Some(format!("Unknown method: {}", req.method)),
            },
        };

        let resp_str = serde_json::to_string(&resp)? + "\n";
        writer.write_all(resp_str.as_bytes()).await?;
        line.clear();
    }
    Ok(())
}

#[derive(serde::Deserialize)]
struct RegisterPortDtmfTrackingReq {
    call_id: String,
    port: u16,
    payload_type: u8,
}

#[derive(serde::Deserialize)]
struct CallIdReq {
    call_id: String,
}

async fn register_port_dtmf_tracking(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    Json(payload): Json<RegisterPortDtmfTrackingReq>,
) -> Json<bool> {
    state.media_relay.register_port_dtmf_tracking(
        &payload.call_id,
        payload.port,
        payload.payload_type,
    );
    Json(true)
}

async fn get_dtmf_digits(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    Json(payload): Json<CallIdReq>,
) -> Json<Option<String>> {
    Json(state.media_relay.get_dtmf_digits(&payload.call_id))
}

async fn clear_dtmf_digits(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    Json(payload): Json<CallIdReq>,
) -> Json<bool> {
    state.media_relay.clear_dtmf_digits(&payload.call_id);
    Json(true)
}

async fn take_dtmf_events(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    Json(payload): Json<CallIdReq>,
) -> Json<Vec<cdr_core::DtmfEventRecord>> {
    Json(state.media_relay.take_dtmf_events(&payload.call_id))
}

async fn clear_dtmf_events(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    Json(payload): Json<CallIdReq>,
) -> Json<bool> {
    state.media_relay.clear_dtmf_events(&payload.call_id);
    Json(true)
}

// ===== 监控（旁听）端点 =====

#[derive(serde::Deserialize)]
struct MonitorReq {
    port: u16,
    supervisor: SocketAddr,
}

async fn start_monitoring(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    Json(payload): Json<MonitorReq>,
) -> Json<bool> {
    state
        .media_relay
        .start_monitoring(payload.port, payload.supervisor);
    Json(true)
}

async fn stop_monitoring(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    Json(payload): Json<MonitorReq>,
) -> Json<bool> {
    state
        .media_relay
        .stop_monitoring(payload.port, payload.supervisor);
    Json(true)
}

// ===== SRTP / 编解码注册端点 =====

#[derive(serde::Deserialize)]
struct RegisterSrtpSessionReq {
    relay_port: u16,
    suite: String,
    key_params: String,
    ssrc: u32,
}

async fn register_srtp_session(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    Json(payload): Json<RegisterSrtpSessionReq>,
) -> Json<Result<bool, String>> {
    match state.media_relay.register_srtp_session(
        payload.relay_port,
        &payload.suite,
        &payload.key_params,
        payload.ssrc,
    ) {
        Ok(()) => Json(Ok(true)),
        Err(e) => Json(Err(e.to_string())),
    }
}

#[derive(serde::Deserialize)]
struct RegisterSrtpOfferReq {
    relay_port: u16,
    suite: String,
    key_params: String,
}

async fn register_srtp_offer(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    Json(payload): Json<RegisterSrtpOfferReq>,
) -> Json<bool> {
    state
        .media_relay
        .register_srtp_offer(payload.relay_port, &payload.suite, &payload.key_params);
    Json(true)
}

#[derive(serde::Deserialize)]
struct RegisterPortCodecReq {
    port: u16,
    codec: rtp_core::AudioCodec,
}

async fn register_port_codec(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    Json(payload): Json<RegisterPortCodecReq>,
) -> Json<bool> {
    state
        .media_relay
        .register_port_codec(payload.port, payload.codec);
    Json(true)
}

// ===== WebRTC ICE 候选注入端点 =====

#[derive(serde::Deserialize)]
struct SetRemoteIceCredentialsReq {
    port: u16,
    ufrag: String,
    password: String,
}

async fn set_remote_ice_credentials(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    Json(payload): Json<SetRemoteIceCredentialsReq>,
) -> Json<Result<bool, String>> {
    let Some(session) = state
        .media_relay
        .webrtc_sessions
        .get(&payload.port)
        .map(|entry| entry.clone())
    else {
        return Json(Err(format!("端口 {} 未注册 WebRTC 会话", payload.port)));
    };
    session
        .set_remote_ice_credentials(payload.ufrag, payload.password)
        .await;
    Json(Ok(true))
}

#[derive(serde::Deserialize)]
struct AddRemoteCandidateReq {
    port: u16,
    /// SDP `a=candidate:` 行内容，由 `parse_candidate_line` 解析。
    candidate_line: String,
}

async fn add_remote_candidate(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    Json(payload): Json<AddRemoteCandidateReq>,
) -> Json<Result<bool, String>> {
    let candidate = match crate::media::relay::webrtc::parse_candidate_line(&payload.candidate_line)
    {
        Ok(c) => c,
        Err(e) => return Json(Err(e)),
    };
    let Some(session) = state
        .media_relay
        .webrtc_sessions
        .get(&payload.port)
        .map(|entry| entry.clone())
    else {
        return Json(Err(format!("端口 {} 未注册 WebRTC 会话", payload.port)));
    };
    session.add_remote_candidate(candidate).await;
    Json(Ok(true))
}

// ===== 音频播放（IVR/放音）端点 =====

#[derive(serde::Deserialize)]
struct StartPlaybackReq {
    port: u16,
    file_path: std::path::PathBuf,
    mode: crate::media::relay::PlaybackMode,
    loop_playback: bool,
}

async fn start_playback(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    Json(payload): Json<StartPlaybackReq>,
) -> Json<Result<bool, String>> {
    match state
        .media_relay
        .start_playback(
            payload.port,
            payload.file_path,
            payload.mode,
            payload.loop_playback,
        )
        .await
    {
        Ok(()) => Json(Ok(true)),
        Err(e) => Json(Err(e)),
    }
}

#[derive(serde::Deserialize)]
struct StopPlaybackReq {
    port: u16,
}

async fn stop_playback(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    Json(payload): Json<StopPlaybackReq>,
) -> Json<bool> {
    state.media_relay.stop_playback(payload.port);
    Json(true)
}

// ===== RTP 中继监听器启动端点 =====

#[derive(serde::Deserialize)]
struct StartRelayListenersReq {
    port_min: u16,
    port_max: u16,
    symmetric_rtp_learning: bool,
    anti_spoofing: bool,
    #[serde(default = "default_source_relearn_secs")]
    source_relearn_after_secs: u64,
}

fn default_source_relearn_secs() -> u64 {
    30
}

async fn start_relay_listeners(
    axum::extract::State(state): axum::extract::State<Arc<AppState>>,
    Json(payload): Json<StartRelayListenersReq>,
) -> Json<Result<usize, String>> {
    let mut config = MediaConfig::new_with_symmetric_learning(
        "0.0.0.0",
        payload.port_min,
        payload.port_max,
        payload.symmetric_rtp_learning,
    );
    config.anti_spoofing = payload.anti_spoofing;
    config.source_relearn_after_secs = payload.source_relearn_after_secs;
    match crate::media::relay::spawn_rtp_relay_listeners(&config, state.media_relay.clone()).await {
        Ok(handles) => {
            let count = handles.len();
            for handle in handles {
                tokio::spawn(async move {
                    let _ = handle.await;
                });
            }
            Json(Ok(count))
        }
        Err(e) => Json(Err(e.to_string())),
    }
}
