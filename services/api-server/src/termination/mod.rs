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
    pub description: Option<String>,
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

async fn ensure_trunk_role(state: &AppState, id: &str, role: &str) -> Result<(), Error> {
    let valid: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sip_gateways WHERE id=$1 AND role=$2)")
            .bind(id)
            .bind(role)
            .fetch_one(state.store.pool())
            .await
            .map_err(database)?;
    if valid {
        Ok(())
    } else {
        Err(invalid(format!("中继 {id} 不存在或不是 {role} 类型")))
    }
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

async fn ensure_source_exists(
    state: &AppState,
    source_type: &str,
    source_id: &str,
) -> Result<(), Error> {
    let exists: bool = match source_type {
        "trunk" => sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM sip_gateways WHERE id=$1 AND role='access' AND enabled)",
        )
        .bind(source_id)
        .fetch_one(state.store.pool())
        .await
        .map_err(database)?,
        "extension" => {
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sip_users WHERE username=$1)")
                .bind(source_id)
                .fetch_one(state.store.pool())
                .await
                .map_err(database)?
        }
        "extension_group" => {
            return Err(invalid("分机群组尚未接入运行时，请选择接入中继或分机"));
        }
        _ => return Err(invalid("来源类型不受支持")),
    };
    if exists {
        Ok(())
    } else {
        Err(invalid(format!("授权来源 {source_id} 不存在或类型不匹配")))
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
    if body.fallback_mode.as_deref().unwrap_or("reject") != "reject" {
        return Err(invalid("当前仅支持失败时拒绝呼叫"));
    }
    Ok(mode)
}

mod did;
mod groups;
mod numbers;
mod policies;
mod pools;
mod trunks;

pub use did::*;
pub use groups::*;
pub use numbers::*;
pub use policies::*;
pub use pools::*;
pub use trunks::*;

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
