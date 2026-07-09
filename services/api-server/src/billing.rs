use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use time::{Duration, OffsetDateTime};

use crate::{parse_dt, AppState};
use cdr_core::{BillingAccount, BillingRate, LedgerEntry, ReconcileResult};

#[derive(Debug, Deserialize)]
pub struct RateBody {
    pub id: String,
    pub prefix: String,
    pub rate_per_minute: f64,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RateUpdate {
    pub prefix: String,
    pub rate_per_minute: f64,
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreditBody {
    pub amount: f64,
}

#[derive(Debug, Deserialize)]
pub struct LedgerQuery {
    pub username: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ReconcileQuery {
    pub start_time: Option<String>,
    pub end_time: Option<String>,
}

#[derive(Serialize)]
pub struct CreditResult {
    pub username: String,
    pub balance: f64,
}

type E = (StatusCode, String);
fn err(e: impl std::fmt::Display) -> E {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

pub async fn list_rates(State(state): State<AppState>) -> Result<Json<Vec<BillingRate>>, E> {
    state.store.list_rates().await.map(Json).map_err(err)
}

pub async fn create_rate(
    State(state): State<AppState>,
    Json(b): Json<RateBody>,
) -> Result<StatusCode, E> {
    state
        .store
        .upsert_rate(
            &b.id,
            &b.prefix,
            b.rate_per_minute,
            b.description.as_deref(),
        )
        .await
        .map_err(err)?;
    Ok(StatusCode::CREATED)
}

pub async fn update_rate(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(b): Json<RateUpdate>,
) -> Result<StatusCode, E> {
    state
        .store
        .upsert_rate(&id, &b.prefix, b.rate_per_minute, b.description.as_deref())
        .await
        .map_err(err)?;
    Ok(StatusCode::OK)
}

pub async fn delete_rate(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, E> {
    let deleted = state.store.delete_rate(&id).await.map_err(err)?;
    Ok(if deleted {
        StatusCode::OK
    } else {
        StatusCode::NOT_FOUND
    })
}

pub async fn list_accounts(State(state): State<AppState>) -> Result<Json<Vec<BillingAccount>>, E> {
    state.store.list_accounts().await.map(Json).map_err(err)
}

pub async fn credit_account(
    State(state): State<AppState>,
    Path(username): Path<String>,
    Json(b): Json<CreditBody>,
) -> Result<Json<CreditResult>, E> {
    let balance = state
        .store
        .credit_account(&username, b.amount)
        .await
        .map_err(err)?;
    Ok(Json(CreditResult { username, balance }))
}

pub async fn list_ledger(
    State(state): State<AppState>,
    Query(q): Query<LedgerQuery>,
) -> Result<Json<Vec<LedgerEntry>>, E> {
    state
        .store
        .list_ledger(q.username.as_deref())
        .await
        .map(Json)
        .map_err(err)
}

pub async fn reconcile(
    State(state): State<AppState>,
    Query(q): Query<ReconcileQuery>,
) -> Result<Json<ReconcileResult>, E> {
    let end = q
        .end_time
        .as_deref()
        .and_then(parse_dt)
        .unwrap_or_else(OffsetDateTime::now_utc);
    let start = q
        .start_time
        .as_deref()
        .and_then(parse_dt)
        .unwrap_or(end - Duration::days(30));
    state
        .store
        .reconcile_billing(start, end)
        .await
        .map(Json)
        .map_err(err)
}
