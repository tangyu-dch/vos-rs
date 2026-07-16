use crate::config::EdgeConfig;
use crate::edge_state::CdrSinks;
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

    Ok(CdrSinks { postgres })
}

pub(crate) async fn flush_cdr_batch(
    cdr_sinks: &CdrSinks,
    cdrs: &[CallCdr],
) -> Result<(), AnyError> {
    if cdrs.is_empty() {
        return Ok(());
    }

    if let Some(cdr_store) = &cdr_sinks.postgres {
        for cdr in cdrs {
            cdr_store.insert_call_cdr(cdr).await?;
        }
        debug!(count = cdrs.len(), "persisted batch CDRs to PostgreSQL");
        return Ok(());
    }

    debug!(count = cdrs.len(), "discarded batch CDRs without CDR sink");
    Ok(())
}

pub(crate) async fn flush_cdr_batch_with_retry_and_wal(cdr_sinks: &CdrSinks, batch: &[CallCdr]) {
    if batch.is_empty() {
        return;
    }

    let mut success = false;
    for attempt in 1..=3 {
        match flush_cdr_batch(cdr_sinks, batch).await {
            Ok(_) => {
                success = true;
                break;
            }
            Err(e) => {
                warn!(attempt, error = %e, "批量发送 CDR 失败，正在重试...");
                tokio::time::sleep(Duration::from_millis(1000)).await;
            }
        }
    }

    if !success {
        tracing::error!("致命错误: 连续 3 次批量刷新 CDR 失败！为防止数据丢失，正将 CDR 数据追加写入本地 logs/cdr_dlq.jsonl 死信归档...");

        let _ = tokio::fs::create_dir_all("logs").await;
        if let Ok(mut file) = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("logs/cdr_dlq.jsonl")
            .await
        {
            use tokio::io::AsyncWriteExt;
            for cdr in batch {
                if let Ok(json_str) = serde_json::to_string(cdr) {
                    let _ = file.write_all(format!("{}\n", json_str).as_bytes()).await;
                }
            }
            let _ = file.flush().await;
            info!(
                count = batch.len(),
                "已成功将未送达的批量 CDR 追加归档至本地 logs/cdr_dlq.jsonl 中"
            );
        } else {
            tracing::error!(
                count = batch.len(),
                "极其严重: 写入本地 logs/cdr_dlq.jsonl 文件失败！CDR 数据将丢弃！"
            );
        }
    }
}
