mod config;
mod discovery;
mod http;
mod metrics;
mod proxy;
mod routes;
mod security;
mod tcp;

use config::RouterConfig;
use std::sync::Arc;
use tracing_subscriber::EnvFilter;

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let config = RouterConfig::load()?;
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new(config_logging_filter("sip_router=info")))
        .init();
    let redis_client = redis::Client::open(config.redis_url.clone())?;
    let guard = Arc::new(security::RouterGuard::from_config(&config)?);
    let nodes = discovery::start(redis_client.clone(), config.clone()).await?;
    http::start(
        &config.manage_bind,
        Arc::clone(&nodes),
        redis_client.clone(),
    )
    .await?;
    let routes = routes::DialogRouteStore::new(redis_client, config.dialog_route_ttl_secs).await?;
    tokio::try_join!(
        proxy::run(
            config.clone(),
            Arc::clone(&nodes),
            Arc::clone(&routes),
            Arc::clone(&guard)
        ),
        tcp::run(config, nodes, routes, guard)
    )?;
    Ok(())
}

fn config_logging_filter(default: &str) -> String {
    let path = std::env::var("VOS_RS_CONFIG_FILE").unwrap_or_else(|_| "config.yaml".to_string());
    std::fs::read_to_string(path)
        .ok()
        .and_then(|content| serde_yaml::from_str::<serde_yaml::Value>(&content).ok())
        .and_then(|root| {
            root.get("logging")?
                .get("filter")?
                .as_str()
                .map(str::to_owned)
        })
        .filter(|filter| !filter.trim().is_empty())
        .unwrap_or_else(|| default.to_string())
}
