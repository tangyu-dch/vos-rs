use super::*;

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
    ensure_trunk_role(&state, &id, "access").await?;
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
    let auth_mode: String = sqlx::query_scalar(
        "SELECT access_auth_mode FROM sip_gateways WHERE id=$1 AND role='access'",
    )
    .bind(&id)
    .fetch_one(state.store.pool())
    .await
    .map_err(database)?;
    if matches!(auth_mode.as_str(), "ip_allowlist" | "ip_and_digest")
        && !rules.iter().any(|rule| rule.enabled)
    {
        return Err(invalid("IP 白名单认证必须至少保留一条已启用的来源地址"));
    }
    state
        .store
        .replace_trunk_ip_rules(&id, &rules)
        .await
        .map_err(database)?;
    crate::resources::routes::publish_route_reload(&state.nats_client).await;
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
    ensure_trunk_role(&state, &id, "egress").await?;
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
    crate::resources::routes::publish_route_reload(&state.nats_client).await;
    Ok(StatusCode::OK)
}
