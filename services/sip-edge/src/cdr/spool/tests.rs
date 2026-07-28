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

    crate::cdr::pipeline::flush_cdr_batch_with_retry_policy(
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
