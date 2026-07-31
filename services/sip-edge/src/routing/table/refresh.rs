use crate::edge_state::{AccessIpRule, EdgeState};
use crate::security::sbc::IpNet;
use call_core::{
    CallSource, CallerNumberDirectory, CallerPoolStrategy, OutboundPolicyDirectory, Route,
    RouteTable, RouteTarget, RuntimeCallerPool, RuntimeCallerPoolMember, RuntimeEgressGroupMember,
    RuntimeEgressPolicy, RuntimeSourcePolicy,
};
use cdr_core::PostgresCdrStore;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tracing::warn;

use super::helpers::{now_hhmm_or_current, route_time_is_active};
use super::AnyError;

pub(crate) async fn reload_routes_from_database(
    edge_state: &EdgeState,
    db: &PostgresCdrStore,
) -> Result<(), AnyError> {
    let db_routes = db.load_routes().await?;
    let db_gateways = db.load_gateways().await?;
    let gateway_details = db.list_gateways_full().await?;
    let endpoints = db.list_enabled_egress_endpoints().await?;
    let caller_numbers = db.list_numbers().await?;
    let gateway_map = db_gateways
        .into_iter()
        .filter(|gw| {
            let role = gw.9.as_deref().unwrap_or("egress");
            role != "access"
        })
        .map(
            |(
                id,
                host,
                port,
                transport,
                max_capacity,
                caller_id_mode,
                virtual_caller,
                prefix_rules,
                max_concurrent,
                _role,
            )| {
                (
                    id,
                    (
                        host,
                        port,
                        transport,
                        max_capacity.filter(|capacity| *capacity > 0),
                        caller_id_mode,
                        virtual_caller,
                        prefix_rules,
                        max_concurrent.filter(|&c| c > 0),
                    ),
                )
            },
        )
        .collect::<HashMap<_, _>>();
    let mut gateway_identities = gateway_map
        .iter()
        .map(|(id, (host, port, _, _, _, _, _, _))| {
            (host.clone(), port.unwrap_or(5060), id.clone())
        })
        .collect::<Vec<_>>();
    let mut endpoint_routes = HashMap::<String, Vec<Route>>::new();
    for endpoint in endpoints {
        if let Some(gateway) = gateway_map.get(&endpoint.trunk_id) {
            if let Ok(port) = u16::try_from(endpoint.port) {
                gateway_identities.push((endpoint.host.clone(), port, endpoint.trunk_id.clone()));
                let mut target = RouteTarget::new(&endpoint.trunk_id, endpoint.host, Some(port));
                target.transport = Some(endpoint.transport);
                target.max_capacity = gateway.3;
                target.caller_id_mode.clone_from(&gateway.4);
                target.virtual_caller.clone_from(&gateway.5);
                target.prefix_rules.clone_from(&gateway.6);
                target.max_concurrent = gateway.7;
                endpoint_routes.entry(endpoint.trunk_id).or_default().push(
                    Route::new(format!("endpoint-{}", endpoint.id), "", 0, target)
                        .with_endpoint_priority(endpoint.priority),
                );
            }
        }
    }
    for (gateway_id, gateway) in &gateway_map {
        endpoint_routes
            .entry(gateway_id.clone())
            .or_insert_with(|| {
                let mut target = RouteTarget::new(gateway_id, gateway.0.clone(), gateway.1);
                target.transport = Some(gateway.2.clone());
                target.max_capacity = gateway.3;
                target.caller_id_mode.clone_from(&gateway.4);
                target.virtual_caller.clone_from(&gateway.5);
                target.prefix_rules.clone_from(&gateway.6);
                target.max_concurrent = gateway.7;
                vec![Route::new(format!("gateway-{gateway_id}"), "", 0, target)]
            });
    }
    edge_state.replace_gateway_endpoint_cache(gateway_identities);
    edge_state
        .call_manager
        .update_caller_numbers(CallerNumberDirectory::new_with_capacity(
            caller_numbers.into_iter().filter_map(|number| {
                let enabled = matches!(
                    number.status.trim().to_ascii_lowercase().as_str(),
                    "available" | "assigned" | "active"
                );
                let outbound = matches!(
                    number
                        .direction
                        .as_deref()
                        .unwrap_or("bidirectional")
                        .trim()
                        .to_ascii_lowercase()
                        .as_str(),
                    "outbound" | "both" | "bidirectional"
                );
                (enabled && outbound)
                    .then_some(number.gateway_id)
                    .flatten()
                    .map(|gateway_id| {
                        (
                            number.number,
                            gateway_id,
                            u32::try_from(number.max_concurrent.unwrap_or(0)).unwrap_or(0),
                        )
                    })
            }),
        ));
    refresh_termination_runtime(
        edge_state,
        db,
        &gateway_details,
        &now_hhmm_or_current(),
        endpoint_routes.clone(),
    )
    .await?;

    let mut routes = Vec::new();
    let now_hhmm = cdr_core::current_hhmm();
    for (id, prefix, priority, gateway_id, cost, weight, time_start, time_end, tenant_id, strip_prefix, add_prefix) in db_routes {
        let Ok(priority) = u16::try_from(priority) else {
            warn!(route_id = %id, priority, "skipping route with an invalid priority");
            continue;
        };
        if !cost.is_finite() || cost < 0.0 || weight <= 0 {
            warn!(route_id = %id, cost, weight, "skipping route with invalid cost or weight");
            continue;
        }
        if !route_time_is_active(
            now_hhmm.as_deref(),
            time_start.as_deref(),
            time_end.as_deref(),
        ) {
            continue;
        }
        if let Some(targets) = endpoint_routes.get(&gateway_id) {
            for endpoint_route in targets {
                let mut route = Route::with_cost_and_weight(
                    format!("{id}:{}", endpoint_route.id),
                    prefix.clone(),
                    priority,
                    cost,
                    weight as u32,
                    endpoint_route.target.clone(),
                )
                .with_endpoint_priority(endpoint_route.endpoint_priority);
                route.tenant_id = tenant_id.clone();
                route.strip_prefix = strip_prefix.clone();
                route.add_prefix = add_prefix.clone();
                routes.push(route);
            }
        }
    }
    // An empty database table is authoritative and must clear stale in-memory routes.
    edge_state
        .call_manager
        .update_routes(RouteTable::new(routes));
    Ok(())
}

async fn refresh_termination_runtime(
    edge_state: &EdgeState,
    db: &PostgresCdrStore,
    gateways: &[cdr_core::SipGateway],
    now_hhmm: &str,
    endpoint_routes: HashMap<String, Vec<Route>>,
) -> Result<(), AnyError> {
    let owners = db.list_runtime_number_owners().await?;
    let allocations = db.list_number_allocations(None).await?;
    let policies = db.list_source_outbound_policies().await?;
    let pools = db.list_caller_pools().await?;
    let pool_members = db.list_enabled_caller_pool_members().await?;
    let groups = db.list_egress_groups().await?;
    let group_members = db.list_enabled_egress_group_members().await?;
    let dids = db.list_did_destinations().await?;
    let billing_accounts = db.list_trunk_billing_accounts().await?;

    let enabled_groups = groups
        .into_iter()
        .filter(|group| group.enabled)
        .map(|group| group.id)
        .collect::<std::collections::HashSet<_>>();
    edge_state.call_manager.update_outbound_policies(
        OutboundPolicyDirectory::new(
            owners,
            allocations
                .into_iter()
                .filter(|item| item.enabled)
                .map(|item| {
                    (
                        item.number,
                        CallSource::new(item.source_type, item.source_id),
                    )
                }),
            policies
                .into_iter()
                .filter(|item| item.enabled)
                .filter_map(runtime_policy),
            runtime_pools(pools, pool_members),
            group_members
                .into_iter()
                .filter(|member| {
                    enabled_groups.contains(&member.group_id)
                        && route_time_is_active(
                            Some(now_hhmm),
                            member.time_start.as_deref(),
                            member.time_end.as_deref(),
                        )
                })
                .map(|member| RuntimeEgressGroupMember {
                    group_id: member.group_id,
                    gateway_id: member.egress_trunk_id,
                    destination_prefix: member.destination_prefix,
                    priority: member.priority,
                    weight: u32::try_from(member.weight).unwrap_or(1),
                }),
        )
        .with_egress_routes(endpoint_routes),
    );

    edge_state.replace_did_destinations(dids.into_iter().map(|d| (d.number.clone(), d)).collect());
    if let Ok(mut current) = edge_state.trunk_billing_accounts.write() {
        *current = billing_accounts.into_iter().collect();
    }

    refresh_access_sources(edge_state, db, gateways).await
}

fn runtime_policy(policy: cdr_core::SourceOutboundPolicy) -> Option<RuntimeSourcePolicy> {
    let egress = match policy.egress_mode.as_str() {
        "direct" => RuntimeEgressPolicy::Direct(policy.direct_egress_trunk_id?),
        "group" => RuntimeEgressPolicy::Group(policy.egress_group_id?),
        _ => return None,
    };
    Some(RuntimeSourcePolicy {
        source: CallSource::new(policy.source_type, policy.source_id),
        caller_mode: policy.caller_mode,
        fixed_number: policy.fixed_number,
        caller_pool_id: policy.caller_pool_id,
        egress,
    })
}

fn runtime_pools(
    pools: Vec<cdr_core::CallerPool>,
    members: Vec<cdr_core::CallerPoolMember>,
) -> Vec<RuntimeCallerPool> {
    let mut members_by_pool = HashMap::<String, Vec<RuntimeCallerPoolMember>>::new();
    for member in members {
        members_by_pool
            .entry(member.pool_id)
            .or_default()
            .push(RuntimeCallerPoolMember {
                number: member.number,
                priority: member.priority,
                weight: u32::try_from(member.weight).unwrap_or(1),
                max_concurrent: u32::try_from(member.max_concurrent).unwrap_or(0),
            });
    }
    pools
        .into_iter()
        .filter(|pool| pool.enabled)
        .filter_map(|pool| {
            let Some(strategy) = CallerPoolStrategy::from_config(&pool.strategy) else {
                tracing::warn!(
                    pool_id = %pool.id,
                    strategy = %pool.strategy,
                    "skipping caller pool with unsupported selection strategy"
                );
                return None;
            };
            Some(RuntimeCallerPool {
                id: pool.id.clone(),
                owner: CallSource::new(pool.owner_source_type, pool.owner_source_id),
                strategy,
                members: members_by_pool.remove(&pool.id).unwrap_or_default(),
            })
        })
        .collect()
}

async fn refresh_access_sources(
    edge_state: &EdgeState,
    db: &PostgresCdrStore,
    gateways: &[cdr_core::SipGateway],
) -> Result<(), AnyError> {
    let access_trunks = gateways
        .iter()
        .filter(|gateway| {
            gateway.enabled.unwrap_or(true) && gateway.role.as_deref() == Some("access")
        })
        .collect::<Vec<_>>();

    let access_modes = access_trunks
        .iter()
        .map(|gateway| {
            (
                gateway.id.clone(),
                gateway.access_auth_mode.clone().unwrap_or_default(),
            )
        })
        .collect::<HashMap<_, _>>();

    let username_to_trunk_id = access_trunks
        .iter()
        .filter_map(|gateway| {
            gateway
                .access_username
                .clone()
                .filter(|u| !u.trim().is_empty())
                .map(|u| (u, gateway.id.clone()))
        })
        .collect::<HashMap<_, _>>();

    if let Ok(mut current) = edge_state.access_trunk_auth_modes.write() {
        *current = access_modes.clone();
    }
    if let Ok(mut current) = edge_state.access_username_to_trunk_id.write() {
        *current = username_to_trunk_id;
    }

    let rules = db
        .list_enabled_trunk_ip_rules()
        .await?
        .into_iter()
        .filter_map(|rule| {
            let mode = access_modes.get(&rule.trunk_id)?;
            if mode == "ip_allowlist" || mode == "ip_and_digest" {
                let network = IpNet::parse(&rule.cidr).ok()?;
                Some(AccessIpRule {
                    trunk_id: rule.trunk_id,
                    network,
                    source_port: rule.source_port.and_then(|port| u16::try_from(port).ok()),
                    transport: rule.transport,
                })
            } else {
                None
            }
        })
        .collect();

    let registered_users = access_trunks
        .iter()
        .filter_map(|gateway| {
            let mode = gateway.access_auth_mode.as_deref().unwrap_or("none");
            let username = gateway.access_username.as_deref().unwrap_or("");
            (matches!(mode, "digest_register" | "ip_and_digest") && !username.is_empty())
                .then(|| username.to_string())
        })
        .collect();

    edge_state.replace_access_sources(rules, registered_users);
    Ok(())
}

pub(crate) fn spawn_periodic_route_refresh(edge_state: Arc<EdgeState>, db: PostgresCdrStore) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        interval.tick().await; // skip first immediate tick
        loop {
            interval.tick().await;
            if let Err(e) = reload_routes_from_database(&edge_state, &db).await {
                warn!(%e, "periodic route refresh failed");
            }
        }
    });
}
