mod config;
mod media;

use axum::{
    extract::State,
    http::{Request, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use std::net::SocketAddr;
use std::sync::Arc;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

use crate::media::config::MediaConfig;
use crate::media::MediaRelayState;

struct AppState {
    media_relay: MediaRelayState,
    control_token: String,
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
        .route("/clear_target", post(clear_target))
        .route("/start_call_recording", post(start_call_recording))
        .route("/clear_monitors", post(clear_monitors))
        .route("/metrics_for_port", post(metrics_for_port))
        .route("/metrics_totals", post(metrics_totals))
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
