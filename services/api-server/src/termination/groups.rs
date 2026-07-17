use super::*;

pub async fn list_egress_groups(
    State(state): State<AppState>,
) -> Result<Json<Vec<EgressGroup>>, Error> {
    state
        .store
        .list_egress_groups()
        .await
        .map(Json)
        .map_err(database)
}
async fn save_egress_group(
    state: AppState,
    id: String,
    body: EgressGroupBody,
    status: StatusCode,
) -> EmptyResult {
    if id.trim().is_empty() || body.name.trim().is_empty() {
        return Err(invalid("落地组 ID 和名称不能为空"));
    }
    let now = OffsetDateTime::now_utc();
    state
        .store
        .upsert_egress_group(&EgressGroup {
            id,
            name: body.name,
            enabled: body.enabled.unwrap_or(true),
            created_at: now,
            updated_at: now,
        })
        .await
        .map_err(database)?;
    crate::routes::publish_route_reload(&state.nats_client).await;
    Ok(status)
}

pub async fn create_egress_group(
    State(state): State<AppState>,
    Json(body): Json<EgressGroupBody>,
) -> EmptyResult {
    let id = body
        .id
        .clone()
        .ok_or_else(|| invalid("落地组 ID 不能为空"))?;
    save_egress_group(state, id, body, StatusCode::CREATED).await
}
pub async fn update_egress_group(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<EgressGroupBody>,
) -> EmptyResult {
    save_egress_group(state, id, body, StatusCode::OK).await
}
pub async fn delete_egress_group(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> EmptyResult {
    if state
        .store
        .delete_egress_group(&id)
        .await
        .map_err(database)?
    {
        crate::routes::publish_route_reload(&state.nats_client).await;
        Ok(StatusCode::OK)
    } else {
        Err((StatusCode::NOT_FOUND, "落地组不存在".to_string()))
    }
}

pub async fn list_egress_group_members(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Vec<EgressGroupMember>>, Error> {
    state
        .store
        .list_egress_group_members(&id)
        .await
        .map(Json)
        .map_err(database)
}

fn valid_hhmm(value: &str) -> bool {
    let mut p = value.split(':');
    matches!(
        (
            p.next().and_then(|v| v.parse::<u8>().ok()),
            p.next().and_then(|v| v.parse::<u8>().ok()),
            p.next()
        ),
        (Some(0..=23), Some(0..=59), None)
    )
}
pub async fn replace_egress_group_members(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<Batch<EgressGroupMemberBody>>,
) -> EmptyResult {
    let mut members = Vec::with_capacity(body.items.len());
    for item in body.items {
        ensure_trunk_role(&state, &item.egress_trunk_id, "egress").await?;
        let priority = item.priority.unwrap_or(100);
        let weight = item.weight.unwrap_or(100);
        if item.egress_trunk_id.trim().is_empty()
            || !(0..=65535).contains(&priority)
            || !(1..=10000).contains(&weight)
        {
            return Err(invalid("落地组成员参数无效"));
        }
        if item.time_start.is_some() != item.time_end.is_some()
            || item.time_start.as_deref().is_some_and(|v| !valid_hhmm(v))
            || item.time_end.as_deref().is_some_and(|v| !valid_hhmm(v))
        {
            return Err(invalid("时间窗口必须成对使用 HH:MM"));
        }
        members.push(EgressGroupMember {
            id: 0,
            group_id: id.clone(),
            egress_trunk_id: item.egress_trunk_id,
            destination_prefix: item.destination_prefix.unwrap_or_default(),
            priority,
            weight,
            time_start: item.time_start,
            time_end: item.time_end,
            enabled: item.enabled.unwrap_or(true),
        });
    }
    state
        .store
        .replace_egress_group_members(&id, &members)
        .await
        .map_err(database)?;
    crate::routes::publish_route_reload(&state.nats_client).await;
    Ok(StatusCode::OK)
}


