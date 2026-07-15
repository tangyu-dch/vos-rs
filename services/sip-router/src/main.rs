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
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    let config = RouterConfig::load()?;
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
