use super::*;

pub async fn set_number_owner(
    State(state): State<AppState>,
    Path(number): Path<String>,
    Json(body): Json<NumberOwnerBody>,
) -> EmptyResult {
    let updated = state
        .store
        .set_number_owner(&number, &body.owner_egress_trunk_id)
        .await
        .map_err(database)?;
    if updated {
        crate::resources::routes::publish_route_reload(&state.nats_client).await;
        Ok(StatusCode::OK)
    } else {
        Err((StatusCode::NOT_FOUND, "号码或落地中继不存在".to_string()))
    }
}

pub async fn list_allocations(
    State(state): State<AppState>,
    Path(number): Path<String>,
) -> Result<Json<Vec<NumberAllocation>>, Error> {
    state
        .store
        .list_number_allocations(Some(&number))
        .await
        .map(Json)
        .map_err(database)
}

pub async fn replace_allocations(
    State(state): State<AppState>,
    Path(number): Path<String>,
    Json(body): Json<Batch<AllocationBody>>,
) -> EmptyResult {
    let mut allocations = Vec::with_capacity(body.items.len());
    if body
        .items
        .iter()
        .filter(|item| item.enabled.unwrap_or(true))
        .count()
        > 1
    {
        return Err(invalid("一个号码默认只能有一个有效授权"));
    }
    for item in body.items {
        validate_source_type(&item.source_type)?;
        if item.source_id.trim().is_empty() {
            return Err(invalid("授权来源 ID 不能为空"));
        }
        ensure_source_exists(&state, &item.source_type, &item.source_id).await?;
        allocations.push(NumberAllocation {
            id: 0,
            number: number.clone(),
            source_type: item.source_type,
            source_id: item.source_id,
            enabled: item.enabled.unwrap_or(true),
        });
    }
    state
        .store
        .replace_number_allocations(&number, &allocations)
        .await
        .map_err(database)?;
    crate::resources::routes::publish_route_reload(&state.nats_client).await;
    Ok(StatusCode::OK)
}
