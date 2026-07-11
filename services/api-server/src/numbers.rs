use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;

use crate::{normalize_page, AppState, PageQuery, PaginatedResponse};
use cdr_core::NumberInventory;

#[derive(Debug, Deserialize)]
pub struct CreateNumberBody {
    pub number: String,
    pub username: Option<String>,
    pub gateway_id: Option<String>,
    pub direction: Option<String>,
    pub max_concurrent: Option<i32>,
    pub status: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateNumberBody {
    pub username: Option<String>,
    pub gateway_id: Option<String>,
    pub direction: Option<String>,
    pub max_concurrent: Option<i32>,
    pub status: Option<String>,
}

type E = (StatusCode, String);
fn err(e: impl std::fmt::Display) -> E {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

pub async fn list_numbers(
    State(state): State<AppState>,
    Query(query): Query<PageQuery>,
) -> Result<Json<PaginatedResponse<NumberInventory>>, E> {
    let (page, page_size, offset) = normalize_page(&query);
    let (items, total) = tokio::try_join!(
        state.store.list_numbers_page(page_size, offset),
        state.store.count_numbers(),
    )
    .map_err(err)?;
    Ok(Json(PaginatedResponse {
        items,
        total,
        page,
        page_size,
    }))
}

pub async fn create_number(
    State(state): State<AppState>,
    Json(b): Json<CreateNumberBody>,
) -> Result<StatusCode, E> {
    state
        .store
        .upsert_number(
            &b.number,
            b.username.as_deref(),
            b.gateway_id.as_deref(),
            b.direction.as_deref(),
            b.max_concurrent,
            &b.status,
        )
        .await
        .map_err(err)?;
    Ok(StatusCode::CREATED)
}

pub async fn update_number(
    State(state): State<AppState>,
    Path(number): Path<String>,
    Json(b): Json<UpdateNumberBody>,
) -> Result<StatusCode, E> {
    state
        .store
        .upsert_number(
            &number,
            b.username.as_deref(),
            b.gateway_id.as_deref(),
            b.direction.as_deref(),
            b.max_concurrent,
            b.status.as_deref().unwrap_or("available"),
        )
        .await
        .map_err(err)?;
    Ok(StatusCode::OK)
}

pub async fn delete_number(
    State(state): State<AppState>,
    Path(number): Path<String>,
) -> Result<StatusCode, E> {
    let deleted = state.store.delete_number(&number).await.map_err(err)?;
    Ok(if deleted {
        StatusCode::OK
    } else {
        StatusCode::NOT_FOUND
    })
}
