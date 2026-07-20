use super::*;

pub async fn list_dids(State(state): State<AppState>) -> Result<Json<Vec<DidDestination>>, Error> {
    state
        .store
        .list_did_destinations()
        .await
        .map(Json)
        .map_err(database)
}
async fn save_did(
    state: AppState,
    number: String,
    body: DidDestinationBody,
    status: StatusCode,
) -> EmptyResult {
    if number.trim().is_empty()
        || !matches!(body.target_type.as_str(), "extension" | "reject")
        || (body.target_type == "extension" && body.target_id.trim().is_empty())
    {
        return Err(invalid("DID 号码或目标参数无效"));
    }
    let target_id = if body.target_type == "reject" {
        "reject".to_string()
    } else {
        body.target_id
    };
    state
        .store
        .upsert_did_destination(&DidDestination {
            number,
            tenant_id: body.tenant_id,
            target_type: body.target_type,
            target_id,
            enabled: body.enabled.unwrap_or(true),
            updated_at: OffsetDateTime::now_utc(),
        })
        .await
        .map_err(database)?;
    crate::routes::publish_route_reload(&state.nats_client).await;
    Ok(status)
}
pub async fn create_did(
    State(state): State<AppState>,
    Json(body): Json<DidDestinationBody>,
) -> EmptyResult {
    let number = body
        .number
        .clone()
        .ok_or_else(|| invalid("DID 号码不能为空"))?;
    save_did(state, number, body, StatusCode::CREATED).await
}
pub async fn update_did(
    State(state): State<AppState>,
    Path(number): Path<String>,
    Json(body): Json<DidDestinationBody>,
) -> EmptyResult {
    save_did(state, number, body, StatusCode::OK).await
}
pub async fn delete_did(State(state): State<AppState>, Path(number): Path<String>) -> EmptyResult {
    if state
        .store
        .delete_did_destination(&number)
        .await
        .map_err(database)?
    {
        crate::routes::publish_route_reload(&state.nats_client).await;
        Ok(StatusCode::OK)
    } else {
        Err((StatusCode::NOT_FOUND, "DID 目标不存在".to_string()))
    }
}

pub async fn get_number_did(
    State(state): State<AppState>,
    Path(number): Path<String>,
) -> Result<Json<DidDestination>, Error> {
    state
        .store
        .list_did_destinations()
        .await
        .map_err(database)?
        .into_iter()
        .find(|destination| destination.number == number)
        .map(Json)
        .ok_or((StatusCode::NOT_FOUND, "DID 目标不存在".to_string()))
}

pub async fn put_number_did(
    State(state): State<AppState>,
    Path(number): Path<String>,
    Json(body): Json<DidDestinationBody>,
) -> EmptyResult {
    save_did(state, number, body, StatusCode::OK).await
}
