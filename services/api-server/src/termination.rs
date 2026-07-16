use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use cdr_core::{
    CallerPool, CallerPoolMember, DidDestination, EgressEndpoint, EgressGroup, EgressGroupMember,
    NumberAllocation, SourceOutboundPolicy, TrunkIpRule,
};
use serde::Deserialize;
use std::net::IpAddr;
use time::OffsetDateTime;

use crate::AppState;

type Error = (StatusCode, String);
type EmptyResult = Result<StatusCode, Error>;

#[derive(Debug, Deserialize)]
pub struct Batch<T> {
    pub items: Vec<T>,
}

#[derive(Debug, Deserialize)]
pub struct IpRuleBody {
    pub cidr: String,
    pub source_port: Option<i32>,
    pub transport: Option<String>,
    pub description: Option<String>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct EndpointBody {
    pub host: String,
    pub port: Option<i32>,
    pub transport: Option<String>,
    pub priority: Option<i32>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct NumberOwnerBody {
    pub owner_egress_trunk_id: String,
}

#[derive(Debug, Deserialize)]
pub struct AllocationBody {
    pub source_type: String,
    pub source_id: String,
    pub enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct CallerPoolBody {
    pub id: Option<String>,
    pub owner_source_type: String,
    pub owner_source_id: String,
    pub virtual_alias: String,
    pub strategy: String,
    pub fallback_mode: Option<String>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct CallerPoolMemberBody {
    pub number: String,
    pub priority: Option<i32>,
    pub weight: Option<i32>,
    pub max_concurrent: Option<i32>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct EgressGroupBody {
    pub id: Option<String>,
    pub name: String,
    pub enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct EgressGroupMemberBody {
    pub egress_trunk_id: String,
    pub destination_prefix: Option<String>,
    pub priority: Option<i32>,
    pub weight: Option<i32>,
    pub time_start: Option<String>,
    pub time_end: Option<String>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct SourcePolicyBody {
    pub caller_mode: String,
    pub fixed_number: Option<String>,
    pub caller_pool_id: Option<String>,
    pub egress_mode: String,
    pub direct_egress_trunk_id: Option<String>,
    pub egress_group_id: Option<String>,
    pub fallback_mode: Option<String>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct DidDestinationBody {
    pub number: Option<String>,
    pub tenant_id: Option<String>,
    pub target_type: String,
    pub target_id: String,
    pub enabled: Option<bool>,
}

fn invalid(message: impl Into<String>) -> Error {
    (StatusCode::BAD_REQUEST, message.into())
}

fn database(error: sqlx::Error) -> Error {
    if error.to_string().contains("重叠") {
        (StatusCode::CONFLICT, error.to_string())
    } else if error.as_database_error().is_some() {
        (StatusCode::UNPROCESSABLE_ENTITY, error.to_string())
    } else {
        (StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
    }
}

fn validate_source_type(value: &str) -> Result<(), Error> {
    if matches!(value, "trunk" | "extension" | "extension_group") {
        Ok(())
    } else {
        Err(invalid(
            "来源类型只能是 trunk、extension 或 extension_group",
        ))
    }
}

fn validate_cidr(value: &str) -> Result<(), Error> {
    let (address, prefix) = value
        .split_once('/')
        .ok_or_else(|| invalid("IP 白名单必须使用 CIDR 格式"))?;
    let address: IpAddr = address.parse().map_err(|_| invalid("CIDR 地址无效"))?;
    let prefix: u8 = prefix.parse().map_err(|_| invalid("CIDR 前缀无效"))?;
    let max = if address.is_ipv4() { 32 } else { 128 };
    if prefix > max {
        return Err(invalid("CIDR 前缀超出地址范围"));
    }
    Ok(())
}

fn canonical_caller_mode(value: &str) -> Option<&'static str> {
    match value {
        "strict_passthrough" | "passthrough" => Some("strict_passthrough"),
        "fixed_number" | "fixed" => Some("fixed_number"),
        "virtual_pool" | "pool" => Some("virtual_pool"),
        _ => None,
    }
}

fn validate_policy(body: &SourcePolicyBody) -> Result<&'static str, Error> {
    let mode = canonical_caller_mode(&body.caller_mode)
        .ok_or_else(|| invalid("主叫策略只能是严格透传、固定号码或虚拟号码池"))?;
    match mode {
        "strict_passthrough" if body.fixed_number.is_none() && body.caller_pool_id.is_none() => {}
        "fixed_number"
            if body
                .fixed_number
                .as_deref()
                .is_some_and(|v| !v.trim().is_empty())
                && body.caller_pool_id.is_none() => {}
        "virtual_pool"
            if body
                .caller_pool_id
                .as_deref()
                .is_some_and(|v| !v.trim().is_empty())
                && body.fixed_number.is_none() => {}
        _ => return Err(invalid("主叫策略与固定号码/号码池字段不匹配或存在冲突")),
    }
    match body.egress_mode.as_str() {
        "direct" if body.direct_egress_trunk_id.is_some() && body.egress_group_id.is_none() => {}
        "group" if body.egress_group_id.is_some() && body.direct_egress_trunk_id.is_none() => {}
        _ => return Err(invalid("落地模式与直绑中继/落地组字段不匹配或存在冲突")),
    }
    if !matches!(
        body.fallback_mode.as_deref().unwrap_or("reject"),
        "reject" | "fallback_number" | "fallback_pool" | "fixed" | "pool"
    ) {
        return Err(invalid("失败策略不受支持"));
    }
    Ok(mode)
}

pub async fn list_ip_rules(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Vec<TrunkIpRule>>, Error> {
    state
        .store
        .list_trunk_ip_rules(&id)
        .await
        .map(Json)
        .map_err(database)
}

pub async fn replace_ip_rules(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<Batch<IpRuleBody>>,
) -> EmptyResult {
    let mut rules = Vec::with_capacity(body.items.len());
    for item in body.items {
        validate_cidr(&item.cidr)?;
        if item
            .source_port
            .is_some_and(|port| !(1..=65535).contains(&port))
        {
            return Err(invalid("来源端口必须在 1 到 65535 之间"));
        }
        let transport = item.transport.unwrap_or_else(|| "udp".to_string());
        if transport != "udp" {
            return Err(invalid("第一阶段 IP 规则仅支持 udp"));
        }
        rules.push(TrunkIpRule {
            id: 0,
            trunk_id: id.clone(),
            cidr: item.cidr,
            source_port: item.source_port,
            transport,
            description: item.description.unwrap_or_default(),
            enabled: item.enabled.unwrap_or(true),
        });
    }
    state
        .store
        .replace_trunk_ip_rules(&id, &rules)
        .await
        .map_err(database)?;
    Ok(StatusCode::OK)
}

pub async fn list_endpoints(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Vec<EgressEndpoint>>, Error> {
    state
        .store
        .list_egress_endpoints(&id)
        .await
        .map(Json)
        .map_err(database)
}

pub async fn replace_endpoints(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<Batch<EndpointBody>>,
) -> EmptyResult {
    let mut endpoints = Vec::with_capacity(body.items.len());
    for item in body.items {
        if item.host.trim().is_empty() || item.host.chars().any(char::is_whitespace) {
            return Err(invalid("端点主机不能为空或包含空格"));
        }
        let port = item.port.unwrap_or(5060);
        let priority = item.priority.unwrap_or(100);
        let transport = item.transport.unwrap_or_else(|| "udp".to_string());
        if !(1..=65535).contains(&port) || !(0..=65535).contains(&priority) || transport != "udp" {
            return Err(invalid("端点端口、优先级或传输协议无效"));
        }
        endpoints.push(EgressEndpoint {
            id: 0,
            trunk_id: id.clone(),
            host: item.host,
            port,
            transport,
            priority,
            enabled: item.enabled.unwrap_or(true),
        });
    }
    state
        .store
        .replace_egress_endpoints(&id, &endpoints)
        .await
        .map_err(database)?;
    Ok(StatusCode::OK)
}

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
    for item in body.items {
        validate_source_type(&item.source_type)?;
        if item.source_id.trim().is_empty() {
            return Err(invalid("授权来源 ID 不能为空"));
        }
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
    Ok(StatusCode::OK)
}

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
    let fallback_mode = match body.fallback_mode.as_deref().unwrap_or("reject") {
        "fixed" => "fallback_number",
        "pool" => "fallback_pool",
        value => value,
    }
    .to_string();
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
    Ok(StatusCode::OK)
}

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
    Ok(StatusCode::OK)
}

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
    let policy = build_policy("trunk".to_string(), id, body)?;
    state
        .store
        .upsert_source_outbound_policy(&policy)
        .await
        .map_err(database)?;
    Ok(StatusCode::OK)
}

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
        || body.target_id.trim().is_empty()
        || !matches!(
            body.target_type.as_str(),
            "extension" | "extension_group" | "ivr" | "reject"
        )
    {
        return Err(invalid("DID 号码或目标参数无效"));
    }
    state
        .store
        .upsert_did_destination(&DidDestination {
            number,
            tenant_id: body.tenant_id,
            target_type: body.target_type,
            target_id: body.target_id,
            enabled: body.enabled.unwrap_or(true),
            updated_at: OffsetDateTime::now_utc(),
        })
        .await
        .map_err(database)?;
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
        Ok(StatusCode::OK)
    } else {
        Err((StatusCode::NOT_FOUND, "DID 目标不存在".to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(caller_mode: &str, egress_mode: &str) -> SourcePolicyBody {
        SourcePolicyBody {
            caller_mode: caller_mode.to_string(),
            fixed_number: None,
            caller_pool_id: None,
            egress_mode: egress_mode.to_string(),
            direct_egress_trunk_id: None,
            egress_group_id: None,
            fallback_mode: None,
            enabled: None,
        }
    }

    #[test]
    fn validates_ipv4_and_ipv6_cidr() {
        assert!(validate_cidr("192.0.2.0/24").is_ok());
        assert!(validate_cidr("2001:db8::/64").is_ok());
        assert!(validate_cidr("192.0.2.1").is_err());
        assert!(validate_cidr("192.0.2.0/33").is_err());
    }

    #[test]
    fn policy_requires_exclusive_caller_and_egress_fields() {
        let mut body = policy("fixed_number", "direct");
        body.fixed_number = Some("10086".to_string());
        body.direct_egress_trunk_id = Some("carrier-a".to_string());
        assert_eq!(validate_policy(&body).ok(), Some("fixed_number"));
        body.caller_pool_id = Some("pool-a".to_string());
        assert!(validate_policy(&body).is_err());
        body.caller_pool_id = None;
        body.egress_group_id = Some("group-a".to_string());
        assert!(validate_policy(&body).is_err());
    }

    #[test]
    fn legacy_policy_aliases_are_canonicalized() {
        let mut body = policy("passthrough", "group");
        body.egress_group_id = Some("g".to_string());
        assert_eq!(validate_policy(&body).ok(), Some("strict_passthrough"));
    }
}
