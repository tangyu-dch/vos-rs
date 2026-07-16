use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use std::io;
use tracing::{info, warn};
use call_core::{Route, RouteTable, RouteTarget};
use cdr_core::PostgresCdrStore;
use sip_core::SipUri;
use std::str::FromStr;
use futures::StreamExt;
use crate::config::EdgeConfig;
use crate::edge_state::EdgeState;
use crate::security::rules::refresh_anti_fraud_rules;

type AnyError = Box<dyn std::error::Error + Send + Sync>;

pub(crate) async fn reload_routes_from_database(
    edge_state: &EdgeState,
    db: &PostgresCdrStore,
) -> Result<(), AnyError> {
    let db_routes = db.load_routes().await?;
    let db_gateways = db.load_gateways().await?;
    let gateway_map: HashMap<String, (String, Option<u16>, String, Option<u32>)> = db_gateways
        .into_iter()
        .map(|(id, host, port, transport, cap, _, _, _)| (id, (host, port, transport, cap)))
        .collect();
    edge_state.replace_gateway_cache(gateway_map.values().map(|(host, _, _, _)| host));

    let mut routes = Vec::new();
    let now_hhmm = cdr_core::current_hhmm();
    for (id, prefix, priority, gateway_id, cost, weight, time_start, time_end) in db_routes {
        if let (Some(start), Some(end)) = (time_start.as_ref(), time_end.as_ref()) {
            if let Some(now) = now_hhmm.as_deref() {
                if now < start.as_str() || now > end.as_str() {
                    continue;
                }
            }
        }
        if let Some((host, port, _transport, max_capacity)) = gateway_map.get(&gateway_id) {
            let mut target = RouteTarget::new(&gateway_id, host.clone(), *port);
            target.max_capacity = *max_capacity;
            routes.push(Route::with_cost_and_weight(
                id,
                prefix,
                priority as u16,
                cost,
                weight.max(0) as u32,
                target,
            ));
        }
    }
    if !routes.is_empty() {
        edge_state
            .call_manager
            .update_routes(RouteTable::new(routes));
    }
    Ok(())
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
    let (credentials, rates, accounts) = tokio::try_join!(
        db.list_user_credentials(),
        db.list_rates(),
        db.list_accounts(),
    )?;

    let mut pipeline = redis::pipe();
    pipeline
        .atomic()
        .del("vos_rs:auth_users")
        .ignore()
        .del("vos_rs:billing:rates")
        .ignore()
        .del("vos_rs:billing:balances")
        .ignore();
    for (username, password) in credentials {
        pipeline
            .hset("vos_rs:auth_users", username, password)
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
