use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use cdr_core::SipRoute;
use serde::Deserialize;

use crate::{normalize_page, ApiError, AppState, PageQuery, PaginatedResponse};

#[derive(Debug, Deserialize)]
pub struct CreateRouteRequest {
    pub id: String,
    pub prefix: String,
    pub priority: i32,
    pub gateway_id: String,
    pub cost: f64,
    pub weight: Option<i32>,
    pub time_start: Option<String>,
    pub time_end: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateRouteRequest {
    pub prefix: String,
    pub priority: i32,
    pub gateway_id: String,
    pub cost: f64,
    pub weight: Option<i32>,
    pub time_start: Option<String>,
    pub time_end: Option<String>,
}

pub async fn publish_route_reload(nats: &Option<async_nats::Client>) {
    if let Some(client) = nats {
        if let Err(e) = client
            .publish("vos_rs.routing.reload", axum::body::Bytes::from("reload"))
            .await
        {
            tracing::warn!(error = %e, "NATS 路由重载广播发布失败");
        }
    }
}

pub async fn list_routes(
    State(state): State<AppState>,
    Query(query): Query<PageQuery>,
) -> Result<Json<PaginatedResponse<SipRoute>>, ApiError> {
    let (page, page_size, offset) = normalize_page(&query);
    let (items, total) = tokio::try_join!(
        state.store.list_routes_page(page_size, offset),
        state.store.count_routes(),
    )
    .map_err(|e| ApiError {
        error: e.to_string(),
    })?;
    Ok(Json(PaginatedResponse {
        items,
        total,
        page,
        page_size,
    }))
}

pub async fn create_route(
    State(state): State<AppState>,
    Json(req): Json<CreateRouteRequest>,
) -> Result<StatusCode, ApiError> {
    let weight = req.weight.unwrap_or(100).clamp(1, 10000);
    state
        .store
        .insert_route_with_cost(
            &req.id,
            &req.prefix,
            req.priority,
            &req.gateway_id,
            req.cost,
            weight,
            req.time_start.as_deref(),
            req.time_end.as_deref(),
        )
        .await
        .map_err(|e| ApiError {
            error: e.to_string(),
        })?;
    publish_route_reload(&state.nats_client).await;
    Ok(StatusCode::CREATED)
}

pub async fn update_route(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateRouteRequest>,
) -> Result<StatusCode, ApiError> {
    let weight = req.weight.unwrap_or(100).clamp(1, 10000);
    state
        .store
        .insert_route_with_cost(
            &id,
            &req.prefix,
            req.priority,
            &req.gateway_id,
            req.cost,
            weight,
            req.time_start.as_deref(),
            req.time_end.as_deref(),
        )
        .await
        .map_err(|e| ApiError {
            error: e.to_string(),
        })?;
    publish_route_reload(&state.nats_client).await;
    Ok(StatusCode::OK)
}

pub async fn delete_route(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let deleted = state.store.delete_route(&id).await.map_err(|e| ApiError {
        error: e.to_string(),
    })?;
    if deleted {
        publish_route_reload(&state.nats_client).await;
        Ok(StatusCode::OK)
    } else {
        Ok(StatusCode::NOT_FOUND)
    }
}
