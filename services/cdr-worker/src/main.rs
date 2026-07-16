//! # cdr-worker：CDR 异步写入服务
//!
//! 本服务从 NATS JetStream 消费 CDR 事件，批量写入 PostgreSQL。
//!
//! ## 架构
//!
//! ```text
//! sip-edge → NATS JetStream → cdr-worker → PostgreSQL
//! ```
//!
//! ## 功能
//!
//! - **批量写入**：累积多条 CDR 后批量 INSERT，减少数据库压力
//! - **超时刷新**：批量未满时按超时强制刷新
//! - **死信队列**：写入失败的 CDR 进入 DLQ，避免阻塞
//! - **指数退避**：数据库写入失败时指数退避重试
//! - **幂等性**：按 call_id 去重，防止重复写入
//!
//! ## 配置
//!
//! PostgreSQL、Redis、NATS、日志及批处理参数统一从 `config.yaml` 读取，
//! 仅 `VOS_RS_CONFIG_FILE` 用于选择配置文件路径。

use async_nats::jetstream::{self, consumer::PullConsumer, stream, AckKind};
use cdr_core::{CdrEvent, PostgresCdrStore, DEFAULT_CDR_STREAM, DEFAULT_CDR_SUBJECT};
use futures::StreamExt;
use std::env;
use std::time::{Duration, Instant};
use tracing::{debug, error, info, warn};
use tracing_subscriber::EnvFilter;

const DEFAULT_NATS_URL: &str = "nats://127.0.0.1:4222";
const DEFAULT_CDR_CONSUMER: &str = "vos-rs-cdr-worker";
const DEFAULT_CDR_DLQ_SUBJECT: &str = "vos-rs.cdrs.dlq";
const DEFAULT_CDR_DLQ_STREAM: &str = "VOS_RS_CDR_DLQ";
const DEFAULT_BATCH_SIZE: usize = 50;
const DEFAULT_BATCH_TIMEOUT_MS: u64 = 100;
const DEFAULT_MAX_DELIVERIES: u32 = 5;
const DEFAULT_NAK_DELAY_MS: u64 = 1000;
const DEFAULT_DB_RETRY_ATTEMPTS: u32 = 3;

type AnyError = Box<dyn std::error::Error + Send + Sync>;

#[derive(serde::Deserialize, Debug, Default)]
struct CdrWorkerConfig {
    connections: Option<ConnectionsSection>,
    cdr_worker: Option<CdrWorkerSection>,
}

#[derive(serde::Deserialize, Debug, Default)]
struct ConnectionsSection {
    database: Option<DatabaseSection>,
    redis: Option<RedisSection>,
    nats: Option<NatsSection>,
}

#[derive(serde::Deserialize, Debug, Default)]
struct RedisSection {
    host: Option<String>,
    port: Option<u16>,
    password: Option<String>,
    database: Option<u16>,
}

#[derive(serde::Deserialize, Debug, Default)]
struct DatabaseSection {
    host: Option<String>,
    port: Option<u16>,
    username: Option<String>,
    password: Option<String>,
    database: Option<String>,
    max_connections: Option<u32>,
}

#[derive(serde::Deserialize, Debug, Default)]
struct NatsSection {
    url: Option<String>,
    cdr_stream: Option<String>,
    cdr_subject: Option<String>,
}

#[derive(serde::Deserialize, Debug, Default)]
struct CdrWorkerSection {
    queue: Option<QueueSection>,
    batch_settings: Option<BatchSettingsSection>,
}

#[derive(serde::Deserialize, Debug, Default)]
struct QueueSection {
    nats_cdr_consumer: Option<String>,
    nats_cdr_dlq_subject: Option<String>,
    nats_cdr_dlq_stream: Option<String>,
}

#[derive(serde::Deserialize, Debug, Default)]
struct BatchSettingsSection {
    max_batch_size: Option<usize>,
    batch_timeout_ms: Option<u64>,
    max_deliveries: Option<u32>,
    nak_delay_ms: Option<u64>,
    db_retry_attempts: Option<u32>,
}

#[tokio::main]
async fn main() -> Result<(), AnyError> {
    init_tracing(&config_logging_filter("cdr_worker=info"));

    let config_file_path =
        env::var("VOS_RS_CONFIG_FILE").unwrap_or_else(|_| "config.yaml".to_string());
    let config_content = std::fs::read_to_string(&config_file_path).unwrap_or_default();
    let config: CdrWorkerConfig = serde_yaml::from_str(&config_content).unwrap_or_default();

    let conn_section = config.connections.unwrap_or_default();
    let db_section = conn_section.database.unwrap_or_default();
    let nats_section = conn_section.nats.unwrap_or_default();
    let worker_section = config.cdr_worker.unwrap_or_default();
    let queue_section = worker_section.queue.unwrap_or_default();
    let batch_section = worker_section.batch_settings.unwrap_or_default();

    let database_url = if let (Some(host), Some(port), Some(username), Some(database)) = (
        db_section.host.clone(),
        db_section.port,
        db_section.username.clone(),
        db_section.database.clone(),
    ) {
        let password = db_section.password.clone().unwrap_or_default();
        if password.is_empty() {
            format!("postgres://{}@{}:{}/{}", username, host, port, database)
        } else {
            format!(
                "postgres://{}:{}@{}:{}/{}",
                username, password, host, port, database
            )
        }
    } else {
        return Err("PostgreSQL 数据库连接配置缺失，请检查 config.yaml".into());
    };

    let redis_section = conn_section.redis.unwrap_or_default();
    let redis_url =
        if let (Some(host), Some(port)) = (redis_section.host.clone(), redis_section.port) {
            let password = redis_section.password.clone().unwrap_or_default();
            let db = redis_section.database.unwrap_or(0);
            if password.is_empty() {
                format!("redis://{}:{}/{}", host, port, db)
            } else {
                format!("redis://:{}@{}:{}/{}", password, host, port, db)
            }
        } else {
            "redis://127.0.0.1:6379".to_string()
        };
    let nats_url = nats_section
        .url
        .unwrap_or_else(|| DEFAULT_NATS_URL.to_string());
    let stream_name = nats_section
        .cdr_stream
        .unwrap_or_else(|| DEFAULT_CDR_STREAM.to_string());
    let subject = nats_section
        .cdr_subject
        .unwrap_or_else(|| DEFAULT_CDR_SUBJECT.to_string());
    let consumer_name = queue_section
        .nats_cdr_consumer
        .unwrap_or_else(|| DEFAULT_CDR_CONSUMER.to_string());
    let dlq_subject = queue_section
        .nats_cdr_dlq_subject
        .unwrap_or_else(|| DEFAULT_CDR_DLQ_SUBJECT.to_string());
    let dlq_stream_name = queue_section
        .nats_cdr_dlq_stream
        .unwrap_or_else(|| DEFAULT_CDR_DLQ_STREAM.to_string());

    let max_batch_size = batch_section.max_batch_size.unwrap_or(DEFAULT_BATCH_SIZE);
    let batch_timeout_ms = batch_section
        .batch_timeout_ms
        .unwrap_or(DEFAULT_BATCH_TIMEOUT_MS);
    let batch_timeout = Duration::from_millis(batch_timeout_ms);
    let max_deliveries = batch_section
        .max_deliveries
        .unwrap_or(DEFAULT_MAX_DELIVERIES);
    let nak_delay_ms = batch_section.nak_delay_ms.unwrap_or(DEFAULT_NAK_DELAY_MS);
    let db_retry_attempts = batch_section
        .db_retry_attempts
        .unwrap_or(DEFAULT_DB_RETRY_ATTEMPTS);

    // 强制校验 PostgreSQL
    let max_connections = db_section.max_connections.unwrap_or(10);
    let store = match PostgresCdrStore::connect(&database_url, max_connections).await {
        Ok(s) => s,
        Err(e) => {
            error!(database_url, error = %e, "PostgreSQL 数据库连接失败。VOS-RS 必须有 PostgreSQL 运行！");
            return Err(e.into());
        }
    };

    // 强制校验 Redis
    let redis_client = match redis::Client::open(redis_url.clone()) {
        Ok(c) => c,
        Err(e) => {
            error!(redis_url, error = %e, "Redis 客户端打开失败。VOS-RS 必须有 Redis 运行！");
            return Err(e.into());
        }
    };
    let _redis_conn = match redis_client.get_multiplexed_tokio_connection().await {
        Ok(conn) => conn,
        Err(e) => {
            error!(redis_url, error = %e, "Redis 连接失败，请检查服务状态。VOS-RS 必须有 Redis 运行！");
            return Err(e.into());
        }
    };
    info!("Redis 存储连接成功 (必须要求)");

    let (jetstream, consumer) = connect_consumer(
        &nats_url,
        &stream_name,
        &subject,
        &consumer_name,
        max_deliveries,
        &dlq_stream_name,
        &dlq_subject,
    )
    .await?;

    info!(
        nats_url,
        stream = stream_name,
        subject,
        consumer = consumer_name,
        dlq_subject,
        dlq_stream = dlq_stream_name,
        max_batch_size,
        batch_timeout_ms,
        max_deliveries,
        nak_delay_ms,
        db_retry_attempts,
        "cdr-worker started"
    );

    let mut messages = consumer.messages().await?;
    let mut batch = Vec::new();
    let mut first_msg_time: Option<Instant> = None;

    loop {
        let timeout_fut = if let Some(first_time) = first_msg_time {
            let elapsed = Instant::now().duration_since(first_time);
            let remaining = batch_timeout.checked_sub(elapsed).unwrap_or(Duration::ZERO);
            tokio::time::sleep(remaining)
        } else {
            tokio::time::sleep(Duration::from_secs(3600 * 24))
        };

        tokio::select! {
            message = messages.next() => {
                let Some(message) = message else {
                    warn!("NATS JetStream consumer ended");
                    break;
                };

                let message = match message {
                    Ok(message) => message,
                    Err(error) => {
                        warn!(%error, "failed to receive CDR message from NATS");
                        continue;
                    }
                };

                if first_msg_time.is_none() {
                    first_msg_time = Some(Instant::now());
                }

                batch.push(message);

                if batch.len() >= max_batch_size {
                    process_batch(
                        &batch,
                        &store,
                        &jetstream,
                        &dlq_subject,
                        max_deliveries,
                        Duration::from_millis(nak_delay_ms),
                        db_retry_attempts,
                    )
                    .await?;
                    batch.clear();
                    first_msg_time = None;
                }
            }
            _ = timeout_fut, if first_msg_time.is_some() => {
                process_batch(
                    &batch,
                    &store,
                    &jetstream,
                    &dlq_subject,
                    max_deliveries,
                    Duration::from_millis(nak_delay_ms),
                    db_retry_attempts,
                )
                .await?;
                batch.clear();
                first_msg_time = None;
            }
            _ = tokio::signal::ctrl_c() => {
                info!("shutdown signal received");
                if !batch.is_empty() {
                    info!(size = batch.len(), "persisting remaining batch before shutdown");
                    let _ = process_batch(
                        &batch,
                        &store,
                        &jetstream,
                        &dlq_subject,
                        max_deliveries,
                        Duration::from_millis(nak_delay_ms),
                        db_retry_attempts,
                    )
                    .await;
                }
                break;
            }
        }
    }

    Ok(())
}

async fn process_batch(
    batch: &[jetstream::message::Message],
    store: &PostgresCdrStore,
    jetstream: &jetstream::Context,
    dlq_subject: &str,
    max_deliveries: u32,
    nak_delay: Duration,
    db_retry_attempts: u32,
) -> Result<(), AnyError> {
    if batch.is_empty() {
        return Ok(());
    }

    let mut valid_events = Vec::new();
    let mut valid_messages = Vec::new();

    for msg in batch {
        match CdrEvent::from_json_slice(&msg.payload) {
            Ok(event) => {
                valid_events.push(event);
                valid_messages.push(msg);
            }
            Err(error) => {
                // Poison message: JSON deserialization failed — this is a permanent error.
                // Route to DLQ and terminate so NATS will not redeliver it.
                error!(%error, "invalid CDR event JSON; routing to DLQ as poison message");
                if publish_to_dlq(jetstream, dlq_subject, &msg.payload).await {
                    if let Err(term_err) = msg.ack_with(AckKind::Term).await {
                        error!(%term_err, "failed to term poison message");
                    }
                } else if let Err(nak_err) = msg.ack_with(AckKind::Nak(Some(nak_delay))).await {
                    error!(%nak_err, "failed to nak poison message after DLQ failure");
                }
            }
        }
    }

    if valid_events.is_empty() {
        return Ok(());
    }

    let mut success = false;
    let mut last_error = String::new();
    for attempt in 1..=db_retry_attempts {
        match store.insert_events_batch(&valid_events).await {
            Ok(_) => {
                success = true;
                break;
            }
            Err(error) => {
                last_error = error.to_string();
                warn!(%error, attempt, "failed to insert batch of CDR events; retrying...");
                if attempt < db_retry_attempts {
                    tokio::time::sleep(Duration::from_millis(100 * attempt as u64)).await;
                }
            }
        }
    }

    if success {
        for msg in valid_messages {
            if let Err(error) = msg.ack().await {
                error!(%error, "failed to ack message after successful insert");
            }
        }
        debug!(
            count = valid_events.len(),
            "successfully persisted batch of CDR events"
        );
    } else {
        // DB insert failed after all in-process retries.
        // Check NATS delivery count to decide: nak for redelivery or route to DLQ.
        let max_delivery = valid_messages
            .iter()
            .filter_map(|msg| msg.info().ok())
            .map(|m| m.delivered)
            .max()
            .unwrap_or(1);

        if max_delivery < max_deliveries as i64 {
            // Transient failure: nak so NATS will redeliver after a delay.
            warn!(
                count = valid_events.len(),
                max_delivery,
                max_deliveries,
                %last_error,
                "DB insert failed; naking messages for NATS redelivery"
            );
            for msg in valid_messages {
                // AckKind::Nak(Some(delay)) instructs NATS to delay redelivery by `nak_delay`.
                if let Err(nak_err) = msg.ack_with(AckKind::Nak(Some(nak_delay))).await {
                    error!(%nak_err, "failed to nak message");
                }
            }
        } else {
            // Poison message: exceeded max delivery attempts — route to DLQ and terminate.
            error!(
                count = valid_events.len(),
                max_delivery,
                max_deliveries,
                %last_error,
                "exceeded max deliveries; routing batch to DLQ as poison messages"
            );
            for (event, msg) in valid_events.iter().zip(valid_messages.iter()) {
                if publish_to_dlq(jetstream, dlq_subject, &msg.payload).await {
                    if let Err(term_err) = msg.ack_with(AckKind::Term).await {
                        error!(%term_err, call_id = %event.call_id, "failed to term poison message");
                    }
                } else if let Err(nak_err) = msg.ack_with(AckKind::Nak(Some(nak_delay))).await {
                    error!(%nak_err, call_id = %event.call_id, "failed to nak message after DLQ failure");
                }
            }
        }
    }

    Ok(())
}

/// Publish a payload to the DLQ subject and await the JetStream publish ack.
async fn publish_to_dlq(jetstream: &jetstream::Context, dlq_subject: &str, payload: &[u8]) -> bool {
    match jetstream
        .publish(dlq_subject.to_string(), payload.to_vec().into())
        .await
    {
        Ok(ack_future) => {
            if let Err(ack_err) = ack_future.await {
                error!(%ack_err, "DLQ publish ack failed");
                false
            } else {
                true
            }
        }
        Err(pub_err) => {
            error!(%pub_err, "failed to publish message to DLQ");
            false
        }
    }
}

fn init_tracing(filter: &str) {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new(filter))
        .init();
}

fn config_logging_filter(default: &str) -> String {
    let path = env::var("VOS_RS_CONFIG_FILE").unwrap_or_else(|_| "config.yaml".to_string());
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

#[allow(clippy::too_many_arguments)]
async fn connect_consumer(
    nats_url: &str,
    stream_name: &str,
    subject: &str,
    consumer_name: &str,
    max_deliveries: u32,
    dlq_stream_name: &str,
    dlq_subject: &str,
) -> Result<(jetstream::Context, PullConsumer), AnyError> {
    let client = async_nats::connect(nats_url).await?;
    let jetstream = jetstream::new(client);

    // Create the primary CDR stream (WorkQueue retention).
    let stream = jetstream
        .get_or_create_stream(stream::Config {
            name: stream_name.to_string(),
            subjects: vec![subject.to_string()],
            retention: stream::RetentionPolicy::WorkQueue,
            ..Default::default()
        })
        .await?;

    // Create or ensure the DLQ stream exists so poison messages are durably persisted.
    jetstream
        .get_or_create_stream(stream::Config {
            name: dlq_stream_name.to_string(),
            subjects: vec![dlq_subject.to_string()],
            retention: stream::RetentionPolicy::Limits,
            ..Default::default()
        })
        .await?;

    let consumer = stream
        .get_or_create_consumer(
            consumer_name,
            jetstream::consumer::pull::Config {
                durable_name: Some(consumer_name.to_string()),
                filter_subject: subject.to_string(),
                max_deliver: max_deliveries as i64,
                ack_policy: jetstream::consumer::AckPolicy::Explicit,
                ..Default::default()
            },
        )
        .await?;

    Ok((jetstream, consumer))
}

#[cfg(test)]
mod tests {
    use super::*;

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
