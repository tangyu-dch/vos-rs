use axum::{
    extract::State,
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::IntoResponse,
};
use std::env;

use crate::metrics::{MediaMetricsSnapshot, Metrics};
use crate::AppState;

pub async fn health() -> &'static str {
    "OK"
}

/// 就绪探针：进程存活不代表数据库可用，只有依赖检查通过才返回 200。
pub async fn ready(State(state): State<AppState>) -> StatusCode {
    match state.store.ping().await {
        Ok(()) => StatusCode::OK,
        Err(error) => {
            tracing::warn!(%error, "API 就绪检查失败");
            StatusCode::SERVICE_UNAVAILABLE
        }
    }
}

pub async fn prometheus_metrics(State(state): State<AppState>) -> impl IntoResponse {
    let url = format!("{}/manage/media-metrics", state.sip_manage_base);
    let secret = match env::var("VOS_RS_INTERNAL_SECRET") {
        Ok(val) => val,
        Err(_) => {
            tracing::warn!("VOS_RS_INTERNAL_SECRET 未配置，跳过 sip-edge 指标拉取");
            let mut headers = HeaderMap::new();
            headers.insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/plain; version=0.0.4; charset=utf-8"),
            );
            return (headers, Metrics::encode_metrics());
        }
    };
    match state
        .internal_client
        .get(&url)
        .header("X-VOS-Token", secret)
        .send()
        .await
    {
        Ok(response) => match response.json::<MediaMetricsSnapshot>().await {
            Ok(snapshot) => Metrics::update_media_metrics(&snapshot),
            Err(error) => tracing::debug!(%error, "failed to decode sip-edge media metrics"),
        },
        Err(error) => tracing::debug!(%error, "failed to fetch sip-edge media metrics"),
    }

    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; version=0.0.4; charset=utf-8"),
    );
    (headers, Metrics::encode_metrics())
}
