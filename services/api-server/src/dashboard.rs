use axum::{
    extract::State,
    response::sse::{Event, KeepAlive, Sse},
    Json,
};
use cdr_core::{DashboardStats, HourlyTrend};
use futures::stream::{self, Stream};

use crate::{ApiError, AppState};

pub async fn get_dashboard_stats(
    State(state): State<AppState>,
) -> Result<Json<DashboardStats>, ApiError> {
    let active_calls = {
        let url = format!("{}/manage/active-calls", state.sip_manage_base);
        let token = &state.internal_secret;
        let request = state.internal_client.get(&url);
        let request = if !token.is_empty() {
            request.header("X-VOS-Token", token)
        } else {
            return state
                .store
                .get_dashboard_stats(0)
                .await
                .map(Json)
                .map_err(|e| ApiError {
                    error: e.to_string(),
                });
        };
        match request.send().await {
            Ok(resp) => resp
                .json::<Vec<serde_json::Value>>()
                .await
                .map(|calls| calls.len() as i64)
                .unwrap_or(0),
            Err(_) => 0,
        }
    };
    state
        .store
        .get_dashboard_stats(active_calls)
        .await
        .map(Json)
        .map_err(|e| ApiError {
            error: e.to_string(),
        })
}

pub async fn get_dashboard_trend(
    State(state): State<AppState>,
) -> Result<Json<Vec<HourlyTrend>>, ApiError> {
    state
        .store
        .get_hourly_trend()
        .await
        .map(Json)
        .map_err(|e| ApiError {
            error: e.to_string(),
        })
}

pub async fn dashboard_events(
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>> {
    let state_clone = state.clone();
    let stream = stream::unfold(
        (
            state_clone,
            tokio::time::interval(std::time::Duration::from_secs(2)),
        ),
        |(state, mut interval)| async move {
            interval.tick().await;

            let token = &state.internal_secret;
            let active_calls = if !token.is_empty() {
                match state
                    .internal_client
                    .get(format!("{}/manage/active-calls", state.sip_manage_base))
                    .header("X-VOS-Token", token)
                    .send()
                    .await
                {
                    Ok(resp) => resp
                        .json::<Vec<serde_json::Value>>()
                        .await
                        .map(|v| v.len() as u32)
                        .unwrap_or(0),
                    Err(_) => 0,
                }
            } else {
                0
            };

            let trunk_online_count = match state.store.list_gateways_full().await {
                Ok(gateways) => gateways
                    .iter()
                    .filter(|gateway| gateway.enabled != Some(false))
                    .filter(|gateway| gateway.circuit_state.as_deref() != Some("open"))
                    .count() as u32,
                Err(_) => 0,
            };

            let data = serde_json::json!({
                "active_calls": active_calls,
                "trunk_online_count": trunk_online_count,
                "timestamp": time::OffsetDateTime::now_utc().unix_timestamp(),
            });

            let event = Event::default().data(data.to_string());
            Some((Ok(event), (state, interval)))
        },
    );

    Sse::new(stream).keep_alive(KeepAlive::default())
}
