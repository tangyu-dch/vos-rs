use crate::cdr::pipeline::flush_cdr_batch;
use crate::edge_state::CdrSinks;
use call_core::{CallCdr, CdrSendError, CdrSink};
use serde::Serialize;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const ACTIVE_SPOOL_FILE: &str = "active.jsonl";
const REPLAY_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Debug, Default)]
pub(crate) struct CdrPipelineMetrics {
    queue_overflow_total: AtomicU64,
    spooled_total: AtomicU64,
    replayed_total: AtomicU64,
    spool_failures_total: AtomicU64,
    pending_spool_records: AtomicU64,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub(crate) struct CdrPipelineSnapshot {
    pub(crate) queue_overflow_total: u64,
    pub(crate) spooled_total: u64,
    pub(crate) replayed_total: u64,
    pub(crate) spool_failures_total: u64,
    pub(crate) pending_spool_records: u64,
}

impl CdrPipelineMetrics {
    pub(crate) fn snapshot(&self) -> CdrPipelineSnapshot {
        CdrPipelineSnapshot {
            queue_overflow_total: self.queue_overflow_total.load(Ordering::Relaxed),
            spooled_total: self.spooled_total.load(Ordering::Relaxed),
            replayed_total: self.replayed_total.load(Ordering::Relaxed),
            spool_failures_total: self.spool_failures_total.load(Ordering::Relaxed),
            pending_spool_records: self.pending_spool_records.load(Ordering::Relaxed),
        }
    }
}

#[derive(Debug)]
struct SpoolWriter {
    file: File,
}

#[derive(Clone, Debug)]
pub(crate) struct CdrSpool {
    directory: Arc<PathBuf>,
    writer: Arc<Mutex<SpoolWriter>>,
    metrics: Arc<CdrPipelineMetrics>,
}

impl CdrSpool {
    pub(crate) fn open(directory: PathBuf) -> std::io::Result<Self> {
        std::fs::create_dir_all(&directory)?;
        let active_path = directory.join(ACTIVE_SPOOL_FILE);
        let file = open_append_file(&active_path)?;
        let pending = count_pending_records(&directory)?;
        let metrics = Arc::new(CdrPipelineMetrics::default());
        metrics
            .pending_spool_records
            .store(pending, Ordering::Relaxed);
        Ok(Self {
            directory: Arc::new(directory),
            writer: Arc::new(Mutex::new(SpoolWriter { file })),
            metrics,
        })
    }

    pub(crate) fn metrics(&self) -> Arc<CdrPipelineMetrics> {
        Arc::clone(&self.metrics)
    }

    pub(crate) fn append(&self, cdr: &CallCdr) -> std::io::Result<()> {
        let payload = serde_json::to_vec(cdr).map_err(std::io::Error::other)?;
        let mut writer = self
            .writer
            .lock()
            .map_err(|_| std::io::Error::other("CDR spool writer lock poisoned"))?;
        writer.file.write_all(&payload)?;
        writer.file.write_all(b"\n")?;
        writer.file.flush()?;
        writer.file.sync_data()?;
        self.metrics.spooled_total.fetch_add(1, Ordering::Relaxed);
        self.metrics
            .pending_spool_records
            .fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    pub(crate) fn append_batch(&self, cdrs: &[CallCdr]) -> std::io::Result<()> {
        for cdr in cdrs {
            self.append(cdr)?;
        }
        Ok(())
    }

    fn rotate_active(&self) -> std::io::Result<Option<PathBuf>> {
        let mut writer = self
            .writer
            .lock()
            .map_err(|_| std::io::Error::other("CDR spool writer lock poisoned"))?;
        writer.file.flush()?;
        writer.file.sync_data()?;
        let active_path = self.directory.join(ACTIVE_SPOOL_FILE);
        if active_path.metadata()?.len() == 0 {
            return Ok(None);
        }

        let replay_path = self
            .directory
            .join(format!("replay-{}.jsonl", uuid::Uuid::new_v4()));
        std::fs::rename(&active_path, &replay_path)?;
        writer.file = match open_append_file(&active_path) {
            Ok(file) => file,
            Err(error) => {
                let _ = std::fs::rename(&replay_path, &active_path);
                return Err(error);
            }
        };
        Ok(Some(replay_path))
    }

    fn replay_files(&self) -> std::io::Result<Vec<PathBuf>> {
        let mut files = std::fs::read_dir(self.directory.as_ref())?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("replay-") && name.ends_with(".jsonl"))
            })
            .collect::<Vec<_>>();
        files.sort();
        Ok(files)
    }

    async fn replay_once(&self, sinks: &CdrSinks) {
        if let Err(error) = self.rotate_active() {
            self.metrics
                .spool_failures_total
                .fetch_add(1, Ordering::Relaxed);
            tracing::error!(%error, "failed to rotate CDR overflow spool");
            return;
        }

        let files = match self.replay_files() {
            Ok(files) => files,
            Err(error) => {
                self.metrics
                    .spool_failures_total
                    .fetch_add(1, Ordering::Relaxed);
                tracing::error!(%error, "failed to enumerate CDR overflow spool");
                return;
            }
        };

        for path in files {
            let cdrs = match read_spool_file(&path) {
                Ok(cdrs) => cdrs,
                Err(error) => {
                    self.archive_corrupt_file(&path, error);
                    continue;
                }
            };
            if cdrs.is_empty() {
                let _ = std::fs::remove_file(&path);
                continue;
            }
            match flush_cdr_batch(sinks, &cdrs).await {
                Ok(()) => {
                    let count = cdrs.len() as u64;
                    if let Err(error) = self.complete_replay(&path, count) {
                        tracing::warn!(%error, path = %path.display(), "replayed CDR spool but could not remove segment; idempotent replay will retry");
                        continue;
                    }
                    tracing::info!(count, path = %path.display(), "replayed CDR overflow spool");
                }
                Err(error) => {
                    tracing::warn!(%error, path = %path.display(), count = cdrs.len(), "CDR spool replay deferred because persistence is unavailable");
                    break;
                }
            }
        }
    }

    fn archive_corrupt_file(&self, path: &Path, error: std::io::Error) {
        let count = count_lines(path).unwrap_or(0);
        let corrupt_path = path.with_extension("corrupt");
        if let Err(rename_error) = std::fs::rename(path, &corrupt_path) {
            tracing::error!(%error, %rename_error, path = %path.display(), "invalid CDR spool segment could not be archived");
        } else {
            saturating_sub(&self.metrics.pending_spool_records, count);
            tracing::error!(%error, path = %corrupt_path.display(), "invalid CDR spool segment archived for manual recovery");
        }
        self.metrics
            .spool_failures_total
            .fetch_add(count.max(1), Ordering::Relaxed);
    }

    fn complete_replay(&self, path: &Path, count: u64) -> std::io::Result<()> {
        std::fs::remove_file(path)?;
        self.metrics
            .replayed_total
            .fetch_add(count, Ordering::Relaxed);
        saturating_sub(&self.metrics.pending_spool_records, count);
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub(crate) struct DurableCdrSink {
    queue: tokio::sync::mpsc::Sender<CallCdr>,
    spool: CdrSpool,
}

impl DurableCdrSink {
    pub(crate) fn new(queue: tokio::sync::mpsc::Sender<CallCdr>, spool: CdrSpool) -> Self {
        Self { queue, spool }
    }
}

impl CdrSink for DurableCdrSink {
    fn try_send_cdr(&self, cdr: CallCdr) -> Result<(), CdrSendError> {
        match self.queue.try_send(cdr) {
            Ok(()) => Ok(()),
            Err(tokio::sync::mpsc::error::TrySendError::Full(cdr)) => {
                self.spool
                    .metrics
                    .queue_overflow_total
                    .fetch_add(1, Ordering::Relaxed);
                self.spool.append(&cdr).map_err(|error| {
                    self.spool
                        .metrics
                        .spool_failures_total
                        .fetch_add(1, Ordering::Relaxed);
                    tracing::error!(%error, call_id = cdr.call_id.as_str(), "CDR queue full and durable spool append failed");
                    CdrSendError::QueueFull
                })
            }
            Err(tokio::sync::mpsc::error::TrySendError::Closed(cdr)) => {
                self.spool.append(&cdr).map_err(|error| {
                    self.spool
                        .metrics
                        .spool_failures_total
                        .fetch_add(1, Ordering::Relaxed);
                    tracing::error!(%error, call_id = cdr.call_id.as_str(), "CDR consumer closed and durable spool append failed");
                    CdrSendError::ConsumerClosed
                })
            }
        }
    }
}

pub(crate) fn configured_spool_dir() -> PathBuf {
    std::env::var_os("VOS_RS_CDR_SPOOL_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("logs/cdr-spool"))
}

pub(crate) fn spawn_replay_loop(spool: CdrSpool, sinks: Arc<CdrSinks>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(REPLAY_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            spool.replay_once(&sinks).await;
        }
    });
}

fn open_append_file(path: &Path) -> std::io::Result<File> {
    OpenOptions::new().create(true).append(true).open(path)
}

fn read_spool_file(path: &Path) -> std::io::Result<Vec<CallCdr>> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut cdrs = Vec::new();
    let mut corrupt_file: Option<File> = None;

    for line_result in reader.lines() {
        let line = match line_result {
            Ok(line) => line,
            Err(e) => {
                tracing::error!("读取 spool 文件行失败: {}", e);
                break;
            }
        };

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        match serde_json::from_str::<CallCdr>(trimmed) {
            Ok(cdr) => {
                cdrs.push(cdr);
            }
            Err(e) => {
                tracing::warn!(
                    "解析 spool CDR 行失败，移入 .corrupt 文件: {}, 错误: {}",
                    trimmed,
                    e
                );
                let corrupt_path = path.with_extension("jsonl.corrupt");
                let c_file = match &mut corrupt_file {
                    Some(f) => f,
                    None => {
                        let f = OpenOptions::new()
                            .create(true)
                            .append(true)
                            .open(&corrupt_path)?;
                        corrupt_file = Some(f);
                        corrupt_file.as_mut().unwrap()
                    }
                };
                writeln!(c_file, "{}", trimmed)?;
            }
        }
    }

    Ok(cdrs)
}

fn count_pending_records(directory: &Path) -> std::io::Result<u64> {
    std::fs::read_dir(directory)?.try_fold(0_u64, |total, entry| {
        let path = entry?.path();
        let is_spool = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name == ACTIVE_SPOOL_FILE || name.ends_with(".jsonl"));
        if is_spool {
            Ok(total.saturating_add(count_lines(&path)?))
        } else {
            Ok(total)
        }
    })
}

fn count_lines(path: &Path) -> std::io::Result<u64> {
    Ok(BufReader::new(File::open(path)?).lines().count() as u64)
}

fn saturating_sub(value: &AtomicU64, amount: u64) {
    let _ = value.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
        Some(current.saturating_sub(amount))
    });
}

#[cfg(test)]
mod tests;
