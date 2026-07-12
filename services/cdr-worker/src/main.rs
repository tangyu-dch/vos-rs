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
//! | 环境变量 | 说明 | 默认值 |
//! |---------|------|--------|
//! | `VOS_RS_DATABASE_URL` | PostgreSQL 连接 | postgres://localhost/vos_rs |
//! | `VOS_RS_NATS_URL` | NATS 地址 | nats://127.0.0.1:4222 |
//! | `VOS_RS_CDR_BATCH_SIZE` | 批量大小 | 50 |
//! | `VOS_RS_CDR_BATCH_TIMEOUT_MS` | 超时刷新 | 100ms |
//! | `VOS_RS_CDR_MAX_DELIVERIES` | 最大投递次数 | 5 |
//! | `VOS_RS_CDR_DB_RETRY_ATTEMPTS` | DB 重试次数 | 3 |

use async_nats::jetstream::{self, consumer::PullConsumer, stream, AckKind};
use cdr_core::{CdrEvent, PostgresCdrStore, DEFAULT_CDR_STREAM, DEFAULT_CDR_SUBJECT};
use futures::StreamExt;
use std::env;
use std::time::{Duration, Instant};
use tracing::{debug, error, info, warn};
use tracing_subscriber::EnvFilter;

const DATABASE_URL_ENV: &str = "VOS_RS_DATABASE_URL";
const NATS_URL_ENV: &str = "VOS_RS_NATS_URL";
const NATS_CDR_STREAM_ENV: &str = "VOS_RS_NATS_CDR_STREAM";
const NATS_CDR_SUBJECT_ENV: &str = "VOS_RS_NATS_CDR_SUBJECT";
const NATS_CDR_CONSUMER_ENV: &str = "VOS_RS_NATS_CDR_CONSUMER";
const NATS_CDR_DLQ_SUBJECT_ENV: &str = "VOS_RS_NATS_CDR_DLQ_SUBJECT";
const NATS_CDR_DLQ_STREAM_ENV: &str = "VOS_RS_NATS_CDR_DLQ_STREAM";
const CDR_BATCH_SIZE_ENV: &str = "VOS_RS_CDR_BATCH_SIZE";
const CDR_BATCH_TIMEOUT_MS_ENV: &str = "VOS_RS_CDR_BATCH_TIMEOUT_MS";
const CDR_MAX_DELIVERIES_ENV: &str = "VOS_RS_CDR_MAX_DELIVERIES";
const CDR_NAK_DELAY_MS_ENV: &str = "VOS_RS_CDR_NAK_DELAY_MS";
const CDR_DB_RETRY_ATTEMPTS_ENV: &str = "VOS_RS_CDR_DB_RETRY_ATTEMPTS";

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

#[tokio::main]
async fn main() -> Result<(), AnyError> {
    init_tracing();

    let database_url = env::var(DATABASE_URL_ENV).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("{DATABASE_URL_ENV} is required"),
        )
    })?;
    let nats_url = env::var(NATS_URL_ENV).unwrap_or_else(|_| DEFAULT_NATS_URL.to_string());
    let stream_name =
        env::var(NATS_CDR_STREAM_ENV).unwrap_or_else(|_| DEFAULT_CDR_STREAM.to_string());
    let subject =
        env::var(NATS_CDR_SUBJECT_ENV).unwrap_or_else(|_| DEFAULT_CDR_SUBJECT.to_string());
    let consumer_name =
        env::var(NATS_CDR_CONSUMER_ENV).unwrap_or_else(|_| DEFAULT_CDR_CONSUMER.to_string());
    let dlq_subject =
        env::var(NATS_CDR_DLQ_SUBJECT_ENV).unwrap_or_else(|_| DEFAULT_CDR_DLQ_SUBJECT.to_string());
    let dlq_stream_name =
        env::var(NATS_CDR_DLQ_STREAM_ENV).unwrap_or_else(|_| DEFAULT_CDR_DLQ_STREAM.to_string());

    let max_batch_size: usize = env::var(CDR_BATCH_SIZE_ENV)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_BATCH_SIZE);
    let batch_timeout_ms: u64 = env::var(CDR_BATCH_TIMEOUT_MS_ENV)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_BATCH_TIMEOUT_MS);
    let batch_timeout = Duration::from_millis(batch_timeout_ms);
    let max_deliveries: u32 = env::var(CDR_MAX_DELIVERIES_ENV)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_MAX_DELIVERIES);
    let nak_delay_ms: u64 = env::var(CDR_NAK_DELAY_MS_ENV)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_NAK_DELAY_MS);
    let db_retry_attempts: u32 = env::var(CDR_DB_RETRY_ATTEMPTS_ENV)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_DB_RETRY_ATTEMPTS);

    let store = PostgresCdrStore::connect(&database_url).await?;
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

fn init_tracing() {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("cdr_worker=info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
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
        assert_eq!(CDR_BATCH_SIZE_ENV, "VOS_RS_CDR_BATCH_SIZE");
        assert_eq!(CDR_BATCH_TIMEOUT_MS_ENV, "VOS_RS_CDR_BATCH_TIMEOUT_MS");
        assert_eq!(CDR_MAX_DELIVERIES_ENV, "VOS_RS_CDR_MAX_DELIVERIES");
        assert_eq!(CDR_NAK_DELAY_MS_ENV, "VOS_RS_CDR_NAK_DELAY_MS");
        assert_eq!(CDR_DB_RETRY_ATTEMPTS_ENV, "VOS_RS_CDR_DB_RETRY_ATTEMPTS");
        assert_eq!(NATS_CDR_DLQ_STREAM_ENV, "VOS_RS_NATS_CDR_DLQ_STREAM");
    }
}
