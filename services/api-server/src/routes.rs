use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use cdr_core::SipRoute;
use serde::Deserialize;
use serde_json::Value as JsonValue;

use crate::{normalize_page, ApiError, AppState, PageQuery, PaginatedResponse};

#[derive(Debug, Deserialize)]
pub struct CreateRouteRequest {
    pub id: String,
    pub prefix: String,
    pub priority: i32,
    pub gateway_id: String,
    pub cost: f64,
    pub weight: Option<i32>,
    pub time_start: Option<String>,
    pub time_end: Option<String>,
    /// 可视化拓扑编排数据 (节点 + 边 + 视口), 由前端 route-rule-binding 画布保存
    #[serde(default)]
    pub topology: Option<JsonValue>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateRouteRequest {
    pub prefix: String,
    pub priority: i32,
    pub gateway_id: String,
    pub cost: f64,
    pub weight: Option<i32>,
    pub time_start: Option<String>,
    pub time_end: Option<String>,
    /// 可视化拓扑编排数据 (节点 + 边 + 视口), 由前端 route-rule-binding 画布保存
    #[serde(default)]
    pub topology: Option<JsonValue>,
}

#[allow(clippy::too_many_arguments)]
fn validate_route(
    id: &str,
    prefix: &str,
    priority: i32,
    gateway_id: &str,
    cost: f64,
    weight: Option<i32>,
    time_start: Option<&str>,
    time_end: Option<&str>,
) -> Result<i32, ApiError> {
    if id.trim().is_empty() {
        return Err(ApiError::internal("参数无效: 规则 ID 不能为空"));
    }
    if prefix
        .chars()
        .any(|character| !character.is_ascii_digit() && !matches!(character, '+' | '*' | '#'))
    {
        return Err(ApiError::internal(
            "参数无效: 匹配前缀只能包含数字、+、* 或 #",
        ));
    }
    if !(0..=i32::from(u16::MAX)).contains(&priority) {
        return Err(ApiError::internal("参数无效: 优先级必须在 0 到 65535 之间"));
    }
    if gateway_id.trim().is_empty() {
        return Err(ApiError::internal("参数无效: 目标中继不能为空"));
    }
    if !cost.is_finite() || cost < 0.0 {
        return Err(ApiError::internal(
            "参数无效: 成本必须是大于等于 0 的有限数值",
        ));
    }

    let weight = weight.unwrap_or(100);
    if !(1..=10_000).contains(&weight) {
        return Err(ApiError::internal("参数无效: 权重必须在 1 到 10000 之间"));
    }
    if time_start.is_some() != time_end.is_some() {
        return Err(ApiError::internal(
            "参数无效: 生效开始和结束时间必须同时填写",
        ));
    }
    if let (Some(start), Some(end)) = (time_start, time_end) {
        if !is_hhmm(start) || !is_hhmm(end) {
            return Err(ApiError::internal("参数无效: 生效时间必须使用 HH:MM 格式"));
        }
    }
    Ok(weight)
}

fn is_hhmm(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 5
        || bytes[2] != b':'
        || !bytes[..2].iter().all(u8::is_ascii_digit)
        || !bytes[3..].iter().all(u8::is_ascii_digit)
    {
        return false;
    }
    let (Ok(hour), Ok(minute)) = (value[..2].parse::<u8>(), value[3..].parse::<u8>()) else {
        return false;
    };
    hour < 24 && minute < 60
}

pub async fn publish_route_reload(nats: &Option<async_nats::Client>) -> bool {
    if let Some(client) = nats {
        if let Err(e) = client
            .publish("vos_rs.routing.reload", axum::body::Bytes::from("reload"))
            .await
        {
            tracing::warn!(error = %e, "NATS 路由重载广播发布失败，数据库变更将在周期刷新后生效");
            return false;
        }
        return true;
    }
    tracing::warn!("NATS 未连接，路由重载通知处于 pending，数据库变更将在周期刷新后生效");
    false
}

pub async fn list_routes(
    State(state): State<AppState>,
    Query(query): Query<PageQuery>,
) -> Result<Json<PaginatedResponse<SipRoute>>, ApiError> {
    let (page, page_size, offset) = normalize_page(&query);
    let (items, total) = tokio::try_join!(
        state.store.list_routes_page(page_size, offset),
        state.store.count_routes(),
    )
    .map_err(|e| ApiError {
        error: e.to_string(),
    })?;
    Ok(Json(PaginatedResponse {
        items,
        total,
        page,
        page_size,
    }))
}

pub async fn create_route(
    State(state): State<AppState>,
    Json(req): Json<CreateRouteRequest>,
) -> Result<StatusCode, ApiError> {
    let weight = validate_route(
        &req.id,
        &req.prefix,
        req.priority,
        &req.gateway_id,
        req.cost,
        req.weight,
        req.time_start.as_deref(),
        req.time_end.as_deref(),
    )?;
    state
        .store
        .insert_route_with_cost(
            &req.id,
            &req.prefix,
            req.priority,
            &req.gateway_id,
            req.cost,
            weight,
            req.time_start.as_deref(),
            req.time_end.as_deref(),
            req.topology.as_ref(),
        )
        .await
        .map_err(|e| ApiError {
            error: e.to_string(),
        })?;
    publish_route_reload(&state.nats_client).await;
    Ok(StatusCode::CREATED)
}

pub async fn update_route(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateRouteRequest>,
) -> Result<StatusCode, ApiError> {
    let weight = validate_route(
        &id,
        &req.prefix,
        req.priority,
        &req.gateway_id,
        req.cost,
        req.weight,
        req.time_start.as_deref(),
        req.time_end.as_deref(),
    )?;
    let updated = state
        .store
        .update_route_with_cost(
            &id,
            &req.prefix,
            req.priority,
            &req.gateway_id,
            req.cost,
            weight,
            req.time_start.as_deref(),
            req.time_end.as_deref(),
            req.topology.as_ref(),
        )
        .await
        .map_err(|e| ApiError {
            error: e.to_string(),
        })?;
    if !updated {
        return Err(ApiError::internal("路由规则不存在"));
    }
    publish_route_reload(&state.nats_client).await;
    Ok(StatusCode::OK)
}

pub async fn delete_route(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let deleted = state.store.delete_route(&id).await.map_err(|e| ApiError {
        error: e.to_string(),
    })?;
    if deleted {
        publish_route_reload(&state.nats_client).await;
        Ok(StatusCode::OK)
    } else {
        Ok(StatusCode::NOT_FOUND)
    }
}

#[cfg(test)]
mod tests {
    use super::{is_hhmm, validate_route};

    #[test]
    fn validates_route_ranges_and_time_pairs() {
        assert!(validate_route("r1", "86", 100, "gw1", 0.1, Some(100), None, None).is_ok());
        assert!(validate_route("r1", "86", -1, "gw1", 0.1, None, None, None).is_err());
        assert!(validate_route("r1", "86", 100, "gw1", -0.1, None, None, None).is_err());
        assert!(validate_route("r1", "86", 100, "gw1", 0.1, None, Some("09:00"), None).is_err());
        assert!(is_hhmm("23:59"));
        assert!(!is_hhmm("24:00"));
    }
}
