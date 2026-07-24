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

async fn validate_caller_reference(
    state: &AppState,
    policy: &SourceOutboundPolicy,
) -> Result<(), Error> {
    let valid: bool = match policy.caller_mode.as_str() {
        "strict_passthrough" => true,
        "fixed_number" => sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM number_inventory n \
             JOIN number_allocations a ON a.number=n.number AND a.enabled \
             WHERE n.number=$1 AND a.source_type=$2 AND a.source_id=$3 \
               AND LOWER(n.status) IN ('available','assigned','active') \
               AND LOWER(COALESCE(n.direction,'both')) IN ('outbound','both','bidirectional') \
               AND COALESCE(NULLIF(n.owner_egress_trunk_id,''),NULLIF(n.gateway_id,'')) IS NOT NULL)",
        )
        .bind(policy.fixed_number.as_deref())
        .bind(&policy.source_type)
        .bind(&policy.source_id)
        .fetch_one(state.store.pool())
        .await
        .map_err(database)?,
        "virtual_pool" => sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM caller_pools WHERE id=$1 AND owner_source_type=$2 AND owner_source_id=$3 AND enabled)",
        )
        .bind(policy.caller_pool_id.as_deref())
        .bind(&policy.source_type)
        .bind(&policy.source_id)
        .fetch_one(state.store.pool())
        .await
        .map_err(database)?,
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(invalid(
            "固定号码或号码池未授权给当前来源，或当前不可用于显号",
        ))
    }
}

async fn validate_egress_reference(
    state: &AppState,
    policy: &SourceOutboundPolicy,
) -> Result<(), Error> {
    let valid: bool = match policy.egress_mode.as_str() {
        "direct" => sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM sip_gateways WHERE id=$1 AND role='egress' AND enabled)",
        )
        .bind(policy.direct_egress_trunk_id.as_deref())
        .fetch_one(state.store.pool())
        .await
        .map_err(database)?,
        "group" => {
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM egress_groups WHERE id=$1 AND enabled)")
                .bind(policy.egress_group_id.as_deref())
                .fetch_one(state.store.pool())
                .await
                .map_err(database)?
        }
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(invalid("落地中继或落地分组不存在、未启用或类型不匹配"))
    }
}

async fn validate_number_egress_coverage(
    state: &AppState,
    policy: &SourceOutboundPolicy,
) -> Result<(), Error> {
    let covered: bool = match (policy.caller_mode.as_str(), policy.egress_mode.as_str()) {
        ("strict_passthrough", _) => true,
        ("fixed_number", "direct") => sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM number_inventory WHERE number=$1 \
             AND COALESCE(NULLIF(owner_egress_trunk_id,''),NULLIF(gateway_id,''))=$2)",
        )
        .bind(policy.fixed_number.as_deref())
        .bind(policy.direct_egress_trunk_id.as_deref())
        .fetch_one(state.store.pool())
        .await
        .map_err(database)?,
        ("fixed_number", "group") => sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM number_inventory n JOIN egress_group_members m \
             ON m.egress_trunk_id=COALESCE(NULLIF(n.owner_egress_trunk_id,''),NULLIF(n.gateway_id,'')) \
             WHERE n.number=$1 AND m.group_id=$2 AND m.enabled)",
        )
        .bind(policy.fixed_number.as_deref())
        .bind(policy.egress_group_id.as_deref())
        .fetch_one(state.store.pool())
        .await
        .map_err(database)?,
        ("virtual_pool", "direct") => sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM caller_pool_members m JOIN number_inventory n ON n.number=m.number \
             WHERE m.pool_id=$1 AND m.enabled) AND NOT EXISTS(SELECT 1 FROM caller_pool_members m \
             JOIN number_inventory n ON n.number=m.number WHERE m.pool_id=$1 AND m.enabled \
             AND COALESCE(NULLIF(n.owner_egress_trunk_id,''),NULLIF(n.gateway_id,''))<>$2)",
        )
        .bind(policy.caller_pool_id.as_deref())
        .bind(policy.direct_egress_trunk_id.as_deref())
        .fetch_one(state.store.pool())
        .await
        .map_err(database)?,
        ("virtual_pool", "group") => sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM caller_pool_members WHERE pool_id=$1 AND enabled) \
             AND NOT EXISTS(SELECT 1 FROM caller_pool_members p JOIN number_inventory n ON n.number=p.number \
             WHERE p.pool_id=$1 AND p.enabled AND NOT EXISTS(SELECT 1 FROM egress_group_members m \
             WHERE m.group_id=$2 AND m.enabled AND m.egress_trunk_id=COALESCE(NULLIF(n.owner_egress_trunk_id,''),NULLIF(n.gateway_id,''))))",
        )
        .bind(policy.caller_pool_id.as_deref())
        .bind(policy.egress_group_id.as_deref())
        .fetch_one(state.store.pool())
        .await
        .map_err(database)?,
        _ => false,
    };
    if covered {
        Ok(())
    } else {
        Err(invalid("主叫号码的唯一落地归属不在当前落地绑定范围内"))
    }
}

async fn validate_policy_references(
    state: &AppState,
    policy: &SourceOutboundPolicy,
) -> Result<(), Error> {
    ensure_source_exists(state, &policy.source_type, &policy.source_id).await?;
    validate_caller_reference(state, policy).await?;
    validate_egress_reference(state, policy).await?;
    validate_number_egress_coverage(state, policy).await
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
    validate_policy_references(&state, &policy).await?;
    state
        .store
        .upsert_source_outbound_policy(&policy)
        .await
        .map_err(database)?;
    crate::resources::routes::publish_route_reload(&state.nats_client).await;
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
        crate::resources::routes::publish_route_reload(&state.nats_client).await;
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
    validate_policy_references(&state, &policy).await?;
    state
        .store
        .upsert_source_outbound_policy(&policy)
        .await
        .map_err(database)?;
    crate::resources::routes::publish_route_reload(&state.nats_client).await;
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
    validate_policy_references(&state, &policy).await?;
    state
        .store
        .upsert_source_outbound_policy(&policy)
        .await
        .map_err(database)?;
    crate::resources::routes::publish_route_reload(&state.nats_client).await;
    Ok(StatusCode::OK)
}
