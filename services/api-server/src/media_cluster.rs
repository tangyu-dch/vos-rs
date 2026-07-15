use std::collections::{HashMap, HashSet};

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use serde::{Deserialize, Serialize};
use sqlx::Row;

use crate::AppState;

const CONFIG_KEY: &str = "media_cluster_json";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct MediaClusterConfigPayload {
    pub allocation_strategy: String,
    pub health_check_interval_secs: u64,
    pub unhealthy_threshold: u32,
    pub nodes: Vec<MediaNodePayload>,
}

impl Default for MediaClusterConfigPayload {
    fn default() -> Self {
        Self {
            allocation_strategy: "weighted_round_robin".to_string(),
            health_check_interval_secs: 3,
            unhealthy_threshold: 3,
            nodes: vec![MediaNodePayload {
                id: "local-media".to_string(),
                node_type: "local".to_string(),
                control_url: None,
                advertised_addr: "127.0.0.1".to_string(),
                port_min: 40_000,
                port_max: 40_100,
                weight: 1,
                control_token: None,
                control_token_configured: false,
            }],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct MediaNodePayload {
    pub id: String,
    #[serde(rename = "type")]
    pub node_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control_url: Option<String>,
    pub advertised_addr: String,
    pub port_min: u16,
    pub port_max: u16,
    pub weight: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control_token: Option<String>,
    #[serde(default)]
    pub control_token_configured: bool,
}

pub(crate) async fn get_media_cluster(State(state): State<AppState>) -> impl IntoResponse {
    match load_stored(state.store.pool()).await {
        Ok(mut config) => {
            mask_tokens(&mut config);
            (StatusCode::OK, Json(config)).into_response()
        }
        Err(error) => {
            tracing::error!(%error, "读取媒体集群配置失败");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

pub(crate) async fn update_media_cluster(
    State(state): State<AppState>,
    Json(mut payload): Json<MediaClusterConfigPayload>,
) -> impl IntoResponse {
    if let Err(message) = validate(&payload) {
        return (StatusCode::BAD_REQUEST, message).into_response();
    }
    let existing = match load_stored(state.store.pool()).await {
        Ok(config) => config,
        Err(error) => {
            tracing::error!(%error, "读取原媒体集群配置失败");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    preserve_tokens(&mut payload, &existing);
    let serialized = match serde_json::to_string(&payload) {
        Ok(value) => value,
        Err(error) => {
            tracing::error!(%error, "序列化媒体集群配置失败");
            return StatusCode::BAD_REQUEST.into_response();
        }
    };
    if let Err(error) = sqlx::query(
        "INSERT INTO system_configs (config_key, config_value) VALUES ($1, $2) \
         ON CONFLICT (config_key) DO UPDATE SET config_value = EXCLUDED.config_value",
    )
    .bind(CONFIG_KEY)
    .bind(&serialized)
    .execute(state.store.pool())
    .await
    {
        tracing::error!(%error, "保存媒体集群配置失败");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    if let Ok(mut redis) = state.redis_client.get_multiplexed_tokio_connection().await {
        let _: Result<(), redis::RedisError> = redis::cmd("HSET")
            .arg("vos_rs:system_configs")
            .arg(CONFIG_KEY)
            .arg(&serialized)
            .query_async(&mut redis)
            .await;
    }
    mask_tokens(&mut payload);
    (StatusCode::OK, Json(payload)).into_response()
}

async fn load_stored(
    pool: &sqlx::PgPool,
) -> Result<MediaClusterConfigPayload, Box<dyn std::error::Error + Send + Sync>> {
    let row = sqlx::query("SELECT config_value FROM system_configs WHERE config_key = $1")
        .bind(CONFIG_KEY)
        .fetch_optional(pool)
        .await?;
    let Some(row) = row else {
        return Ok(MediaClusterConfigPayload::default());
    };
    let value: String = row.get("config_value");
    Ok(serde_json::from_str(&value)?)
}

fn preserve_tokens(payload: &mut MediaClusterConfigPayload, existing: &MediaClusterConfigPayload) {
    let tokens: HashMap<&str, &str> = existing
        .nodes
        .iter()
        .filter_map(|node| {
            node.control_token
                .as_deref()
                .map(|token| (node.id.as_str(), token))
        })
        .collect();
    for node in &mut payload.nodes {
        if node.control_token.is_none() {
            node.control_token = tokens
                .get(node.id.as_str())
                .map(|value| (*value).to_string());
        }
        node.control_token_configured = false;
    }
}

fn mask_tokens(config: &mut MediaClusterConfigPayload) {
    for node in &mut config.nodes {
        node.control_token_configured = node
            .control_token
            .as_deref()
            .is_some_and(|token| !token.is_empty());
        node.control_token = None;
    }
}

fn validate(config: &MediaClusterConfigPayload) -> Result<(), &'static str> {
    if !matches!(
        config.allocation_strategy.as_str(),
        "weighted_round_robin" | "least_sessions" | "call_id_hash"
    ) {
        return Err("不支持的媒体节点分配策略");
    }
    if config.health_check_interval_secs == 0 || config.unhealthy_threshold == 0 {
        return Err("健康检查周期和失败阈值必须大于零");
    }
    if config.nodes.is_empty() {
        return Err("至少需要配置一个媒体节点");
    }
    let mut identifiers = HashSet::new();
    let mut local_nodes = 0_u8;
    for (index, node) in config.nodes.iter().enumerate() {
        if node.id.trim().is_empty() || node.advertised_addr.trim().is_empty() {
            return Err("媒体节点标识和通告地址不能为空");
        }
        if !identifiers.insert(node.id.as_str()) {
            return Err("媒体节点标识不能重复");
        }
        match node.node_type.as_str() {
            "local" => {
                local_nodes = local_nodes.saturating_add(1);
                if local_nodes > 1 {
                    return Err("最多只能配置一个本地媒体节点");
                }
                if node
                    .control_url
                    .as_deref()
                    .is_some_and(|url| !url.trim().is_empty())
                {
                    return Err("本地媒体节点不能配置控制地址");
                }
            }
            "remote" => {
                let valid_url = node.control_url.as_deref().is_some_and(|url| {
                    url.starts_with("http://")
                        || url.starts_with("https://")
                        || url.starts_with("uds://")
                });
                if !valid_url {
                    return Err("远程媒体节点控制地址必须使用 http、https 或 uds");
                }
            }
            _ => return Err("媒体节点类型只能是 local 或 remote"),
        }
        if node.port_min < 1024
            || node.port_min % 2 != 0
            || node.port_max % 2 != 0
            || node.port_max <= node.port_min
            || node.weight == 0
        {
            return Err("媒体节点端口范围或权重无效");
        }
        for other in &config.nodes[..index] {
            if node.port_min <= other.port_max && other.port_min <= node.port_max {
                return Err("媒体节点 RTP 端口范围不能重叠");
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str, port_min: u16) -> MediaNodePayload {
        MediaNodePayload {
            id: id.to_string(),
            node_type: "remote".to_string(),
            control_url: Some(format!("http://{id}:3030")),
            advertised_addr: "203.0.113.10".to_string(),
            port_min,
            port_max: port_min + 98,
            weight: 1,
            control_token: None,
            control_token_configured: false,
        }
    }

    #[test]
    fn test_validate_rejects_overlapping_media_ranges() {
        let config = MediaClusterConfigPayload {
            nodes: vec![node("a", 40_000), node("b", 40_098)],
            ..MediaClusterConfigPayload::default()
        };
        assert!(validate(&config).is_err());
    }

    #[test]
    fn test_mask_tokens_never_returns_secret() {
        let mut config = MediaClusterConfigPayload {
            nodes: vec![MediaNodePayload {
                control_token: Some("secret".to_string()),
                ..node("a", 40_000)
            }],
            ..MediaClusterConfigPayload::default()
        };
        mask_tokens(&mut config);
        assert!(config.nodes[0].control_token.is_none());
        assert!(config.nodes[0].control_token_configured);
    }

    #[test]
    fn test_validate_rejects_empty_media_nodes() {
        let config = MediaClusterConfigPayload {
            nodes: Vec::new(),
            ..MediaClusterConfigPayload::default()
        };
        assert!(validate(&config).is_err());
    }

    #[test]
    fn test_validate_accepts_local_node_without_control_url() {
        let mut local = node("local-media", 40_000);
        local.node_type = "local".to_string();
        local.control_url = None;
        let config = MediaClusterConfigPayload {
            nodes: vec![local],
            ..MediaClusterConfigPayload::default()
        };
        assert!(validate(&config).is_ok());
    }
}
