use crate::cdr::flush_cdr_batch;
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
mod tests {
    use super::*;
    use call_core::{CallId, CdrAuditSnapshot, CdrStatus};
    use std::time::SystemTime;

    fn test_cdr(call_id: &str) -> CallCdr {
        CallCdr {
            call_id: CallId::new(call_id),
            caller: Some("1001".to_string()),
            callee: Some("1002".to_string()),
            started_at: SystemTime::UNIX_EPOCH,
            answered_at: None,
            ended_at: SystemTime::UNIX_EPOCH,
            duration: Duration::ZERO,
            billable_duration: Duration::ZERO,
            status: CdrStatus::Canceled,
            failure_cause: None,
            caller_rtcp_loss_rate: None,
            caller_rtcp_jitter_ms: None,
            caller_rtcp_rtt_ms: None,
            gateway_rtcp_loss_rate: None,
            gateway_rtcp_jitter_ms: None,
            gateway_rtcp_rtt_ms: None,
            mos: None,
            dtmf_digits: None,
            recording_path: None,
            direction: "outbound".to_string(),
            audit: CdrAuditSnapshot::default(),
        }
    }

    fn temp_spool_dir() -> PathBuf {
        std::env::temp_dir().join(format!("vos-rs-cdr-spool-{}", uuid::Uuid::new_v4()))
    }

    #[test]
    fn queue_overflow_is_durably_spooled() {
        let directory = temp_spool_dir();
        let spool = CdrSpool::open(directory.clone()).expect("open spool");
        let (sender, _receiver) = tokio::sync::mpsc::channel(1);
        sender.try_send(test_cdr("first")).expect("fill queue");
        let sink = DurableCdrSink::new(sender, spool.clone());

        sink.try_send_cdr(test_cdr("overflow"))
            .expect("overflow must be spooled");

        let snapshot = spool.metrics().snapshot();
        assert_eq!(snapshot.queue_overflow_total, 1);
        assert_eq!(snapshot.spooled_total, 1);
        assert_eq!(snapshot.pending_spool_records, 1);
        let records = read_spool_file(&directory.join(ACTIVE_SPOOL_FILE)).expect("read spool");
        assert_eq!(records[0].call_id.as_str(), "overflow");
        std::fs::remove_dir_all(directory).expect("remove temp spool");
    }

    #[test]
    fn pending_records_are_recovered_after_restart() {
        let directory = temp_spool_dir();
        {
            let spool = CdrSpool::open(directory.clone()).expect("open spool");
            spool.append(&test_cdr("persisted")).expect("append CDR");
        }

        let reopened = CdrSpool::open(directory.clone()).expect("reopen spool");
        assert_eq!(reopened.metrics().snapshot().pending_spool_records, 1);
        std::fs::remove_dir_all(directory).expect("remove temp spool");
    }

    #[test]
    fn rotation_preserves_records_and_keeps_accepting_new_cdrs() {
        let directory = temp_spool_dir();
        let spool = CdrSpool::open(directory.clone()).expect("open spool");
        spool
            .append(&test_cdr("before-rotate"))
            .expect("append CDR");

        let replay_path = spool
            .rotate_active()
            .expect("rotate spool")
            .expect("non-empty segment");
        spool.append(&test_cdr("after-rotate")).expect("append CDR");

        let replay_records = read_spool_file(&replay_path).expect("read replay segment");
        let active_records =
            read_spool_file(&directory.join(ACTIVE_SPOOL_FILE)).expect("read active segment");
        assert_eq!(replay_records[0].call_id.as_str(), "before-rotate");
        assert_eq!(active_records[0].call_id.as_str(), "after-rotate");
        assert_eq!(spool.metrics().snapshot().pending_spool_records, 2);
        std::fs::remove_dir_all(directory).expect("remove temp spool");
    }

    #[tokio::test]
    async fn unavailable_persistence_sink_is_saved_to_spool() {
        let directory = temp_spool_dir();
        let spool = CdrSpool::open(directory.clone()).expect("open spool");
        let cdrs = vec![test_cdr("db-unavailable")];

        crate::cdr::flush_cdr_batch_with_retry_policy(
            &CdrSinks::default(),
            &spool,
            &cdrs,
            1,
            Duration::ZERO,
        )
        .await;

        let records = read_spool_file(&directory.join(ACTIVE_SPOOL_FILE)).expect("read spool");
        assert_eq!(records[0].call_id.as_str(), "db-unavailable");
        assert_eq!(spool.metrics().snapshot().pending_spool_records, 1);
        std::fs::remove_dir_all(directory).expect("remove temp spool");
    }

    #[test]
    fn successful_replay_removes_segment_and_updates_metrics() {
        let directory = temp_spool_dir();
        let spool = CdrSpool::open(directory.clone()).expect("open spool");
        spool.append(&test_cdr("replayed")).expect("append CDR");
        let replay_path = spool
            .rotate_active()
            .expect("rotate spool")
            .expect("non-empty segment");

        spool
            .complete_replay(&replay_path, 1)
            .expect("complete replay");

        assert!(!replay_path.exists());
        let snapshot = spool.metrics().snapshot();
        assert_eq!(snapshot.replayed_total, 1);
        assert_eq!(snapshot.pending_spool_records, 0);
        std::fs::remove_dir_all(directory).expect("remove temp spool");
    }

    #[test]
    fn corrupt_segment_is_quarantined_without_blocking_active_spool() {
        let directory = temp_spool_dir();
        std::fs::create_dir_all(&directory).expect("create spool directory");
        let replay_path = directory.join("replay-corrupt.jsonl");
        std::fs::write(&replay_path, b"not-json\n").expect("write corrupt segment");

        let cdrs = read_spool_file(&replay_path).expect("read spool file");
        assert!(cdrs.is_empty());
        assert!(replay_path.with_extension("jsonl.corrupt").exists());

        let corrupt_content =
            std::fs::read_to_string(replay_path.with_extension("jsonl.corrupt")).unwrap();
        assert_eq!(corrupt_content, "not-json\n");

        std::fs::remove_dir_all(directory).expect("remove temp spool");
    }

    #[test]
    fn mixed_segment_keeps_valid_cdrs_and_isolates_corrupt_lines() {
        let directory = temp_spool_dir();
        std::fs::create_dir_all(&directory).expect("create spool directory");
        let replay_path = directory.join("replay-mixed.jsonl");

        let valid_cdr_1 = test_cdr("valid-1");
        let valid_json_1 = serde_json::to_string(&valid_cdr_1).unwrap();
        let valid_cdr_2 = test_cdr("valid-2");
        let valid_json_2 = serde_json::to_string(&valid_cdr_2).unwrap();

        let file_content = format!("{}\nnot-json-line\n{}\n", valid_json_1, valid_json_2);
        std::fs::write(&replay_path, file_content.as_bytes()).expect("write mixed segment");

        let cdrs = read_spool_file(&replay_path).expect("read spool file");
        assert_eq!(cdrs.len(), 2);
        assert_eq!(cdrs[0].call_id.as_str(), "valid-1");
        assert_eq!(cdrs[1].call_id.as_str(), "valid-2");

        let corrupt_path = replay_path.with_extension("jsonl.corrupt");
        assert!(corrupt_path.exists());
        let corrupt_content = std::fs::read_to_string(corrupt_path).unwrap();
        assert_eq!(corrupt_content, "not-json-line\n");

        std::fs::remove_dir_all(directory).expect("remove temp spool");
    }
}
