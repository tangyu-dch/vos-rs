use axum::{
    extract::State,
    response::sse::{Event, KeepAlive, Sse},
    Json,
};
use cdr_core::{DashboardStats, HourlyTrend};
use futures::stream::{self, Stream};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

use crate::{ApiError, AppState};

/// 活跃通话列表缓存：2 秒 TTL，避免 summary / monitoring-extras / telemetry loop
/// 在同一时刻重复请求 sip-edge 的 /manage/active-calls 全量接口。
type CachedActiveCalls = Option<(Instant, Vec<serde_json::Value>)>;

#[derive(Clone)]
pub struct ActiveCallsCache {
    inner: Arc<Mutex<CachedActiveCalls>>,
    ttl: Duration,
}

impl ActiveCallsCache {
    pub fn new(ttl: Duration) -> Self {
        Self {
            inner: Arc::new(Mutex::new(None)),
            ttl,
        }
    }

    /// 返回缓存中的活跃通话列表；若已过期则重新拉取。拉取失败时返回空 Vec。
    pub async fn get_or_fetch(&self, state: &AppState) -> Vec<serde_json::Value> {
        let now = Instant::now();
        {
            let guard = self.inner.lock().await;
            if let Some((fetched_at, data)) = guard.as_ref() {
                if now.duration_since(*fetched_at) < self.ttl {
                    return data.clone();
                }
            }
        }
        let data = fetch_active_calls_full(state).await;
        let mut guard = self.inner.lock().await;
        *guard = Some((now, data.clone()));
        data
    }
}

#[derive(Debug, serde::Serialize)]
pub struct HourlyTrendResponse {
    pub hour: String,
    pub total_calls: i64,
    pub answered_calls: i64,
}

#[derive(Debug, serde::Serialize)]
pub struct SummaryResponse {
    #[serde(flatten)]
    pub stats: DashboardStats,
    pub hourly_trends: Vec<HourlyTrendResponse>,
}

/// 拉取活跃通话列表（全量 JSON），供需要逐条过滤的场景使用（monitoring-extras / telemetry）。
async fn fetch_active_calls_full(state: &AppState) -> Vec<serde_json::Value> {
    let fut = async {
        let url = format!("{}/manage/active-calls", state.sip_manage_base);
        let token = &state.internal_secret;
        let request = state.internal_client.get(&url);
        let request = if !token.is_empty() {
            request.header("X-VOS-Token", token)
        } else {
            request
        };
        match request.send().await {
            Ok(resp) => resp.json::<Vec<serde_json::Value>>().await.unwrap_or_default(),
            Err(_) => Vec::new(),
        }
    };
    tokio::time::timeout(Duration::from_millis(200), fut)
        .await
        .unwrap_or_default()
}

/// 轻量拉取活跃通话数量（仅返回一个 usize），summary 接口只需要计数。
async fn fetch_active_calls_count(state: &AppState) -> i64 {
    let fut = async {
        let url = format!("{}/manage/active-calls/count", state.sip_manage_base);
        let token = &state.internal_secret;
        let request = state.internal_client.get(&url);
        let request = if !token.is_empty() {
            request.header("X-VOS-Token", token)
        } else {
            request
        };
        match request.send().await {
            Ok(resp) => resp.json::<usize>().await.map(|n| n as i64).unwrap_or(0),
            Err(_) => 0,
        }
    };
    tokio::time::timeout(Duration::from_millis(200), fut)
        .await
        .unwrap_or(0)
}

pub async fn get_dashboard_stats(
    State(state): State<AppState>,
) -> Result<Json<SummaryResponse>, ApiError> {
    // 并行：HTTP 取活跃通话数（轻量 count 接口） + DB 取 24h 趋势
    // 二者无数据依赖，串行等待会叠加延迟。
    let (active_calls, trends) = tokio::join!(
        fetch_active_calls_count(&state),
        state.store.get_hourly_trend(),
    );
    let trends = trends.unwrap_or_default();

    let stats = state.store.get_dashboard_stats(active_calls).await.map_err(|e| ApiError {
        error: e.to_string(),
    })?;

    let now = time::OffsetDateTime::now_utc()
        .to_offset(time::UtcOffset::from_hms(8, 0, 0).unwrap_or(time::UtcOffset::UTC));
    let current_hour = now.hour() as i32;
    let mut hourly_trends = Vec::new();
    for i in (0..24).rev() {
        let h = (current_hour - i + 24) % 24;
        let hour_str = format!("{:02}:00", h);
        let db_trend = trends.iter().find(|t| t.hour == h);
        let total_calls = db_trend.map(|t| t.total).unwrap_or(0);
        let answered_calls = db_trend.map(|t| t.answered).unwrap_or(0);
        hourly_trends.push(HourlyTrendResponse {
            hour: hour_str,
            total_calls,
            answered_calls,
        });
    }

    Ok(Json(SummaryResponse { stats, hourly_trends }))
}

pub async fn get_dashboard_trend(
    State(state): State<AppState>,
) -> Result<Json<Vec<HourlyTrend>>, ApiError> {
    state
        .store
        .get_hourly_trend()
        .await
        .map(Json)
        .map_err(|e| ApiError {
            error: e.to_string(),
        })
}

pub async fn dashboard_events(
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>> {
    let state_clone = state.clone();
    let stream = stream::unfold(
        (
            state_clone,
            tokio::time::interval(std::time::Duration::from_secs(2)),
        ),
        |(state, mut interval)| async move {
            interval.tick().await;

            let active_calls = state.active_calls_cache.get_or_fetch(&state).await.len() as u32;

            let trunk_online_count = match state.store.list_gateways_full().await {
                Ok(gateways) => gateways
                    .iter()
                    .filter(|gateway| gateway.enabled != Some(false))
                    .filter(|gateway| gateway.circuit_state.as_deref() != Some("open"))
                    .count() as u32,
                Err(_) => 0,
            };

            let data = serde_json::json!({
                "active_calls": active_calls,
                "trunk_online_count": trunk_online_count,
                "timestamp": time::OffsetDateTime::now_utc().unix_timestamp(),
            });

            let event = Event::default().data(data.to_string());
            Some((Ok(event), (state, interval)))
        },
    );

    Sse::new(stream).keep_alive(KeepAlive::default())
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct NodeTrafficItem {
    pub hour: String,
    pub kbps: u64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct NodeTrafficData {
    pub node_id: String,
    pub node_type: String, // "sip" or "media"
    pub series: Vec<NodeTrafficItem>,
}

pub async fn get_node_traffic(
    State(state): State<AppState>,
) -> Result<Json<Vec<NodeTrafficData>>, ApiError> {
    let sip_nodes = crate::sip_cluster::get_node_list(&state).await;
    let media_nodes = crate::media_cluster::get_node_list(&state).await;

    let now = time::OffsetDateTime::now_utc()
        .to_offset(time::UtcOffset::from_hms(8, 0, 0).unwrap_or(time::UtcOffset::UTC));
    let current_hour = now.hour() as i32;
    let mut hours = Vec::new();
    for i in (0..24).rev() {
        let h = (current_hour - i + 24) % 24;
        hours.push(h);
    }

    let hour_strs: Vec<String> = hours.iter().map(|h| format!("{:02}:00", h)).collect();

    // 收集所有节点（SIP + Media），用 Redis pipeline 一次性查询，避免 N 次串行 RTT
    let all_nodes: Vec<(String, String)> = sip_nodes
        .into_iter().map(|id| (id, "sip".to_string()))
        .chain(media_nodes.into_iter().map(|id| (id, "media".to_string())))
        .collect();

    let mut result = Vec::with_capacity(all_nodes.len());
    if !all_nodes.is_empty() {
        let mut conn = state.redis_client.clone();
        // 构建 pipeline：对每个节点 HGETALL，一次网络往返取回全部
        let mut pipeline = redis::pipe();
        for (node_id, _) in &all_nodes {
            let redis_key = format!("vos_rs:traffic:{}", node_id);
            pipeline.cmd("HGETALL").arg(&redis_key);
        }
        let pipe_result: Vec<std::collections::HashMap<String, u64>> = pipeline
            .query_async(&mut conn)
            .await
            .unwrap_or_default();

        for ((node_id, node_type), traffic_map) in all_nodes.iter().zip(pipe_result.into_iter()) {
            let series: Vec<NodeTrafficItem> = hour_strs
                .iter()
                .map(|hs| NodeTrafficItem {
                    hour: hs.clone(),
                    kbps: traffic_map.get(hs).cloned().unwrap_or(0),
                })
                .collect();
            result.push(NodeTrafficData {
                node_id: node_id.clone(),
                node_type: node_type.clone(),
                series,
            });
        }
    }

    Ok(Json(result))
}

pub async fn start_traffic_telemetry_loop(state: AppState) {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
    loop {
        interval.tick().await;

        // 1. Get active calls count (使用缓存，避免与 summary/extras 重复请求)
        let active_calls = state.active_calls_cache.get_or_fetch(&state).await.len() as u64;

        // 2. Get online nodes list
        let sip_nodes = crate::sip_cluster::get_node_list(&state).await;
        let media_nodes = crate::media_cluster::get_node_list(&state).await;

        let now = time::OffsetDateTime::now_utc()
            .to_offset(time::UtcOffset::from_hms(8, 0, 0).unwrap_or(time::UtcOffset::UTC));
        let hour_str = format!("{:02}:00", now.hour());

        let mut conn = state.redis_client.clone();

        // 3. Update SIP Nodes traffic in Redis
        // 仅在有真实活跃通话时写入流量；无通话时写入 0，避免图表出现伪基线曲线
        let sip_count = sip_nodes.len() as u64;
        for node_id in sip_nodes {
            let node_calls = if sip_count > 0 { active_calls / sip_count } else { 0 };
            let kbps = if active_calls > 0 { node_calls * 8 } else { 0 };
            let redis_key = format!("vos_rs:traffic:{}", node_id);
            let _: Result<(), redis::RedisError> = redis::cmd("HSET")
                .arg(&redis_key)
                .arg(&hour_str)
                .arg(kbps)
                .query_async(&mut conn)
                .await;
        }

        // 4. Update Media Nodes traffic in Redis
        let media_count = media_nodes.len() as u64;
        for node_id in media_nodes {
            let node_calls = if media_count > 0 { active_calls / media_count } else { 0 };
            let kbps = if active_calls > 0 { node_calls * 160 } else { 0 };
            let redis_key = format!("vos_rs:traffic:{}", node_id);
            let _: Result<(), redis::RedisError> = redis::cmd("HSET")
                .arg(&redis_key)
                .arg(&hour_str)
                .arg(kbps)
                .query_async(&mut conn)
                .await;
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct GatewayConcurrency {
    pub name: String,
    pub direction: String, // "access" or "egress"
    pub active_calls: u64,
    pub max_channels: u64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SbcSecurityStats {
    pub blocked_calls_24h: u64,
    pub auth_failures_24h: u64,
    pub error_codes_breakdown: std::collections::HashMap<String, u64>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SystemResourceStats {
    pub cpu_percent: f64,
    pub memory_percent: f64,
    pub disk_percent: f64,
    pub db_pool_active: u32,
    pub db_pool_max: u32,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct MonitoringExtras {
    pub gateways: Vec<GatewayConcurrency>,
    pub security: SbcSecurityStats,
    pub resources: SystemResourceStats,
}

pub async fn get_monitoring_extras(
    State(state): State<AppState>,
) -> Result<Json<MonitoringExtras>, ApiError> {
    let (active_calls_list, gateways_full_res, sec_res) = tokio::join!(
        state.active_calls_cache.get_or_fetch(&state),
        state.store.list_gateways_full(),
        state.store.get_security_and_errors_24h(),
    );

    let gateways_full = gateways_full_res.unwrap_or_default();
    let (blocked_calls_24h, auth_failures_24h, error_codes_breakdown) =
        sec_res.unwrap_or((0, 0, std::collections::HashMap::new()));

    let mut gateways = Vec::new();
    for gw in gateways_full {
        let active_calls = active_calls_list
            .iter()
            .filter(|call| {
                call.get("gateway")
                    .and_then(|g| g.as_str())
                    .map(|id| id == gw.id)
                    .unwrap_or(false)
            })
            .count() as u64;

        gateways.push(GatewayConcurrency {
            name: gw.id,
            direction: gw.role.unwrap_or_else(|| "egress".to_string()),
            active_calls,
            max_channels: gw.max_capacity.unwrap_or(0) as u64,
        });
    }

    let pool_size = state.store.pool().size();
    let pool_max = 50;

    // CPU/内存基于活跃通话数估算（仅作运维参考，非真实采集）
    let active_calls_count = active_calls_list.len() as f64;
    let cpu_percent = (active_calls_count * 0.45).min(98.0);
    let memory_percent = (active_calls_count * 0.08).min(95.0);
    let disk_percent = 0.0;

    let resources = SystemResourceStats {
        cpu_percent,
        memory_percent,
        disk_percent,
        db_pool_active: pool_size,
        db_pool_max: pool_max,
    };

    Ok(Json(MonitoringExtras {
        gateways,
        security: SbcSecurityStats {
            blocked_calls_24h,
            auth_failures_24h,
            error_codes_breakdown,
        },
        resources,
    }))
}
