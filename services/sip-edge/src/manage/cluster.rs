//! 集群运维端点：摘流/恢复/状态查询。

use axum::{extract::State, Json};
use serde::Serialize;
use std::sync::{atomic::Ordering, Arc};

use crate::EdgeState;

use super::AdvertisedAddr;

#[derive(Debug, Serialize)]
pub(super) struct ClusterRuntimeStatus {
    status: &'static str,
    active_calls: usize,
    media_nodes_healthy: usize,
    media_nodes_total: usize,
    advertised_addr: String,
}

fn runtime_status(state: &EdgeState, advertised_addr: &str) -> ClusterRuntimeStatus {
    let (media_nodes_healthy, media_nodes_total) = state.media_relay.media_node_counts();
    ClusterRuntimeStatus {
        status: if state.draining.load(Ordering::Acquire) {
            "draining"
        } else {
            "active"
        },
        active_calls: state.call_manager.active_calls_count(),
        media_nodes_healthy,
        media_nodes_total,
        advertised_addr: advertised_addr.to_string(),
    }
}

pub(super) async fn cluster_status(
    State(edge): State<Arc<EdgeState>>,
    State(AdvertisedAddr(addr)): State<AdvertisedAddr>,
) -> Json<ClusterRuntimeStatus> {
    Json(runtime_status(&edge, &addr))
}

pub(super) async fn cluster_drain(
    State(edge): State<Arc<EdgeState>>,
    State(AdvertisedAddr(addr)): State<AdvertisedAddr>,
) -> Json<ClusterRuntimeStatus> {
    edge.draining.store(true, Ordering::Release);
    tracing::info!("SIP 节点已通过管理 API 进入摘流状态");
    Json(runtime_status(&edge, &addr))
}

pub(super) async fn cluster_resume(
    State(edge): State<Arc<EdgeState>>,
    State(AdvertisedAddr(addr)): State<AdvertisedAddr>,
) -> Json<ClusterRuntimeStatus> {
    edge.draining.store(false, Ordering::Release);
    tracing::info!("SIP 节点已通过管理 API 恢复接收新呼叫");
    Json(runtime_status(&edge, &addr))
}
