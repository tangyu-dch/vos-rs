//! Webhook 异步投递流水线。
//!
//! SIP 热路径只写入有界内存队列；本模块负责持久化到 NATS JetStream、
//! HTTP 签名投递、延迟重试和 Redis 投递结果记录。

use crate::config::WebhookConfig;
use crate::webhook_delivery::{deliver_event, record_delivery, retry_delay};
use async_nats::jetstream::{self, consumer::PullConsumer, stream, AckKind};
use call_core::WebhookEvent;
use futures::StreamExt;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

type AnyError = Box<dyn std::error::Error + Send + Sync>;

/// 启动 Webhook NATS 发布器和 HTTP 投递消费者。
pub async fn start_pipeline(
    config: WebhookConfig,
    nats_url: &str,
    redis_client: redis::Client,
    receiver: mpsc::Receiver<WebhookEvent>,
) -> Result<(), AnyError> {
    validate_config(&config)?;
    let (jetstream, consumer) = connect_consumer(nats_url, &config).await?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(config.request_timeout_ms))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let redis_connection = redis_client.get_multiplexed_tokio_connection().await?;

    spawn_publisher(jetstream.clone(), config.clone(), receiver);
    spawn_delivery_consumer(consumer, client, redis_connection, config);
    Ok(())
}

fn validate_config(config: &WebhookConfig) -> Result<(), AnyError> {
    if config.endpoint_url.trim().is_empty() {
        return Err("webhooks.endpoint_url 不能为空".into());
    }
    if config.signing_secret.len() < 16 {
        return Err("webhooks.signing_secret 至少需要 16 个字符".into());
    }
    let url = reqwest::Url::parse(&config.endpoint_url)?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("Webhook endpoint 仅支持 http 或 https".into());
    }
    if config.queue_capacity == 0 || config.max_deliveries == 0 {
        return Err("Webhook queue_capacity 和 max_deliveries 必须大于 0".into());
    }
    Ok(())
}

async fn connect_consumer(
    nats_url: &str,
    config: &WebhookConfig,
) -> Result<(jetstream::Context, PullConsumer), AnyError> {
    let client = async_nats::connect(nats_url).await?;
    let jetstream = jetstream::new(client);
    let stream = jetstream
        .get_or_create_stream(stream::Config {
            name: config.stream.clone(),
            subjects: vec![config.subject.clone()],
            retention: stream::RetentionPolicy::WorkQueue,
            ..Default::default()
        })
        .await?;
    let consumer = stream
        .get_or_create_consumer(
            &config.consumer,
            jetstream::consumer::pull::Config {
                durable_name: Some(config.consumer.clone()),
                filter_subject: config.subject.clone(),
                max_deliver: i64::from(config.max_deliveries),
                ack_policy: jetstream::consumer::AckPolicy::Explicit,
                ..Default::default()
            },
        )
        .await?;
    Ok((jetstream, consumer))
}

fn spawn_publisher(
    jetstream: jetstream::Context,
    config: WebhookConfig,
    mut receiver: mpsc::Receiver<WebhookEvent>,
) {
    tokio::spawn(async move {
        while let Some(event) = receiver.recv().await {
            let payload = match serde_json::to_vec(&event) {
                Ok(payload) => payload,
                Err(error) => {
                    error!(event_id = %event.event_id, %error, "Webhook 事件序列化失败");
                    continue;
                }
            };
            if let Err(error) = publish_with_retry(&jetstream, &config, payload).await {
                error!(event_id = %event.event_id, %error, "Webhook 事件写入 NATS 失败");
            }
        }
    });
}

async fn publish_with_retry(
    jetstream: &jetstream::Context,
    config: &WebhookConfig,
    payload: Vec<u8>,
) -> Result<(), AnyError> {
    let mut last_error = None;
    for attempt in 1..=3 {
        match jetstream
            .publish(config.subject.clone(), payload.clone().into())
            .await
        {
            Ok(ack) => match ack.await {
                Ok(_) => return Ok(()),
                Err(error) => last_error = Some(error.to_string()),
            },
            Err(error) => last_error = Some(error.to_string()),
        }
        tokio::time::sleep(Duration::from_millis(100 * attempt)).await;
    }
    Err(last_error
        .unwrap_or_else(|| "未知 NATS 发布错误".to_string())
        .into())
}

fn spawn_delivery_consumer(
    consumer: PullConsumer,
    client: reqwest::Client,
    mut redis_connection: redis::aio::MultiplexedConnection,
    config: WebhookConfig,
) {
    tokio::spawn(async move {
        let mut messages = match consumer.messages().await {
            Ok(messages) => messages,
            Err(error) => {
                error!(%error, "Webhook NATS Consumer 启动失败");
                return;
            }
        };
        info!(endpoint = %config.endpoint_url, "Webhook 异步投递流水线已启动");
        while let Some(message) = messages.next().await {
            let Ok(message) = message else {
                warn!("读取 Webhook NATS 消息失败");
                continue;
            };
            process_message(&client, &mut redis_connection, &config, &message).await;
        }
    });
}

async fn process_message(
    client: &reqwest::Client,
    redis_connection: &mut redis::aio::MultiplexedConnection,
    config: &WebhookConfig,
    message: &jetstream::message::Message,
) {
    let event: WebhookEvent = match serde_json::from_slice(&message.payload) {
        Ok(event) => event,
        Err(error) => {
            error!(%error, "Webhook NATS 消息协议无效，终止重试");
            let _ = message.ack_with(AckKind::Term).await;
            return;
        }
    };
    let attempt = message
        .info()
        .ok()
        .and_then(|info| u64::try_from(info.delivered).ok())
        .unwrap_or(1);
    let outcome = deliver_event(client, config, &event).await;
    let final_failure =
        !outcome.success && (!outcome.retryable || attempt >= u64::from(config.max_deliveries));
    let status = if outcome.success {
        "delivered"
    } else if final_failure {
        "failed"
    } else {
        "retrying"
    };
    let recorded =
        record_delivery(redis_connection, config, &event, attempt, status, &outcome).await;

    if outcome.success && recorded {
        if let Err(error) = message.ack().await {
            warn!(event_id = %event.event_id, %error, "Webhook 成功后 ACK 失败");
        }
    } else if final_failure {
        let _ = message.ack_with(AckKind::Term).await;
    } else {
        let delay = retry_delay(config.retry_delay_ms, attempt);
        let _ = message.ack_with(AckKind::Nak(Some(delay))).await;
    }
}

#[cfg(test)]
#[path = "tests/webhook_pipeline.rs"]
mod tests;
