use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;

use crate::AppState;
use cdr_core::NumberInventory;

#[derive(Debug, Deserialize)]
pub struct CreateNumberBody {
    pub number: String,
    pub username: Option<String>,
    pub status: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateNumberBody {
    pub username: Option<String>,
    pub status: String,
}

type E = (StatusCode, String);
fn err(e: impl std::fmt::Display) -> E {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

pub async fn list_numbers(State(state): State<AppState>) -> Result<Json<Vec<NumberInventory>>, E> {
    state.store.list_numbers().await.map(Json).map_err(err)
}

pub async fn create_number(
    State(state): State<AppState>,
    Json(b): Json<CreateNumberBody>,
) -> Result<StatusCode, E> {
    state
        .store
        .upsert_number(&b.number, b.username.as_deref(), &b.status)
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
        .upsert_number(&number, b.username.as_deref(), &b.status)
        .await
        .map_err(err)?;
    Ok(StatusCode::OK)
}

pub async fn delete_number(
    State(state): State<AppState>,
    Path(number): Path<String>,
) -> Result<StatusCode, E> {
    let deleted = state
        .store
        .delete_number(&number)
        .await
        .map_err(err)?;
    Ok(if deleted {
        StatusCode::OK
    } else {
        StatusCode::NOT_FOUND
    })
}
