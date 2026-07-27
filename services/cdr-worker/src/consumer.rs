use async_nats::jetstream::{self, consumer::PullConsumer, stream, AckKind};
use cdr_core::{CdrBatchChannel, CdrBatchConfig, CdrEvent, PostgresCdrStore};
use futures::StreamExt;
use std::time::Duration;
use tracing::{error, info, warn};

use crate::config::{AnyError, ResolvedConfig};

#[allow(clippy::too_many_arguments)]
pub async fn connect_consumer(
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

    let stream = jetstream
        .get_or_create_stream(stream::Config {
            name: stream_name.to_string(),
            subjects: vec![subject.to_string()],
            retention: stream::RetentionPolicy::WorkQueue,
            ..Default::default()
        })
        .await?;

    jetstream
        .get_or_create_stream(stream::Config {
            name: dlq_stream_name.to_string(),
            subjects: vec![dlq_subject.to_string()],
            retention: stream::RetentionPolicy::Limits,
            ..Default::default()
        })
        .await?;

    let consumer = match stream.get_consumer(consumer_name).await {
        Ok(c) => c,
        Err(_) => {
            stream
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
                .await?
        }
    };

    Ok((jetstream, consumer))
}

pub async fn run_consumer_loop(
    consumer: PullConsumer,
    jetstream: jetstream::Context,
    store: PostgresCdrStore,
    cfg: &ResolvedConfig,
) -> Result<(), AnyError> {
    let batch_channel = CdrBatchChannel::spawn(
        store,
        CdrBatchConfig {
            max_batch_size: cfg.max_batch_size,
            flush_interval_ms: cfg.batch_timeout_ms,
            channel_capacity: 10000,
        },
    );

    let mut messages = consumer.messages().await?;

    loop {
        tokio::select! {
            message = messages.next() => {
                let Some(message) = message else {
                    warn!("NATS JetStream consumer ended");
                    break;
                };

                let msg = match message {
                    Ok(msg) => msg,
                    Err(error) => {
                        warn!(%error, "failed to receive CDR message from NATS");
                        continue;
                    }
                };

                process_nats_message(&msg, &batch_channel, &jetstream, &cfg.dlq_subject, Duration::from_millis(cfg.nak_delay_ms)).await;
            }
            _ = tokio::signal::ctrl_c() => {
                info!("shutdown signal received in consumer loop");
                break;
            }
        }
    }

    Ok(())
}

async fn process_nats_message(
    msg: &jetstream::message::Message,
    batch_channel: &CdrBatchChannel,
    jetstream: &jetstream::Context,
    dlq_subject: &str,
    nak_delay: Duration,
) {
    match CdrEvent::from_json_slice(&msg.payload) {
        Ok(event) => {
            if let Err(err) = batch_channel.send(event).await {
                error!(%err, "failed to send CDR event to CdrBatchChannel");
            }
            if let Err(ack_err) = msg.ack().await {
                error!(%ack_err, "failed to ack NATS message after batch send");
            }
        }
        Err(error) => {
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

pub async fn publish_to_dlq(
    jetstream: &jetstream::Context,
    dlq_subject: &str,
    payload: &[u8],
) -> bool {
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
