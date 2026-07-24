//! Versioned management API routes and response contract.

mod response;
mod routes;

use axum::{
    extract::State,
    routing::{get, post},
    Json, Router,
};

use crate::{copilot, system::auth, AppState};

use response::response_contract;
use routes::*;

/// Builds public v1 endpoints. These routes do not require a bearer token.
pub(crate) fn public_routes() -> Router<AppState> {
    Router::new()
        .route("/api/v1/auth/sessions", post(auth::login))
        .route_layer(axum::middleware::from_fn(response_contract))
}

/// Builds authenticated v1 endpoints grouped by business domain.
pub(crate) fn protected_routes(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/api/v1/auth/me", get(auth::current_session))
        .merge(overview_routes())
        .merge(subscriber_routes())
        .merge(interconnect_routes())
        .merge(termination_routes())
        .merge(routing_routes())
        .merge(call_routes())
        .merge(billing_routes())
        .merge(security_routes())
        .merge(infrastructure_routes())
        .merge(call_center_routes())
        .merge(ivr_routes())
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::audit_log,
        ))
        .route_layer(axum::middleware::from_fn_with_state(state, crate::jwt_auth))
        .route_layer(axum::middleware::from_fn(response_contract))
}

#[derive(serde::Deserialize)]
struct CopilotRequest {
    query: String,
    model_id: Option<i64>,
}

async fn handle_copilot_chat(
    State(state): State<crate::AppState>,
    Json(payload): Json<CopilotRequest>,
) -> Result<Json<copilot::CopilotChatResponse>, crate::ApiError> {
    let active_llm = crate::llm_configs::get_llm_config_from_redis(&state, payload.model_id)
        .await
        .map(copilot::LlmConfig::from);
    let engine = copilot::TelecomCopilotEngine::new(&state, active_llm);
    let response = engine.analyze(&payload.query, None).await;
    Ok(Json(response))
}
