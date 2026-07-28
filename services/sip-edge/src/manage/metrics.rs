//! 运行时指标端点：RTP/录音聚合指标 + CDR 管线指标。

use axum::{extract::State, Json};
use serde::Serialize;
use std::sync::Arc;

use crate::media::relay::MediaRelayMetrics;
use crate::EdgeState;

/// RTP/录音聚合指标，供 API Server、压测脚本和运维面板读取。
pub(super) async fn media_metrics(State(state): State<Arc<EdgeState>>) -> Json<MediaRelayMetrics> {
    Json(state.media_relay.metrics_totals())
}

#[derive(Debug, Serialize)]
pub(super) struct CdrRuntimeMetrics {
    queue_overflow_total: u64,
    spooled_total: u64,
    replayed_total: u64,
    spool_failures_total: u64,
    pending_spool_records: u64,
    unrecoverable_dropped_total: u64,
}

pub(super) async fn cdr_metrics(State(state): State<Arc<EdgeState>>) -> Json<CdrRuntimeMetrics> {
    let snapshot = state
        .cdr_pipeline_metrics
        .get()
        .map(|metrics| metrics.snapshot())
        .unwrap_or(crate::cdr::CdrPipelineSnapshot {
            queue_overflow_total: 0,
            spooled_total: 0,
            replayed_total: 0,
            spool_failures_total: 0,
            pending_spool_records: 0,
        });
    Json(CdrRuntimeMetrics {
        queue_overflow_total: snapshot.queue_overflow_total,
        spooled_total: snapshot.spooled_total,
        replayed_total: snapshot.replayed_total,
        spool_failures_total: snapshot.spool_failures_total,
        pending_spool_records: snapshot.pending_spool_records,
        unrecoverable_dropped_total: state.call_manager.dropped_cdr_count(),
    })
}
