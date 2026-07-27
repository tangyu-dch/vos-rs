//! # cdr-worker：CDR 异步写入服务
//!
//! 本服务从 NATS JetStream 消费 CDR 事件，通过 `cdr-core::CdrBatchChannel` 批量写入 PostgreSQL。

mod config;
mod consumer;

use cdr_core::PostgresCdrStore;
use config::{load_config, AnyError};
use consumer::{connect_consumer, run_consumer_loop};
use tracing::{error, info};

#[tokio::main]
async fn main() -> Result<(), AnyError> {
    init_tracing(&config_logging_filter("cdr_worker=info"));

    let cfg = load_config()?;

    let store = match PostgresCdrStore::connect(&cfg.database_url, cfg.max_connections).await {
        Ok(s) => s,
        Err(e) => {
            error!(error = %e, "PostgreSQL 数据库连接失败。VOS-RS 必须有 PostgreSQL 运行！");
            return Err(e.into());
        }
    };

    let redis_client = match redis::Client::open(cfg.redis_url.clone()) {
        Ok(c) => c,
        Err(e) => {
            error!(error = %e, "Redis 客户端打开失败。VOS-RS 必须有 Redis 运行！");
            return Err(e.into());
        }
    };
    if let Err(e) = redis_client.get_multiplexed_tokio_connection().await {
        error!(error = %e, "Redis 连接失败，请检查服务状态。VOS-RS 必须有 Redis 运行！");
        return Err(e.into());
    }
    info!("Redis 存储连接成功 (必须要求)");

    let (jetstream, consumer) = connect_consumer(
        &cfg.nats_url,
        &cfg.stream_name,
        &cfg.subject,
        &cfg.consumer_name,
        cfg.max_deliveries,
        &cfg.dlq_stream_name,
        &cfg.dlq_subject,
    )
    .await?;

    info!(
        nats_url = cfg.nats_url,
        stream = cfg.stream_name,
        subject = cfg.subject,
        consumer = cfg.consumer_name,
        dlq_subject = cfg.dlq_subject,
        dlq_stream = cfg.dlq_stream_name,
        max_batch_size = cfg.max_batch_size,
        batch_timeout_ms = cfg.batch_timeout_ms,
        max_deliveries = cfg.max_deliveries,
        nak_delay_ms = cfg.nak_delay_ms,
        db_retry_attempts = cfg.db_retry_attempts,
        "cdr-worker started"
    );

    run_consumer_loop(consumer, jetstream, store, &cfg).await?;

    Ok(())
}

fn init_tracing(filter: &str) {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new(filter))
        .init();
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

#[cfg(test)]
mod tests {
    use super::config::*;

    #[test]
    fn test_config_defaults() {
        assert_eq!(DEFAULT_BATCH_SIZE, 50);
        assert_eq!(DEFAULT_BATCH_TIMEOUT_MS, 100);
        assert_eq!(DEFAULT_CDR_DLQ_SUBJECT, "vos-rs.cdrs.dlq");
        assert_eq!(DEFAULT_CDR_DLQ_STREAM, "VOS_RS_CDR_DLQ");
        assert_eq!(DEFAULT_MAX_DELIVERIES, 5);
        assert_eq!(DEFAULT_NAK_DELAY_MS, 1000);
        assert_eq!(DEFAULT_DB_RETRY_ATTEMPTS, 3);
    }
}
