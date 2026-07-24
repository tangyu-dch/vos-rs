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
    let redis = redis::aio::ConnectionManager::new(redis_client).await?;
    let guard = Arc::new(security::RouterGuard::from_config(&config)?);
    let nodes = discovery::start(redis.clone(), config.clone()).await?;
    http::start(&config.manage_bind, Arc::clone(&nodes), redis.clone()).await?;
    let routes = routes::DialogRouteStore::new(redis, config.dialog_route_ttl_secs);
    tokio::select! {
        res = async {
            tokio::try_join!(
                proxy::run(
                    config.clone(),
                    Arc::clone(&nodes),
                    Arc::clone(&routes),
                    Arc::clone(&guard)
                ),
                tcp::run(config, nodes, routes, guard)
            )
        } => {
            if let Err(e) = res {
                tracing::error!(error = %e, "SIP Router 代理服务异常退出");
            }
        }
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("收到 SIGINT/SIGTERM 终止信号，正在优雅关闭 SIP Router 服务...");
        }
    }
    tracing::info!("SIP Router 服务优雅关闭完成");
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
