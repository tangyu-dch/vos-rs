use crate::pool_selection::PoolSelectionCursor;
use crate::{
    CallError, CallResult, CallerIdentity, CallerIdentityMode, CallerPoolStrategy,
    CdrAuditSnapshot, GatewayHealthTracker, GatewayId, Route, RouteTable, SelectedRoute,
};
use sip_core::SipUri;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

type CallerIdentitySelections = Vec<(CallerIdentity, Vec<SelectedRoute>)>;

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
    pub strategy: CallerPoolStrategy,
    pub members: Vec<RuntimeCallerPoolMember>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeCallerPoolMember {
    pub number: String,
    pub priority: i32,
    pub weight: u32,
    pub max_concurrent: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeEgressGroupMember {
    pub group_id: String,
    pub gateway_id: String,
    pub destination_prefix: String,
    pub priority: i32,
    pub weight: u32,
}

/// New source-owned outbound policy data, atomically refreshed with routes.
#[derive(Debug, Clone, Default)]
pub struct OutboundPolicyDirectory {
    owners: HashMap<String, (GatewayId, u32)>,
    allocations: HashSet<(String, CallSource)>,
    policies: HashMap<CallSource, RuntimeSourcePolicy>,
    pools: HashMap<String, RuntimeCallerPool>,
    group_members: HashMap<String, Vec<RuntimeEgressGroupMember>>,
    egress_routes: HashMap<GatewayId, Vec<Route>>,
    selection_cursors: HashMap<String, Arc<PoolSelectionCursor>>,
}

impl OutboundPolicyDirectory {
    /// Returns the policy metadata that must remain explainable after configuration changes.
    pub fn audit_snapshot(&self, source: &CallSource) -> CdrAuditSnapshot {
        let mut audit = CdrAuditSnapshot {
            source_type: Some(source.source_type.clone()),
            source_id: Some(source.source_id.clone()),
            ingress_trunk_id: (source.source_type == "trunk").then(|| source.source_id.clone()),
            ..CdrAuditSnapshot::default()
        };
        if let Some(policy) = self.policies.get(source) {
            audit.caller_mode = Some(policy.caller_mode.clone());
            audit.caller_pool_id.clone_from(&policy.caller_pool_id);
            audit.caller_selection = policy
                .caller_pool_id
                .as_ref()
                .and_then(|pool_id| self.pools.get(pool_id))
                .map(|pool| pool_strategy_name(pool.strategy).to_string())
                .or_else(|| Some(policy.caller_mode.clone()));
        }
        audit
    }

    pub fn new(
        owners: impl IntoIterator<Item = (String, String, i32)>,
        allocations: impl IntoIterator<Item = (String, CallSource)>,
        policies: impl IntoIterator<Item = RuntimeSourcePolicy>,
        pools: impl IntoIterator<Item = RuntimeCallerPool>,
        group_members: impl IntoIterator<Item = RuntimeEgressGroupMember>,
    ) -> Self {
        let mut directory = Self::default();
        directory.owners.extend(
            owners
                .into_iter()
                .filter(|(number, gateway, _)| !number.is_empty() && !gateway.is_empty())
                .map(|(number, gateway, max_concurrent)| {
                    (
                        number,
                        (
                            GatewayId::new(gateway),
                            u32::try_from(max_concurrent).unwrap_or(0),
                        ),
                    )
                }),
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
        directory.selection_cursors.extend(
            directory
                .pools
                .keys()
                .map(|id| (id.clone(), Arc::new(PoolSelectionCursor::default()))),
        );
        for member in group_members {
            directory
                .group_members
                .entry(member.group_id.clone())
                .or_default()
                .push(member);
        }
        directory
    }

    /// Attaches the current signaling endpoints for every termination gateway.
    pub fn with_egress_routes(
        mut self,
        routes: impl IntoIterator<Item = (String, Vec<Route>)>,
    ) -> Self {
        self.egress_routes.extend(
            routes
                .into_iter()
                .map(|(gateway_id, routes)| (GatewayId::new(gateway_id), routes)),
        );
        self
    }

    /// Builds termination candidates directly from a source policy.
    ///
    /// `None` means the source has no explicit policy and legacy routes should
    /// remain authoritative.
    pub fn select_source_candidates(
        &self,
        source: &CallSource,
        destination: &SipUri,
        direction: &str,
        health: Option<&GatewayHealthTracker>,
    ) -> CallResult<Option<Vec<SelectedRoute>>> {
        let Some(policy) = self.policies.get(source) else {
            return Ok(None);
        };
        // Direct candidate construction is enabled only after the runtime has
        // supplied its endpoint directory. This preserves compatibility for
        // embedders that still provide explicit legacy routes.
        if self.egress_routes.is_empty() {
            return Ok(None);
        }
        let destination_number = destination.user.as_deref().unwrap_or_default();
        let routes = self.policy_routes(policy, destination_number)?;
        let table = RouteTable::new(routes);
        let candidates = match health {
            Some(health) => {
                table.select_healthy_candidates(destination, health, Some(direction))?
            }
            None => table.select_candidates_for_direction(destination, direction)?,
        };
        Ok(Some(candidates))
    }

    fn policy_routes(
        &self,
        policy: &RuntimeSourcePolicy,
        destination: &str,
    ) -> CallResult<Vec<Route>> {
        let members = match &policy.egress {
            RuntimeEgressPolicy::Direct(gateway_id) => vec![RuntimeEgressGroupMember {
                group_id: String::new(),
                gateway_id: gateway_id.clone(),
                destination_prefix: String::new(),
                priority: 0,
                weight: 100,
            }],
            RuntimeEgressPolicy::Group(group_id) => self
                .group_members
                .get(group_id)
                .into_iter()
                .flatten()
                .filter(|member| destination.starts_with(&member.destination_prefix))
                .cloned()
                .collect(),
        };
        let mut routes = Vec::new();
        for member in members {
            let gateway_id = GatewayId::new(&member.gateway_id);
            let Some(endpoint_routes) = self.egress_routes.get(&gateway_id) else {
                continue;
            };
            let priority = u16::try_from(member.priority.max(0)).unwrap_or(u16::MAX);
            for endpoint_route in endpoint_routes {
                routes.push(
                    Route::with_cost_and_weight(
                        format!("policy:{}:{}", member.gateway_id, endpoint_route.id),
                        member.destination_prefix.clone(),
                        priority,
                        0.0,
                        member.weight.max(1),
                        endpoint_route.target.clone(),
                    )
                    .with_endpoint_priority(endpoint_route.endpoint_priority),
                );
            }
        }
        if routes.is_empty() {
            return Err(CallError::CallerIdentityUnavailable(
                "source policy has no configured termination endpoint for destination".to_string(),
            ));
        }
        Ok(routes)
    }

    /// Preserves stateful selection cursors for unchanged pools during a configuration refresh.
    pub fn inherit_selection_state(&mut self, previous: &Self) {
        for (pool_id, pool) in &self.pools {
            let unchanged_strategy = previous
                .pools
                .get(pool_id)
                .is_some_and(|previous_pool| previous_pool.strategy == pool.strategy);
            if unchanged_strategy {
                if let Some(cursor) = previous.selection_cursors.get(pool_id) {
                    self.selection_cursors
                        .insert(pool_id.clone(), Arc::clone(cursor));
                }
            }
        }
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
        Ok(self
            .resolve_with_alternatives(
                source,
                original_number,
                destination,
                candidates,
                selection_key,
            )?
            .and_then(|mut selections| (!selections.is_empty()).then(|| selections.remove(0))))
    }

    pub(crate) fn resolve_with_alternatives(
        &self,
        source: &CallSource,
        original_number: &str,
        destination: &str,
        candidates: &[SelectedRoute],
        selection_key: &str,
    ) -> CallResult<Option<CallerIdentitySelections>> {
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
        let (numbers, mode) = match policy.caller_mode.as_str() {
            "strict_passthrough" => {
                self.ensure_allocated(original_number, source)?;
                (
                    vec![(original_number.to_string(), None)],
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
                (vec![(number.to_string(), None)], CallerIdentityMode::Fixed)
            }
            "virtual_pool" => (
                self.select_pool_numbers(policy, source, selection_key)?,
                CallerIdentityMode::Random,
            ),
            other => {
                return Err(CallError::CallerIdentityUnavailable(format!(
                    "unsupported source caller mode: {other}"
                )))
            }
        };
        let mut selections = Vec::with_capacity(numbers.len());
        for (number, pool_capacity) in numbers {
            selections.push(self.resolve_number(
                original_number,
                number,
                mode,
                pool_capacity,
                &allowed_gateways,
                &allowed_candidates,
            )?);
        }
        Ok(Some(selections))
    }

    fn resolve_number(
        &self,
        original_number: &str,
        number: String,
        mode: CallerIdentityMode,
        pool_capacity: Option<u32>,
        allowed_gateways: &HashSet<GatewayId>,
        allowed_candidates: &[SelectedRoute],
    ) -> CallResult<(CallerIdentity, Vec<SelectedRoute>)> {
        let (owner_gateway_id, inventory_capacity) =
            self.owners.get(&number).cloned().ok_or_else(|| {
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
            .iter()
            .filter(|candidate| candidate.target.gateway_id == owner_gateway_id)
            .cloned()
            .collect::<Vec<_>>();
        if owner_candidates.is_empty() {
            return Err(CallError::CallerIdentityUnavailable(format!(
                "caller number {number} owner gateway is unavailable"
            )));
        }
        Ok((
            CallerIdentity {
                original_number: original_number.to_string(),
                presented_number: number,
                owner_gateway_id,
                mode,
                max_concurrent: effective_capacity(pool_capacity, inventory_capacity),
            },
            owner_candidates,
        ))
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

    fn select_pool_numbers(
        &self,
        policy: &RuntimeSourcePolicy,
        source: &CallSource,
        selection_key: &str,
    ) -> CallResult<Vec<(String, Option<u32>)>> {
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
        let mut eligible = eligible;
        eligible.sort_by(|left, right| left.number.cmp(&right.number));
        eligible.dedup_by(|left, right| left.number == right.number);
        if eligible.is_empty() {
            return Err(CallError::CallerIdentityUnavailable(
                "caller pool has no allocated member".to_string(),
            ));
        }
        let weights = eligible
            .iter()
            .map(|member| member.weight)
            .collect::<Vec<_>>();
        let cursor = self.selection_cursors.get(pool_id).ok_or_else(|| {
            CallError::CallerIdentityUnavailable("caller pool selector is unavailable".to_string())
        })?;
        let index = cursor
            .select_index(pool.strategy, pool_id, selection_key, &weights)
            .ok_or_else(|| {
                CallError::CallerIdentityUnavailable("caller pool selection failed".to_string())
            })?;
        Ok((0..eligible.len())
            .map(|offset| {
                let member = eligible[(index + offset) % eligible.len()];
                (member.number.clone(), Some(member.max_concurrent))
            })
            .collect())
    }
}

fn effective_capacity(pool_capacity: Option<u32>, inventory_capacity: u32) -> u32 {
    match (
        pool_capacity.filter(|capacity| *capacity > 0),
        inventory_capacity,
    ) {
        (Some(pool), inventory) if inventory > 0 => pool.min(inventory),
        (Some(pool), _) => pool,
        (None, inventory) => inventory,
    }
}

#[cfg(test)]
mod capacity_tests {
    use super::effective_capacity;

    #[test]
    fn pool_capacity_cannot_exceed_inventory_capacity() {
        assert_eq!(effective_capacity(Some(20), 10), 10);
        assert_eq!(effective_capacity(Some(5), 10), 5);
        assert_eq!(effective_capacity(Some(5), 0), 5);
        assert_eq!(effective_capacity(Some(0), 10), 10);
        assert_eq!(effective_capacity(None, 0), 0);
    }
}

#[cfg(test)]
mod pool_fallback_tests {
    use super::*;
    use crate::RouteTarget;
    use sip_core::SipUri;
    use std::str::FromStr;

    fn candidate(gateway: &str) -> SelectedRoute {
        SelectedRoute {
            route_id: format!("route-{gateway}"),
            target: RouteTarget::new(gateway, format!("{gateway}.example.com"), Some(5060)),
            outbound_uri: SipUri::from_str(&format!("sip:callee@{gateway}.example.com"))
                .expect("valid test URI"),
        }
    }

    fn directory(members: Vec<RuntimeCallerPoolMember>) -> OutboundPolicyDirectory {
        let source = CallSource::new("trunk", "access-a");
        let allocations = ["10001", "10002", "10003"]
            .into_iter()
            .map(|number| (number.to_string(), source.clone()))
            .collect::<Vec<_>>();
        OutboundPolicyDirectory::new(
            [
                ("10001".to_string(), "egress-a".to_string(), 1),
                ("10002".to_string(), "egress-b".to_string(), 1),
                ("10003".to_string(), "egress-a".to_string(), 1),
            ],
            allocations,
            [RuntimeSourcePolicy {
                source: source.clone(),
                caller_mode: "virtual_pool".to_string(),
                fixed_number: None,
                caller_pool_id: Some("pool-a".to_string()),
                egress: RuntimeEgressPolicy::Group("group-a".to_string()),
            }],
            [RuntimeCallerPool {
                id: "pool-a".to_string(),
                owner: source,
                strategy: CallerPoolStrategy::Priority,
                members,
            }],
            [
                RuntimeEgressGroupMember {
                    group_id: "group-a".to_string(),
                    gateway_id: "egress-a".to_string(),
                    destination_prefix: String::new(),
                    priority: 100,
                    weight: 1,
                },
                RuntimeEgressGroupMember {
                    group_id: "group-a".to_string(),
                    gateway_id: "egress-b".to_string(),
                    destination_prefix: String::new(),
                    priority: 100,
                    weight: 1,
                },
            ],
        )
    }

    #[test]
    fn pool_fallback_keeps_each_number_pinned_to_its_owner_gateway() {
        let directory = directory(vec![
            RuntimeCallerPoolMember {
                number: "10001".to_string(),
                priority: 100,
                weight: 1,
                max_concurrent: 1,
            },
            RuntimeCallerPoolMember {
                number: "10002".to_string(),
                priority: 100,
                weight: 1,
                max_concurrent: 1,
            },
        ]);
        let selections = directory
            .resolve_with_alternatives(
                &CallSource::new("trunk", "access-a"),
                "original",
                "callee",
                &[candidate("egress-a"), candidate("egress-b")],
                "call-a",
            )
            .expect("policy should resolve")
            .expect("source has policy");

        assert_eq!(selections.len(), 2);
        assert_eq!(selections[0].0.presented_number, "10001");
        assert_eq!(selections[0].0.owner_gateway_id.as_str(), "egress-a");
        assert!(selections[0]
            .1
            .iter()
            .all(|route| route.target.gateway_id == selections[0].0.owner_gateway_id));
        assert_eq!(selections[1].0.presented_number, "10002");
        assert_eq!(selections[1].0.owner_gateway_id.as_str(), "egress-b");
        assert!(selections[1]
            .1
            .iter()
            .all(|route| route.target.gateway_id == selections[1].0.owner_gateway_id));
    }

    #[test]
    fn pool_fallback_does_not_cross_the_existing_priority_boundary() {
        let directory = directory(vec![
            RuntimeCallerPoolMember {
                number: "10001".to_string(),
                priority: 100,
                weight: 1,
                max_concurrent: 1,
            },
            RuntimeCallerPoolMember {
                number: "10003".to_string(),
                priority: 90,
                weight: 1,
                max_concurrent: 1,
            },
        ]);
        let selections = directory
            .resolve_with_alternatives(
                &CallSource::new("trunk", "access-a"),
                "original",
                "callee",
                &[candidate("egress-a"), candidate("egress-b")],
                "call-a",
            )
            .expect("policy should resolve")
            .expect("source has policy");

        assert_eq!(selections.len(), 1);
        assert_eq!(selections[0].0.presented_number, "10001");
    }

    #[test]
    fn group_policy_builds_candidates_without_legacy_routes() {
        let source = CallSource::new("trunk", "access-a");
        let directory = OutboundPolicyDirectory::new(
            [],
            [],
            [RuntimeSourcePolicy {
                source: source.clone(),
                caller_mode: "strict_passthrough".to_string(),
                fixed_number: None,
                caller_pool_id: None,
                egress: RuntimeEgressPolicy::Group("group-a".to_string()),
            }],
            [],
            [
                RuntimeEgressGroupMember {
                    group_id: "group-a".to_string(),
                    gateway_id: "egress-a".to_string(),
                    destination_prefix: "13".to_string(),
                    priority: 100,
                    weight: 1,
                },
                RuntimeEgressGroupMember {
                    group_id: "group-a".to_string(),
                    gateway_id: "egress-b".to_string(),
                    destination_prefix: String::new(),
                    priority: 200,
                    weight: 1,
                },
            ],
        )
        .with_egress_routes([
            (
                "egress-a".to_string(),
                vec![Route::new(
                    "endpoint-a",
                    "",
                    0,
                    RouteTarget::new("egress-a", "a.example.com", Some(5060)),
                )],
            ),
            (
                "egress-b".to_string(),
                vec![Route::new(
                    "endpoint-b",
                    "",
                    0,
                    RouteTarget::new("egress-b", "b.example.com", Some(5060)),
                )],
            ),
        ]);
        let destination =
            SipUri::from_str("sip:13800138000@example.com").expect("valid destination URI");

        let candidates = directory
            .select_source_candidates(&source, &destination, "outbound", None)
            .expect("group policy should resolve")
            .expect("source has an explicit group policy");

        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].target.gateway_id.as_str(), "egress-a");
        assert_eq!(candidates[1].target.gateway_id.as_str(), "egress-b");
    }
}

fn pool_strategy_name(strategy: CallerPoolStrategy) -> &'static str {
    match strategy {
        CallerPoolStrategy::Random => "random",
        CallerPoolStrategy::WeightedRandom => "weighted_random",
        CallerPoolStrategy::RoundRobin => "round_robin",
        CallerPoolStrategy::StableHash => "stable_hash",
        CallerPoolStrategy::Priority => "priority",
    }
}
