use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;

use crate::{normalize_page, AppState, PageQuery, PaginatedResponse};

#[derive(Debug, Deserialize)]
pub struct CreateNumberBody {
    pub number: String,
    pub username: Option<String>,
    pub gateway_id: Option<String>,
    pub owner_egress_trunk_id: Option<String>,
    pub direction: Option<String>,
    pub max_concurrent: Option<i32>,
    pub status: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateNumberBody {
    pub username: Option<String>,
    pub gateway_id: Option<String>,
    pub owner_egress_trunk_id: Option<String>,
    pub direction: Option<String>,
    pub max_concurrent: Option<i32>,
    pub status: Option<String>,
}

type E = (StatusCode, String);
fn err(e: impl std::fmt::Display) -> E {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

pub async fn list_numbers(
    State(state): State<AppState>,
    Query(query): Query<PageQuery>,
) -> Result<axum::response::Response, E> {
    let (page, page_size, offset) = normalize_page(&query);
    let (items, total) = tokio::try_join!(
        state.store.list_numbers_page(page_size, offset),
        state.store.count_numbers(),
    )
    .map_err(err)?;

    if query.export.unwrap_or(false) {
        let headers = vec!["号码", "关联分机", "落地中继", "呼叫方向", "最大并发", "当前并发", "状态", "创建时间"];
        let mut rows = Vec::new();
        for item in items {
            rows.push(vec![
                item.number.clone(),
                item.username.clone().unwrap_or_default(),
                item.owner_egress_trunk_id.clone().unwrap_or_default(),
                item.direction.clone().unwrap_or_else(|| "both".to_string()),
                item.max_concurrent.map(|c| c.to_string()).unwrap_or_default(),
                item.current_concurrent.map(|c| c.to_string()).unwrap_or_default(),
                item.status.clone(),
                item.created_at.map(|t| t.to_string()).unwrap_or_default(),
            ]);
        }
        return Ok(crate::utils::to_csv_response("numbers.csv", &headers, &rows));
    }

    use axum::response::IntoResponse;
    Ok(Json(PaginatedResponse {
        items,
        total,
        page,
        page_size,
    }).into_response())
}

pub async fn create_number(
    State(state): State<AppState>,
    Json(b): Json<CreateNumberBody>,
) -> Result<StatusCode, E> {
    let owner = b
        .owner_egress_trunk_id
        .as_deref()
        .or(b.gateway_id.as_deref());
    if owner.is_none() {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            "号码必须归属于一个落地中继".to_string(),
        ));
    }
    validate_owner(&state, owner).await?;
    state
        .store
        .upsert_number(
            &b.number,
            b.username.as_deref(),
            b.gateway_id.as_deref().or(owner),
            owner,
            b.direction.as_deref(),
            b.max_concurrent,
            &b.status,
        )
        .await
        .map_err(err)?;
    crate::routes::publish_route_reload(&state.nats_client).await;
    Ok(StatusCode::CREATED)
}

pub async fn update_number(
    State(state): State<AppState>,
    Path(number): Path<String>,
    Json(b): Json<UpdateNumberBody>,
) -> Result<StatusCode, E> {
    let existing = state
        .store
        .list_numbers()
        .await
        .map_err(err)?
        .into_iter()
        .find(|item| item.number == number)
        .ok_or_else(|| (StatusCode::NOT_FOUND, "号码不存在".to_string()))?;
    let owner = b
        .owner_egress_trunk_id
        .as_deref()
        .or(b.gateway_id.as_deref())
        .or(existing.owner_egress_trunk_id.as_deref());
    validate_owner(&state, owner).await?;
    state
        .store
        .upsert_number(
            &number,
            b.username.as_deref().or(existing.username.as_deref()),
            b.gateway_id
                .as_deref()
                .or(owner)
                .or(existing.gateway_id.as_deref()),
            owner,
            b.direction.as_deref().or(existing.direction.as_deref()),
            b.max_concurrent.or(existing.max_concurrent),
            b.status.as_deref().unwrap_or(&existing.status),
        )
        .await
        .map_err(err)?;
    crate::routes::publish_route_reload(&state.nats_client).await;
    Ok(StatusCode::OK)
}

async fn validate_owner(state: &AppState, owner: Option<&str>) -> Result<(), E> {
    let Some(owner) = owner else {
        return Ok(());
    };
    let valid: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM sip_gateways WHERE id=$1 AND role='egress')",
    )
    .bind(owner)
    .fetch_one(state.store.pool())
    .await
    .map_err(err)?;
    if valid {
        Ok(())
    } else {
        Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            "号码 owner 必须是已存在的落地中继".to_string(),
        ))
    }
}

pub async fn delete_number(
    State(state): State<AppState>,
    Path(number): Path<String>,
) -> Result<StatusCode, E> {
    let deleted = state.store.delete_number(&number).await.map_err(err)?;
    if deleted {
        crate::routes::publish_route_reload(&state.nats_client).await;
    }
    Ok(if deleted {
        StatusCode::OK
    } else {
        StatusCode::NOT_FOUND
    })
}
