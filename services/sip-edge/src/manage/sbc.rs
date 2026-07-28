//! SBC 安全规则动态更新端点。

use axum::{extract::State, http::StatusCode, Json};
use serde::Deserialize;
use std::sync::Arc;

use crate::EdgeState;

#[derive(Deserialize)]
pub(super) struct UpdateSbcRulesRequest {
    allow_rules: Vec<String>,
    block_rules: Vec<String>,
}

pub(super) async fn update_sbc_rules(
    State(state): State<Arc<EdgeState>>,
    Json(payload): Json<UpdateSbcRulesRequest>,
) -> (StatusCode, Json<serde_json::Value>) {
    let allow_refs: Vec<&str> = payload.allow_rules.iter().map(|s| s.as_str()).collect();
    let block_refs: Vec<&str> = payload.block_rules.iter().map(|s| s.as_str()).collect();
    state.sbc_engine.update_rules(&allow_refs, &block_refs);

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "success",
            "message": "SBC rules dynamically updated successfully"
        })),
    )
}
