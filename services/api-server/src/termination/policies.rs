use super::*;

fn build_policy(
    source_type: String,
    source_id: String,
    body: SourcePolicyBody,
) -> Result<SourceOutboundPolicy, Error> {
    validate_source_type(&source_type)?;
    if source_id.trim().is_empty() {
        return Err(invalid("来源 ID 不能为空"));
    }
    let mode = validate_policy(&body)?;
    Ok(SourceOutboundPolicy {
        source_type,
        source_id,
        caller_mode: mode.to_string(),
        fixed_number: body.fixed_number,
        caller_pool_id: body.caller_pool_id,
        egress_mode: body.egress_mode,
        direct_egress_trunk_id: body.direct_egress_trunk_id,
        egress_group_id: body.egress_group_id,
        fallback_mode: match body.fallback_mode.as_deref().unwrap_or("reject") {
            "fixed" => "fallback_number",
            "pool" => "fallback_pool",
            value => value,
        }
        .to_string(),
        enabled: body.enabled.unwrap_or(true),
        updated_at: OffsetDateTime::now_utc(),
    })
}
pub async fn list_policies(
    State(state): State<AppState>,
) -> Result<Json<Vec<SourceOutboundPolicy>>, Error> {
    state
        .store
        .list_source_outbound_policies()
        .await
        .map(Json)
        .map_err(database)
}
pub async fn get_policy(
    State(state): State<AppState>,
    Path((source_type, source_id)): Path<(String, String)>,
) -> Result<Json<SourceOutboundPolicy>, Error> {
    state
        .store
        .list_source_outbound_policies()
        .await
        .map_err(database)?
        .into_iter()
        .find(|p| p.source_type == source_type && p.source_id == source_id)
        .map(Json)
        .ok_or((StatusCode::NOT_FOUND, "来源策略不存在".to_string()))
}
pub async fn put_policy(
    State(state): State<AppState>,
    Path((source_type, source_id)): Path<(String, String)>,
    Json(body): Json<SourcePolicyBody>,
) -> EmptyResult {
    let policy = build_policy(source_type, source_id, body)?;
    state
        .store
        .upsert_source_outbound_policy(&policy)
        .await
        .map_err(database)?;
    crate::routes::publish_route_reload(&state.nats_client).await;
    Ok(StatusCode::OK)

}
pub async fn delete_policy(
    State(state): State<AppState>,
    Path((source_type, source_id)): Path<(String, String)>,
) -> EmptyResult {
    if state
        .store
        .delete_source_outbound_policy(&source_type, &source_id)
        .await
        .map_err(database)?
    {
        crate::routes::publish_route_reload(&state.nats_client).await;
        Ok(StatusCode::OK)
    } else {
        Err((StatusCode::NOT_FOUND, "来源策略不存在".to_string()))
    }

}
pub async fn get_trunk_policy(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<SourceOutboundPolicy>, Error> {
    get_policy(State(state), Path(("trunk".to_string(), id))).await
}
pub async fn put_trunk_policy(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<SourcePolicyBody>,
) -> EmptyResult {
    ensure_trunk_role(&state, &id, "access").await?;
    let policy = build_policy("trunk".to_string(), id, body)?;
    state
        .store
        .upsert_source_outbound_policy(&policy)
        .await
        .map_err(database)?;
    crate::routes::publish_route_reload(&state.nats_client).await;
    Ok(StatusCode::OK)

}

pub async fn get_extension_policy(
    State(state): State<AppState>,
    Path(username): Path<String>,
) -> Result<Json<SourceOutboundPolicy>, Error> {
    get_policy(State(state), Path(("extension".to_string(), username))).await
}

pub async fn put_extension_policy(
    State(state): State<AppState>,
    Path(username): Path<String>,
    Json(body): Json<SourcePolicyBody>,
) -> EmptyResult {
    let policy = build_policy("extension".to_string(), username, body)?;
    state
        .store
        .upsert_source_outbound_policy(&policy)
        .await
        .map_err(database)?;
    crate::routes::publish_route_reload(&state.nats_client).await;
    Ok(StatusCode::OK)

}

