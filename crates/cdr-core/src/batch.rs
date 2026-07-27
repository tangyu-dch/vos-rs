use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::sleep;
use tracing::{error, info};

use crate::models::CdrEvent;
use crate::PostgresCdrStore;

/// 高吞吐量 CDR 批处理通道配置。
#[derive(Debug, Clone)]
pub struct CdrBatchConfig {
    pub max_batch_size: usize,
    pub flush_interval_ms: u64,
    pub channel_capacity: usize,
}

impl Default for CdrBatchConfig {
    fn default() -> Self {
        Self {
            max_batch_size: 500,
            flush_interval_ms: 100,
            channel_capacity: 10000,
        }
    }
}

/// 高吞吐量 CDR 异步批处理通道。
/// 通过 MPSC 异步缓冲 + 批量落盘 + 定时刷新，提升每秒处理并发 (CPS)。
pub struct CdrBatchChannel {
    tx: mpsc::Sender<CdrEvent>,
}

impl CdrBatchChannel {
    /// 启动批量消费 Channel 与后台 Task。
    pub fn spawn(store: PostgresCdrStore, config: CdrBatchConfig) -> Self {
        let (tx, rx) = mpsc::channel(config.channel_capacity);
        let store = Arc::new(store);

        tokio::spawn(async move {
            run_batch_worker(rx, store, config).await;
        });

        Self { tx }
    }

    /// 发送 CDR 事件到批处理队列。
    pub async fn send(&self, event: CdrEvent) -> Result<(), mpsc::error::SendError<CdrEvent>> {
        self.tx.send(event).await
    }

    /// 尝试以非阻塞方式发送 CDR 事件。
    #[allow(clippy::result_large_err)]
    pub fn try_send(&self, event: CdrEvent) -> Result<(), mpsc::error::TrySendError<CdrEvent>> {
        self.tx.try_send(event)
    }
}

async fn run_batch_worker(
    mut rx: mpsc::Receiver<CdrEvent>,
    store: Arc<PostgresCdrStore>,
    config: CdrBatchConfig,
) {
    let mut batch = Vec::with_capacity(config.max_batch_size);
    let flush_interval = Duration::from_millis(config.flush_interval_ms);

    loop {
        let timeout = sleep(flush_interval);
        tokio::pin!(timeout);

        tokio::select! {
            maybe_event = rx.recv() => {
                match maybe_event {
                    Some(event) => {
                        batch.push(event);
                        if batch.len() >= config.max_batch_size {
                            flush_batch(&store, &mut batch).await;
                        }
                    }
                    None => {
                        if !batch.is_empty() {
                            flush_batch(&store, &mut batch).await;
                        }
                        info!("CDR 批处理工作 Task 已平滑停止");
                        break;
                    }
                }
            }
            _ = &mut timeout => {
                if !batch.is_empty() {
                    flush_batch(&store, &mut batch).await;
                }
            }
        }
    }
}

async fn flush_batch(store: &PostgresCdrStore, batch: &mut Vec<CdrEvent>) {
    if let Err(e) = store.insert_events_batch(batch).await {
        error!(error = %e, count = batch.len(), "CDR 批量写入数据库失败");
    }
    batch.clear();
}
