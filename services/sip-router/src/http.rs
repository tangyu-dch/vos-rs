use axum::{extract::State, http::StatusCode, response::IntoResponse, routing::get, Router};

use crate::{discovery::SharedNodes, metrics};

#[derive(Clone)]
struct HttpState {
    nodes: SharedNodes,
    redis: redis::aio::ConnectionManager,
}

pub(crate) async fn start(
    bind: &str,
    nodes: SharedNodes,
    redis: redis::aio::ConnectionManager,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let listener = tokio::net::TcpListener::bind(bind).await?;
    let app = Router::new()
        .route("/health", get(health))
        .route("/ready", get(ready))
        .route("/metrics", get(prometheus))
        .with_state(HttpState { nodes, redis });
    tracing::info!(%bind, "sip-router 管理与指标接口已启动");
    tokio::spawn(async move {
        if let Err(error) = axum::serve(listener, app).await {
            tracing::error!(%error, "sip-router 管理接口异常退出");
        }
    });
    Ok(())
}

async fn health() -> &'static str {
    "ok"
}

async fn ready(State(state): State<HttpState>) -> impl IntoResponse {
    let mut redis = state.redis.clone();
    let redis_ok = redis::cmd("PING")
        .query_async::<String>(&mut redis)
        .await
        .is_ok();
    let nodes = state.nodes.read().await.len();
    if redis_ok && nodes > 0 {
        (StatusCode::OK, format!("ready: {nodes} nodes"))
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            format!("not ready: redis={redis_ok}, nodes={nodes}"),
        )
    }
}

async fn prometheus() -> impl IntoResponse {
    (
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; version=0.0.4",
        )],
        metrics::render(),
    )
}
