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
    processed_total: u64,
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
            processed_total: 0,
        });
    Json(CdrRuntimeMetrics {
        queue_overflow_total: snapshot.queue_overflow_total,
        spooled_total: snapshot.spooled_total,
        replayed_total: snapshot.replayed_total,
        spool_failures_total: snapshot.spool_failures_total,
        pending_spool_records: snapshot.pending_spool_records,
        unrecoverable_dropped_total: state.call_manager.dropped_cdr_count(),
        processed_total: snapshot.processed_total,
    })
}

/// TURN 中继状态：返回当前 allocation 的 relayed/mapped 地址与剩余 lifetime。
///
/// 未配置 TURN 服务器时返回 `enabled=false`。
pub(super) async fn turn_status(State(state): State<Arc<EdgeState>>) -> Json<serde_json::Value> {
    // 优先从 EdgeState 读取（与媒体路径共享同一实例）
    let turn_client = state.turn_client();
    if let Some(client) = turn_client {
        let allocation = client.allocation().await;
        let (relayed, mapped, lifetime) = match allocation {
            Some(a) => (
                Some(a.relayed_address.to_string()),
                Some(a.mapped_address.to_string()),
                Some(a.lifetime_secs),
            ),
            None => (None, None, None),
        };
        // 同时检查媒体路径是否已注入（用于诊断注入失败的场景）
        let media_injected = state.media_relay.turn_client().is_some();
        Json(serde_json::json!({
            "enabled": true,
            "relayed_address": relayed,
            "mapped_address": mapped,
            "lifetime_secs": lifetime,
            "media_path_injected": media_injected,
        }))
    } else {
        Json(serde_json::json!({
            "enabled": false,
        }))
    }
}
