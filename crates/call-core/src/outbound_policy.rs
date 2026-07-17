use crate::{CallError, CallResult, CallerIdentity, CallerIdentityMode, GatewayId, SelectedRoute};
use std::collections::{HashMap, HashSet};

/// Authenticated origin of an outbound call.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CallSource {
    pub source_type: String,
    pub source_id: String,
}

impl CallSource {
    pub fn new(source_type: impl Into<String>, source_id: impl Into<String>) -> Self {
        Self {
            source_type: source_type.into(),
            source_id: source_id.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeSourcePolicy {
    pub source: CallSource,
    pub caller_mode: String,
    pub fixed_number: Option<String>,
    pub caller_pool_id: Option<String>,
    pub egress: RuntimeEgressPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeEgressPolicy {
    Direct(String),
    Group(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeCallerPool {
    pub id: String,
    pub owner: CallSource,
    pub members: Vec<RuntimeCallerPoolMember>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeCallerPoolMember {
    pub number: String,
    pub priority: i32,
    pub weight: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeEgressGroupMember {
    pub group_id: String,
    pub gateway_id: String,
    pub destination_prefix: String,
}

/// New source-owned outbound policy data, atomically refreshed with routes.
#[derive(Debug, Clone, Default)]
pub struct OutboundPolicyDirectory {
    owners: HashMap<String, GatewayId>,
    allocations: HashSet<(String, CallSource)>,
    policies: HashMap<CallSource, RuntimeSourcePolicy>,
    pools: HashMap<String, RuntimeCallerPool>,
    group_members: HashMap<String, Vec<RuntimeEgressGroupMember>>,
}

impl OutboundPolicyDirectory {
    pub fn new(
        owners: impl IntoIterator<Item = (String, String)>,
        allocations: impl IntoIterator<Item = (String, CallSource)>,
        policies: impl IntoIterator<Item = RuntimeSourcePolicy>,
        pools: impl IntoIterator<Item = RuntimeCallerPool>,
        group_members: impl IntoIterator<Item = RuntimeEgressGroupMember>,
    ) -> Self {
        let mut directory = Self::default();
        directory.owners.extend(
            owners
                .into_iter()
                .filter(|(number, gateway)| !number.is_empty() && !gateway.is_empty())
                .map(|(number, gateway)| (number, GatewayId::new(gateway))),
        );
        directory.allocations.extend(allocations);
        directory.policies.extend(
            policies
                .into_iter()
                .map(|policy| (policy.source.clone(), policy)),
        );
        directory
            .pools
            .extend(pools.into_iter().map(|pool| (pool.id.clone(), pool)));
        for member in group_members {
            directory
                .group_members
                .entry(member.group_id.clone())
                .or_default()
                .push(member);
        }
        directory
    }

    /// Applies an explicit source policy. `None` preserves the legacy route-target behavior.
    pub fn resolve(
        &self,
        source: &CallSource,
        original_number: &str,
        destination: &str,
        candidates: &[SelectedRoute],
        selection_key: &str,
    ) -> CallResult<Option<(CallerIdentity, Vec<SelectedRoute>)>> {
        let Some(policy) = self.policies.get(source) else {
            return Ok(None);
        };
        let allowed_gateways = self.allowed_gateways(policy, destination)?;
        let allowed_candidates = candidates
            .iter()
            .filter(|candidate| allowed_gateways.contains(&candidate.target.gateway_id))
            .cloned()
            .collect::<Vec<_>>();
        if allowed_candidates.is_empty() {
            return Err(CallError::CallerIdentityUnavailable(
                "source policy has no available termination gateway for destination".to_string(),
            ));
        }
        let (number, mode) = match policy.caller_mode.as_str() {
            "strict_passthrough" => {
                self.ensure_allocated(original_number, source)?;
                (
                    original_number.to_string(),
                    CallerIdentityMode::StrictPassthrough,
                )
            }
            "fixed_number" => {
                let number = policy.fixed_number.as_deref().ok_or_else(|| {
                    CallError::CallerIdentityUnavailable(
                        "source fixed caller number is missing".to_string(),
                    )
                })?;
                self.ensure_allocated(number, source)?;
                (number.to_string(), CallerIdentityMode::Fixed)
            }
            "virtual_pool" => (
                self.select_pool_number(policy, source, selection_key)?,
                CallerIdentityMode::Random,
            ),
            other => {
                return Err(CallError::CallerIdentityUnavailable(format!(
                    "unsupported source caller mode: {other}"
                )))
            }
        };
        let owner_gateway_id = self.owners.get(&number).cloned().ok_or_else(|| {
            CallError::CallerIdentityUnavailable(format!(
                "caller number {number} has no owner egress trunk"
            ))
        })?;
        if !allowed_gateways.contains(&owner_gateway_id) {
            return Err(CallError::CallerIdentityUnavailable(format!(
                "caller number {number} owner is outside source termination scope"
            )));
        }
        let owner_candidates = allowed_candidates
            .into_iter()
            .filter(|candidate| candidate.target.gateway_id == owner_gateway_id)
            .collect::<Vec<_>>();
        if owner_candidates.is_empty() {
            return Err(CallError::CallerIdentityUnavailable(format!(
                "caller number {number} owner gateway is unavailable"
            )));
        }
        Ok(Some((
            CallerIdentity {
                original_number: original_number.to_string(),
                presented_number: number,
                owner_gateway_id,
                mode,
            },
            owner_candidates,
        )))
    }

    fn allowed_gateways(
        &self,
        policy: &RuntimeSourcePolicy,
        destination: &str,
    ) -> CallResult<HashSet<GatewayId>> {
        let ids = match &policy.egress {
            RuntimeEgressPolicy::Direct(gateway) => vec![gateway.as_str()],
            RuntimeEgressPolicy::Group(group) => self
                .group_members
                .get(group)
                .into_iter()
                .flatten()
                .filter(|member| destination.starts_with(&member.destination_prefix))
                .map(|member| member.gateway_id.as_str())
                .collect(),
        };
        if ids.is_empty() {
            return Err(CallError::CallerIdentityUnavailable(
                "source termination scope is empty".to_string(),
            ));
        }
        Ok(ids.into_iter().map(GatewayId::new).collect())
    }

    fn ensure_allocated(&self, number: &str, source: &CallSource) -> CallResult<()> {
        if self
            .allocations
            .contains(&(number.to_string(), source.clone()))
        {
            Ok(())
        } else {
            Err(CallError::CallerIdentityUnavailable(format!(
                "caller number {number} is not allocated to this source"
            )))
        }
    }

    fn select_pool_number(
        &self,
        policy: &RuntimeSourcePolicy,
        source: &CallSource,
        selection_key: &str,
    ) -> CallResult<String> {
        let pool_id = policy.caller_pool_id.as_deref().ok_or_else(|| {
            CallError::CallerIdentityUnavailable("source caller pool is missing".to_string())
        })?;
        let pool = self.pools.get(pool_id).ok_or_else(|| {
            CallError::CallerIdentityUnavailable("source caller pool is unavailable".to_string())
        })?;
        if pool.owner != *source {
            return Err(CallError::CallerIdentityUnavailable(
                "caller pool belongs to another source".to_string(),
            ));
        }
        let highest_priority = pool.members.iter().map(|member| member.priority).max();
        let eligible = pool
            .members
            .iter()
            .filter(|member| Some(member.priority) == highest_priority)
            .filter(|member| {
                self.allocations
                    .contains(&(member.number.clone(), source.clone()))
            })
            .collect::<Vec<_>>();
        let total_weight = eligible
            .iter()
            .map(|member| member.weight.max(1))
            .sum::<u32>();
        if total_weight == 0 {
            return Err(CallError::CallerIdentityUnavailable(
                "caller pool has no allocated member".to_string(),
            ));
        }
        let mut choice = stable_hash(selection_key) % u64::from(total_weight);
        for member in eligible {
            let weight = u64::from(member.weight.max(1));
            if choice < weight {
                return Ok(member.number.clone());
            }
            choice -= weight;
        }
        Err(CallError::CallerIdentityUnavailable(
            "caller pool selection failed".to_string(),
        ))
    }
}

fn stable_hash(value: &str) -> u64 {
    value.bytes().fold(0xcbf29ce484222325_u64, |hash, byte| {
        hash.wrapping_mul(0x100000001b3) ^ u64::from(byte)
    })
}
