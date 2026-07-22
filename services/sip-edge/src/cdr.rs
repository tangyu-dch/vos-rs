use crate::cdr_spool::CdrSpool;
use crate::config::EdgeConfig;
use crate::edge_state::CdrSinks;
use crate::nats_cdr::NatsCdrPublisher;
use call_core::CallCdr;
use cdr_core::PostgresCdrStore;
use std::time::Duration;
use tracing::{debug, info, warn};

type AnyError = Box<dyn std::error::Error + Send + Sync>;

pub(crate) async fn cdr_sinks_from_config(config: &EdgeConfig) -> Result<CdrSinks, AnyError> {
    let postgres = match &config.database_url {
        Some(database_url) if !database_url.trim().is_empty() => {
            let store =
                PostgresCdrStore::connect(database_url, config.database_max_connections).await?;
            info!("PostgreSQL CDR persistence enabled");
            Some(store)
        }
        _ => {
            return Err("PostgreSQL 数据库连接未配置，数据库为 system 运行的必须依赖项".into());
        }
    };

    let nats = match (
        config.nats_url.as_deref(),
        config.nats_cdr_subject.as_deref(),
        config.nats_cdr_stream.as_deref(),
    ) {
        (Some(url), Some(subject), Some(stream)) if !url.trim().is_empty() => {
            match NatsCdrPublisher::connect(url, subject, stream).await {
                Ok(publisher) => {
                    info!(stream, subject, "NATS JetStream CDR persistence enabled");
                    Some(publisher)
                }
                Err(error) => {
                    warn!(%error, "NATS CDR publisher unavailable; falling back to PostgreSQL with local spool protection");
                    None
                }
            }
        }
        _ => None,
    };

    Ok(CdrSinks { postgres, nats })
}

pub(crate) async fn flush_cdr_batch(
    cdr_sinks: &CdrSinks,
    cdrs: &[CallCdr],
) -> Result<(), AnyError> {
    if cdrs.is_empty() {
        return Ok(());
    }

    if let Some(publisher) = &cdr_sinks.nats {
        for cdr in cdrs {
            publisher.publish_cdr(cdr).await?;
        }
        debug!(count = cdrs.len(), "published batch CDRs to NATS JetStream");
        return Ok(());
    }

    if let Some(cdr_store) = &cdr_sinks.postgres {
        for cdr in cdrs {
            cdr_store.insert_call_cdr(cdr).await?;
        }
        debug!(count = cdrs.len(), "persisted batch CDRs to PostgreSQL");
        return Ok(());
    }

    Err(std::io::Error::other("no CDR persistence sink is available").into())
}

pub(crate) async fn flush_cdr_batch_with_retry_and_spool(
    cdr_sinks: &CdrSinks,
    spool: &CdrSpool,
    batch: &[CallCdr],
) {
    flush_cdr_batch_with_retry_policy(cdr_sinks, spool, batch, 3, Duration::from_secs(1)).await;
}

pub(crate) async fn flush_cdr_batch_with_retry_policy(
    cdr_sinks: &CdrSinks,
    spool: &CdrSpool,
    batch: &[CallCdr],
    attempts: usize,
    retry_delay: Duration,
) {
    if batch.is_empty() {
        return;
    }

    let mut success = false;
    for attempt in 1..=attempts.max(1) {
        match flush_cdr_batch(cdr_sinks, batch).await {
            Ok(_) => {
                success = true;
                break;
            }
            Err(e) => {
                warn!(attempt, error = %e, "批量发送 CDR 失败，正在重试...");
                tokio::time::sleep(retry_delay).await;
            }
        }
    }

    if !success {
        match spool.append_batch(batch) {
            Ok(()) => info!(
                count = batch.len(),
                "CDR persistence failed; batch saved to durable replay spool"
            ),
            Err(error) => tracing::error!(
                %error,
                count = batch.len(),
                "CDR persistence and durable spool append both failed"
            ),
        }
    }
}
