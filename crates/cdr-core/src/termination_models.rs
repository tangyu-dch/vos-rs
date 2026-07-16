//! Models for ingress authentication, caller selection and termination policy.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, PartialEq)]
pub struct TrunkIpRule {
    pub id: i64,
    pub trunk_id: String,
    pub cidr: String,
    pub source_port: Option<i32>,
    pub transport: String,
    pub description: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, PartialEq)]
pub struct EgressEndpoint {
    pub id: i64,
    pub trunk_id: String,
    pub host: String,
    pub port: i32,
    pub transport: String,
    pub priority: i32,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, PartialEq)]
pub struct NumberAllocation {
    pub id: i64,
    pub number: String,
    pub source_type: String,
    pub source_id: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, PartialEq)]
pub struct CallerPool {
    pub id: String,
    pub owner_source_type: String,
    pub owner_source_id: String,
    pub virtual_alias: String,
    pub strategy: String,
    pub fallback_mode: String,
    pub enabled: bool,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, PartialEq)]
pub struct CallerPoolMember {
    pub id: i64,
    pub pool_id: String,
    pub number: String,
    pub priority: i32,
    pub weight: i32,
    pub max_concurrent: i32,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, PartialEq)]
pub struct EgressGroup {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, PartialEq)]
pub struct EgressGroupMember {
    pub id: i64,
    pub group_id: String,
    pub egress_trunk_id: String,
    pub destination_prefix: String,
    pub priority: i32,
    pub weight: i32,
    pub time_start: Option<String>,
    pub time_end: Option<String>,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, PartialEq)]
pub struct SourceOutboundPolicy {
    pub source_type: String,
    pub source_id: String,
    pub caller_mode: String,
    pub fixed_number: Option<String>,
    pub caller_pool_id: Option<String>,
    pub egress_mode: String,
    pub direct_egress_trunk_id: Option<String>,
    pub egress_group_id: Option<String>,
    pub fallback_mode: String,
    pub enabled: bool,
    pub updated_at: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, PartialEq)]
pub struct DidDestination {
    pub number: String,
    pub tenant_id: Option<String>,
    pub target_type: String,
    pub target_id: String,
    pub enabled: bool,
    pub updated_at: OffsetDateTime,
}
