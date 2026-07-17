use crate::config::EdgeConfig;
use crate::edge_state::{AccessIpRule, EdgeState};
use crate::security::sbc::IpNet;
use crate::security::rules::refresh_anti_fraud_rules;
use call_core::{
    CallSource, CallerNumberDirectory, OutboundPolicyDirectory, Route, RouteTable, RouteTarget,
    RuntimeCallerPool, RuntimeCallerPoolMember, RuntimeEgressGroupMember, RuntimeEgressPolicy,
    RuntimeSourcePolicy,
};
use cdr_core::PostgresCdrStore;
use futures::StreamExt;
use sip_core::SipUri;
use std::collections::HashMap;
use std::io;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

type AnyError = Box<dyn std::error::Error + Send + Sync>;

pub(crate) async fn reload_routes_from_database(
    edge_state: &EdgeState,
    db: &PostgresCdrStore,
) -> Result<(), AnyError> {
    let db_routes = db.load_routes().await?;
    let db_gateways = db.load_gateways().await?;
    let gateway_details = db.list_gateways_full().await?;
    let endpoints = db.list_enabled_egress_endpoints().await?;
    let caller_numbers = db.list_numbers().await?;
    let mut gateway_map = db_gateways
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
                        max_concurrent.and_then(|c| if c > 0 { Some(c) } else { None }),
                    ),
                )
            },
        )
        .collect::<HashMap<_, _>>();
    for endpoint in endpoints {
        if let Some(gateway) = gateway_map.get_mut(&endpoint.trunk_id) {
            gateway.0 = endpoint.host;
            gateway.1 = u16::try_from(endpoint.port).ok();
            gateway.2 = endpoint.transport;
        }
    }
    edge_state.replace_gateway_cache(gateway_map.iter().map(|(id, (host, _, _, _, _, _, _, _))| (host.clone(), id.clone())));
    edge_state
        .call_manager
        .update_caller_numbers(CallerNumberDirectory::new(
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
                    .map(|gateway_id| (number.number, gateway_id))
            }),
        ));
    refresh_termination_runtime(edge_state, db, &gateway_details, &now_hhmm_or_current()).await?;

    let mut routes = Vec::new();
    let now_hhmm = cdr_core::current_hhmm();
    for (id, prefix, priority, gateway_id, cost, weight, time_start, time_end) in db_routes {
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
        if let Some((host, port, transport, max_capacity, caller_id_mode, virtual_caller, prefix_rules, max_concurrent)) =
            gateway_map.get(&gateway_id)
        {
            let mut target = RouteTarget::new(&gateway_id, host.clone(), *port);
            target.transport = Some(transport.clone());
            target.max_capacity = *max_capacity;
            target.caller_id_mode = caller_id_mode.clone();
            target.virtual_caller = virtual_caller.clone();
            target.prefix_rules = prefix_rules.clone();
            target.max_concurrent = *max_concurrent;
            routes.push(Route::with_cost_and_weight(
                id,
                prefix,
                priority,
                cost,
                weight as u32,
                target,
            ));
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
    edge_state.call_manager.update_outbound_policies(OutboundPolicyDirectory::new(
        owners,
        allocations.into_iter().filter(|item| item.enabled).map(|item| {
            (item.number, CallSource::new(item.source_type, item.source_id))
        }),
        policies.into_iter().filter(|item| item.enabled).filter_map(runtime_policy),
        runtime_pools(pools, pool_members),
        group_members.into_iter().filter(|member| {
            enabled_groups.contains(&member.group_id)
                && route_time_is_active(
                    Some(now_hhmm),
                    member.time_start.as_deref(),
                    member.time_end.as_deref(),
                )
        }).map(|member| RuntimeEgressGroupMember {
            group_id: member.group_id,
            gateway_id: member.egress_trunk_id,
            destination_prefix: member.destination_prefix,
        }),
    ));

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
        members_by_pool.entry(member.pool_id).or_default().push(RuntimeCallerPoolMember {
            number: member.number,
            priority: member.priority,
            weight: u32::try_from(member.weight).unwrap_or(1),
        });
    }
    pools.into_iter().filter(|pool| pool.enabled).map(|pool| RuntimeCallerPool {
        id: pool.id.clone(),
        owner: CallSource::new(pool.owner_source_type, pool.owner_source_id),
        members: members_by_pool.remove(&pool.id).unwrap_or_default(),
    }).collect()
}

async fn refresh_access_sources(
    edge_state: &EdgeState,
    db: &PostgresCdrStore,
    gateways: &[cdr_core::SipGateway],
) -> Result<(), AnyError> {
    let access_trunks = gateways.iter().filter(|gateway| {
        gateway.enabled.unwrap_or(true) && gateway.role.as_deref() == Some("access")
    }).collect::<Vec<_>>();

    let access_modes = access_trunks.iter().map(|gateway| {
        (gateway.id.clone(), gateway.access_auth_mode.clone().unwrap_or_default())
    }).collect::<HashMap<_, _>>();

    let username_to_trunk_id = access_trunks.iter().filter_map(|gateway| {
        gateway.access_username.clone().filter(|u| !u.trim().is_empty()).map(|u| (u, gateway.id.clone()))
    }).collect::<HashMap<_, _>>();

    if let Ok(mut current) = edge_state.access_trunk_auth_modes.write() {
        *current = access_modes.clone();
    }
    if let Ok(mut current) = edge_state.access_username_to_trunk_id.write() {
        *current = username_to_trunk_id;
    }

    let rules = db.list_enabled_trunk_ip_rules().await?.into_iter().filter_map(|rule| {
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
    }).collect();

    let registered_users = access_trunks.iter().filter_map(|gateway| {
        let mode = gateway.access_auth_mode.as_deref().unwrap_or("none");
        let username = gateway.access_username.as_deref().unwrap_or("");
        (matches!(mode, "digest_register" | "ip_and_digest") && !username.is_empty())
            .then(|| username.to_string())
    }).collect();

    edge_state.replace_access_sources(rules, registered_users);
    Ok(())
}

fn now_hhmm_or_current() -> String {
    cdr_core::current_hhmm().unwrap_or_else(|| "00:00".to_string())
}

fn route_time_is_active(
    now: Option<&str>,
    time_start: Option<&str>,
    time_end: Option<&str>,
) -> bool {
    let (Some(now), Some(start), Some(end)) = (now, time_start, time_end) else {
        return true;
    };

    if start <= end {
        now >= start && now <= end
    } else {
        // A window such as 22:00-06:00 crosses midnight.
        now >= start || now <= end
    }
}



pub(crate) fn spawn_route_reload_listener(
    nats_url: String,
    edge_state: Arc<EdgeState>,
    db_store: Option<PostgresCdrStore>,
) {
    tokio::spawn(async move {
        let Ok(client) = async_nats::connect(&nats_url).await else {
            warn!("路由重载器无法连接到 NATS");
            return;
        };

        let Ok(mut subscriber) = client.subscribe("vos_rs.routing.reload").await else {
            warn!("路由重载器无法订阅 NATS 主题");
            return;
        };

        info!("已成功启动动态路由热加载监听协程");
        while let Some(_msg) = subscriber.next().await {
            info!("收到路由热加载 NATS 广播通知，正在从数据库刷新路由...");
            if let Some(ref db) = db_store {
                match reload_routes_from_database(&edge_state, db).await {
                    Ok(()) => {
                        refresh_anti_fraud_rules(&edge_state).await;
                        info!("动态路由热重载成功，已加载最新路由表数据！");
                    }
                    Err(e) => warn!("热加载路由失败: {}", e),
                }
            }
        }
    });
}

pub(crate) async fn warm_hot_path_redis_cache(
    edge_state: &EdgeState,
    db: Option<&PostgresCdrStore>,
) -> Result<(), AnyError> {
    let Some(db) = db else {
        return Ok(());
    };
    let Some(mut connection) = edge_state.redis_connection() else {
        return Err(std::io::Error::other("Redis connection is not initialized").into());
    };
    let (credentials, trunk_creds, rates, accounts) = tokio::try_join!(
        db.list_user_credentials(),
        db.list_trunk_credentials(),
        db.list_rates(),
        db.list_accounts(),
    )?;

    let mut pipeline = redis::pipe();
    pipeline
        .atomic()
        .del("vos_rs:auth:extensions")
        .ignore()
        .del("vos_rs:auth:trunks")
        .ignore()
        .del("vos_rs:billing:rates")
        .ignore()
        .del("vos_rs:billing:balances")
        .ignore();
    for (username, password) in credentials {
        pipeline
            .hset("vos_rs:auth:extensions", username, password)
            .ignore();
    }
    for (_trunk_id, username, password) in trunk_creds {
        pipeline
            .hset("vos_rs:auth:trunks", username, password)
            .ignore();
    }
    for rate in rates {
        pipeline
            .hset("vos_rs:billing:rates", rate.prefix, rate.rate_per_minute)
            .ignore();
    }
    for account in accounts {
        pipeline
            .hset("vos_rs:billing:balances", account.username, account.balance)
            .ignore();
    }
    pipeline.query_async::<()>(&mut connection).await?;
    info!("Redis hot-path caches warmed from PostgreSQL");
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

pub(crate) fn route_table_from_config(config: &EdgeConfig) -> Result<RouteTable, AnyError> {
    if config.default_gateway.is_empty() {
        return Ok(RouteTable::default());
    }

    let target = parse_gateway_target("default", &config.default_gateway)?;
    Ok(RouteTable::new(vec![Route::new(
        "default", "", 100, target,
    )]))
}

pub(crate) fn parse_gateway_target(gateway_id: &str, raw: &str) -> Result<RouteTarget, AnyError> {
    let value = raw.trim();
    if value.is_empty() {
        return Err(Box::new(io::Error::new(
            io::ErrorKind::InvalidInput,
            "sip_edge.routing.default_gateway must not be empty",
        )));
    }

    let uri = if value.starts_with("sip:") || value.starts_with("sips:") {
        SipUri::from_str(value)
    } else {
        SipUri::from_str(&format!("sip:{value}"))
    }
    .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error.to_string()))?;

    Ok(RouteTarget::new(gateway_id, uri.host, uri.port))
}

#[cfg(test)]
mod tests {
    use super::route_time_is_active;

    #[test]
    fn route_time_window_supports_same_day_and_overnight_ranges() {
        assert!(route_time_is_active(
            Some("12:00"),
            Some("09:00"),
            Some("18:00")
        ));
        assert!(!route_time_is_active(
            Some("08:59"),
            Some("09:00"),
            Some("18:00")
        ));
        assert!(route_time_is_active(
            Some("23:30"),
            Some("22:00"),
            Some("06:00")
        ));
        assert!(route_time_is_active(
            Some("05:30"),
            Some("22:00"),
            Some("06:00")
        ));
        assert!(!route_time_is_active(
            Some("12:00"),
            Some("22:00"),
            Some("06:00")
        ));
    }
}
