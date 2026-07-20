use super::*;

pub async fn list_caller_pools(
    State(state): State<AppState>,
) -> Result<Json<Vec<CallerPool>>, Error> {
    state
        .store
        .list_caller_pools()
        .await
        .map(Json)
        .map_err(database)
}

async fn save_caller_pool(
    state: AppState,
    id: String,
    body: CallerPoolBody,
    status: StatusCode,
) -> EmptyResult {
    validate_source_type(&body.owner_source_type)?;
    if id.trim().is_empty()
        || body.owner_source_id.trim().is_empty()
        || body.virtual_alias.trim().is_empty()
    {
        return Err(invalid("号码池 ID、来源和虚拟别名不能为空"));
    }
    ensure_source_exists(&state, &body.owner_source_type, &body.owner_source_id).await?;
    if !matches!(
        body.strategy.as_str(),
        "random" | "round_robin" | "weighted_random" | "stable_hash" | "weighted" | "hash"
    ) {
        return Err(invalid("号码池策略不受支持"));
    }
    let strategy = match body.strategy.as_str() {
        "weighted" => "weighted_random",
        "hash" => "stable_hash",
        value => value,
    }
    .to_string();
    if body.fallback_mode.as_deref().unwrap_or("reject") != "reject" {
        return Err(invalid("当前仅支持号码池选择失败时拒绝呼叫"));
    }
    let fallback_mode = "reject".to_string();
    let now = OffsetDateTime::now_utc();
    state
        .store
        .upsert_caller_pool(&CallerPool {
            id,
            owner_source_type: body.owner_source_type,
            owner_source_id: body.owner_source_id,
            virtual_alias: body.virtual_alias,
            strategy,
            fallback_mode,
            enabled: body.enabled.unwrap_or(true),
            created_at: now,
            updated_at: now,
        })
        .await
        .map_err(database)?;
    crate::routes::publish_route_reload(&state.nats_client).await;
    Ok(status)
}

pub async fn create_caller_pool(
    State(state): State<AppState>,
    Json(body): Json<CallerPoolBody>,
) -> EmptyResult {
    let id = body
        .id
        .clone()
        .ok_or_else(|| invalid("号码池 ID 不能为空"))?;
    save_caller_pool(state, id, body, StatusCode::CREATED).await
}
pub async fn update_caller_pool(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<CallerPoolBody>,
) -> EmptyResult {
    save_caller_pool(state, id, body, StatusCode::OK).await
}
pub async fn delete_caller_pool(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> EmptyResult {
    if state
        .store
        .delete_caller_pool(&id)
        .await
        .map_err(database)?
    {
        crate::routes::publish_route_reload(&state.nats_client).await;
        Ok(StatusCode::OK)
    } else {
        Err((StatusCode::NOT_FOUND, "号码池不存在".to_string()))
    }
}
pub async fn list_caller_pool_members(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Vec<CallerPoolMember>>, Error> {
    state
        .store
        .list_caller_pool_members(&id)
        .await
        .map(Json)
        .map_err(database)
}

pub async fn replace_caller_pool_members(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<Batch<CallerPoolMemberBody>>,
) -> EmptyResult {
    let pool = state
        .store
        .list_caller_pools()
        .await
        .map_err(database)?
        .into_iter()
        .find(|pool| pool.id == id)
        .ok_or((StatusCode::NOT_FOUND, "号码池不存在".to_string()))?;
    let mut members = Vec::with_capacity(body.items.len());
    for item in body.items {
        let priority = item.priority.unwrap_or(100);
        let weight = item.weight.unwrap_or(100);
        let max = item.max_concurrent.unwrap_or(0);
        if item.number.trim().is_empty()
            || !(0..=65535).contains(&priority)
            || !(1..=10000).contains(&weight)
            || max < 0
        {
            return Err(invalid("号码池成员参数无效"));
        }
        let authorized: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM number_inventory n \
             JOIN number_allocations a ON a.number=n.number AND a.enabled \
             WHERE n.number=$1 AND a.source_type=$2 AND a.source_id=$3 \
               AND LOWER(n.status) IN ('available','assigned','active') \
               AND LOWER(COALESCE(n.direction,'both')) IN ('outbound','both','bidirectional') \
               AND COALESCE(NULLIF(n.owner_egress_trunk_id,''),NULLIF(n.gateway_id,'')) IS NOT NULL)",
        )
        .bind(&item.number)
        .bind(&pool.owner_source_type)
        .bind(&pool.owner_source_id)
        .fetch_one(state.store.pool())
        .await
        .map_err(database)?;
        if !authorized {
            return Err(invalid(format!(
                "号码 {} 未授权给当前来源、不可显号或没有落地归属",
                item.number
            )));
        }
        members.push(CallerPoolMember {
            id: 0,
            pool_id: id.clone(),
            number: item.number,
            priority,
            weight,
            max_concurrent: max,
            enabled: item.enabled.unwrap_or(true),
        });
    }
    state
        .store
        .replace_caller_pool_members(&id, &members)
        .await
        .map_err(database)?;
    crate::routes::publish_route_reload(&state.nats_client).await;
    Ok(StatusCode::OK)
}
