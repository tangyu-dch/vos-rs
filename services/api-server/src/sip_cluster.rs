use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use futures::StreamExt;
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};

use crate::AppState;

#[derive(Debug, Deserialize)]
struct SipNodeRecord {
    node_id: String,
    advertised_addr: String,
    #[serde(default)]
    management_url: String,
    router_mode: String,
    #[serde(default = "default_node_status")]
    status: String,
    #[serde(default)]
    active_calls: usize,
    #[serde(default)]
    version: String,
    #[serde(default)]
    started_at: u64,
    updated_at: u64,
}

fn default_node_status() -> String {
    "active".to_string()
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub(crate) struct SipNodeStatus {
    node_id: String,
    advertised_addr: String,
    management_url: String,
    router_mode: String,
    status: String,
    active_calls: usize,
    version: String,
    started_at: u64,
    updated_at: u64,
    ttl_secs: i64,
}

#[derive(Debug, Serialize)]
pub(crate) struct SipClusterStatus {
    node_key_prefix: String,
    online_nodes: usize,
    active_nodes: usize,
    draining_nodes: usize,
    nodes: Vec<SipNodeStatus>,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct SipNodeActionResult {
    status: String,
    active_calls: usize,
}

/// 返回 Redis 心跳中仍在线的 SIP 信令节点。
pub(crate) async fn get_sip_cluster_status(State(state): State<AppState>) -> impl IntoResponse {
    match load_status(&state.redis_client, &state.sip_node_key_prefix).await {
        Ok(status) => (StatusCode::OK, Json(status)).into_response(),
        Err(error) => {
            tracing::error!(%error, "读取 SIP 集群状态失败");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(crate::ApiError::internal(format!(
                    "读取 SIP 集群状态失败: {error}"
                ))),
            )
                .into_response()
        }
    }
}

/// 将指定 SIP 节点切换为摘流或活动状态。
pub(crate) async fn control_sip_cluster_node(
    State(state): State<AppState>,
    Path((node_id, action)): Path<(String, String)>,
) -> impl IntoResponse {
    let action = match action_path(&action) {
        Ok(action) => action,
        Err(message) => return error_response(StatusCode::BAD_REQUEST, message),
    };
    let status = match load_status(&state.redis_client, &state.sip_node_key_prefix).await {
        Ok(status) => status,
        Err(error) => {
            return error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                format!("读取 SIP 集群状态失败: {error}"),
            );
        }
    };
    let Some(node) = status
        .nodes
        .into_iter()
        .find(|node| node.node_id == node_id)
    else {
        return error_response(StatusCode::NOT_FOUND, "SIP 节点不存在或心跳已过期");
    };
    match send_node_action(&state, &node.management_url, action).await {
        Ok(result) => (StatusCode::OK, Json(result)).into_response(),
        Err((status, message)) => error_response(status, message),
    }
}

fn action_path(action: &str) -> Result<&'static str, &'static str> {
    match action {
        "drain" => Ok("drain"),
        "resume" => Ok("resume"),
        _ => Err("仅支持 drain 或 resume 操作"),
    }
}

async fn send_node_action(
    state: &AppState,
    management_url: &str,
    action: &str,
) -> Result<SipNodeActionResult, (StatusCode, String)> {
    if state.internal_secret.is_empty() {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            "security.internal_secret 未配置".to_string(),
        ));
    }
    let endpoint = management_endpoint(management_url, action)
        .map_err(|message| (StatusCode::CONFLICT, message.to_string()))?;
    let response = state
        .internal_client
        .post(endpoint)
        .header("X-VOS-Token", &state.internal_secret)
        .send()
        .await
        .map_err(|error| {
            (
                StatusCode::BAD_GATEWAY,
                format!("连接 SIP 节点失败: {error}"),
            )
        })?;
    if !response.status().is_success() {
        return Err((
            StatusCode::BAD_GATEWAY,
            format!("SIP 节点拒绝操作: HTTP {}", response.status()),
        ));
    }
    response.json().await.map_err(|error| {
        (
            StatusCode::BAD_GATEWAY,
            format!("解析 SIP 节点响应失败: {error}"),
        )
    })
}

fn management_endpoint(base: &str, action: &str) -> Result<reqwest::Url, &'static str> {
    let mut url = reqwest::Url::parse(base).map_err(|_| "节点 management_url 无效")?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("节点 management_url 必须是无凭据的 HTTP(S) 地址");
    }
    url.set_path(&format!("/manage/cluster/{action}"));
    url.set_query(None);
    url.set_fragment(None);
    Ok(url)
}

fn error_response(status: StatusCode, message: impl Into<String>) -> axum::response::Response {
    (status, Json(crate::ApiError::internal(message))).into_response()
}

async fn load_status(
    client: &redis::Client,
    prefix: &str,
) -> Result<SipClusterStatus, Box<dyn std::error::Error + Send + Sync>> {
    let mut connection = client.get_multiplexed_tokio_connection().await?;
    let iterator = connection
        .scan_match::<_, String>(format!("{}:*", prefix.trim_end_matches(':')))
        .await?;
    let keys: Vec<String> = iterator.collect().await;
    let mut nodes = Vec::with_capacity(keys.len());
    for key in keys {
        let (payload, ttl): (Option<String>, i64) = redis::pipe()
            .get(&key)
            .ttl(&key)
            .query_async(&mut connection)
            .await?;
        if ttl <= 0 {
            continue;
        }
        let Some(record) = payload
            .as_deref()
            .and_then(|value| serde_json::from_str::<SipNodeRecord>(value).ok())
        else {
            tracing::warn!(%key, "忽略无效的 SIP 节点心跳记录");
            continue;
        };
        nodes.push(status_from_record(record, ttl));
    }
    nodes.sort_by(|left, right| left.node_id.cmp(&right.node_id));
    let active_nodes = nodes.iter().filter(|node| node.status == "active").count();
    let draining_nodes = nodes
        .iter()
        .filter(|node| node.status == "draining")
        .count();
    Ok(SipClusterStatus {
        node_key_prefix: prefix.to_string(),
        online_nodes: nodes.len(),
        active_nodes,
        draining_nodes,
        nodes,
    })
}

fn status_from_record(record: SipNodeRecord, ttl_secs: i64) -> SipNodeStatus {
    SipNodeStatus {
        node_id: record.node_id,
        advertised_addr: record.advertised_addr,
        management_url: record.management_url,
        router_mode: record.router_mode,
        status: record.status,
        active_calls: record.active_calls,
        version: record.version,
        started_at: record.started_at,
        updated_at: record.updated_at,
        ttl_secs,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_status_from_record_preserves_runtime_fields() {
        let status = status_from_record(
            SipNodeRecord {
                node_id: "sip-edge-a".to_string(),
                advertised_addr: "10.0.0.11:5060".to_string(),
                management_url: "http://10.0.0.11:8082".to_string(),
                router_mode: "native".to_string(),
                status: "active".to_string(),
                active_calls: 12,
                version: "0.1.0".to_string(),
                started_at: 1_719_999_000,
                updated_at: 1_720_000_000,
            },
            8,
        );

        assert_eq!(status.node_id, "sip-edge-a");
        assert_eq!(status.router_mode, "native");
        assert_eq!(status.status, "active");
        assert_eq!(status.active_calls, 12);
        assert_eq!(status.ttl_secs, 8);
    }

    #[test]
    fn test_management_endpoint_only_accepts_http_without_credentials() {
        let endpoint =
            management_endpoint("http://10.0.0.11:8082", "drain").expect("valid management URL");
        assert_eq!(
            endpoint.as_str(),
            "http://10.0.0.11:8082/manage/cluster/drain"
        );
        assert!(management_endpoint("file:///tmp/socket", "drain").is_err());
        assert!(management_endpoint("http://user:secret@10.0.0.11", "resume").is_err());
    }

    #[test]
    fn test_action_path_rejects_unknown_operation() {
        assert_eq!(action_path("drain"), Ok("drain"));
        assert_eq!(action_path("resume"), Ok("resume"));
        assert!(action_path("delete").is_err());
    }
}
