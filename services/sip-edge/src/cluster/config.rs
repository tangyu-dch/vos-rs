use std::collections::HashSet;

use serde::{Deserialize, Serialize};

const DEFAULT_HEARTBEAT_INTERVAL_SECS: u64 = 3;
const DEFAULT_NODE_TIMEOUT_SECS: u64 = 10;
const DEFAULT_DIALOG_TTL_SECS: u64 = 86_400;

/// SIP 集群入口模式。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouterMode {
    /// 单节点直连，兼容原有部署。
    #[default]
    Direct,
    /// 由 Kamailio/OpenSIPS 等外部 SIP 代理接入。
    External,
    /// 由项目内置的纯 Rust sip-router 接入。
    Native,
}

/// SIP 节点集群配置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ClusterConfig {
    /// 是否启用跨节点共享状态。
    pub enabled: bool,
    /// 当前 SIP 节点的稳定唯一标识。
    pub node_id: String,
    /// 集群入口模式。
    pub router_mode: RouterMode,
    /// 当前节点供路由器访问的 SIP 地址。
    pub advertised_addr: String,
    /// SIP 节点心跳在 Redis 中使用的键前缀。
    pub node_key_prefix: String,
    /// 节点心跳间隔。
    pub heartbeat_interval_secs: u64,
    /// 超过此时间未收到心跳即判定节点不可用。
    pub node_timeout_secs: u64,
    /// Redis 中对话快照的保留时间。
    pub dialog_ttl_secs: u64,
    /// 节点间 NATS 主题前缀。
    pub nats_subject_prefix: String,
}

impl Default for ClusterConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            node_id: "sip-edge-1".to_string(),
            router_mode: RouterMode::Direct,
            advertised_addr: "127.0.0.1:5060".to_string(),
            node_key_prefix: "vos_rs:cluster:sip_nodes".to_string(),
            heartbeat_interval_secs: DEFAULT_HEARTBEAT_INTERVAL_SECS,
            node_timeout_secs: DEFAULT_NODE_TIMEOUT_SECS,
            dialog_ttl_secs: DEFAULT_DIALOG_TTL_SECS,
            nats_subject_prefix: "vos_rs.sip.node".to_string(),
        }
    }
}

/// 媒体节点分配策略。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaAllocationStrategy {
    /// 按节点权重轮询。
    #[default]
    WeightedRoundRobin,
    /// 优先选择当前活跃会话最少的节点。
    LeastSessions,
    /// 根据 Call-ID 做稳定哈希。
    CallIdHash,
}

/// 媒体节点运行方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaNodeType {
    /// 由 sip-edge 进程内承载媒体。
    Local,
    /// 由独立 media-edge 进程承载媒体。
    Remote,
}

/// 单个媒体节点。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaNodeConfig {
    /// 媒体节点唯一标识。
    pub id: String,
    /// 节点运行方式。YAML 中使用 `type`。
    #[serde(rename = "type")]
    pub node_type: MediaNodeType,
    /// HTTP 或 Unix Domain Socket 控制地址；仅远程节点需要。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control_url: Option<String>,
    /// 写入 SDP 的媒体地址。
    pub advertised_addr: String,
    /// 节点 RTP 端口池起点。
    pub port_min: u16,
    /// 节点 RTP 端口池终点。
    pub port_max: u16,
    /// 节点调度权重，必须大于零。
    #[serde(default = "default_media_node_weight")]
    pub weight: u16,
    /// 控制平面鉴权令牌；空值表示未启用。
    #[serde(default)]
    pub control_token: String,
}

const fn default_media_node_weight() -> u16 {
    1
}

/// 多媒体节点池配置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MediaClusterConfig {
    /// 节点分配策略。
    pub allocation_strategy: MediaAllocationStrategy,
    /// 健康检查周期。
    pub health_check_interval_secs: u64,
    /// 连续失败多少次后摘除节点。
    pub unhealthy_threshold: u32,
    /// 可用媒体节点列表。
    pub nodes: Vec<MediaNodeConfig>,
}

impl Default for MediaClusterConfig {
    fn default() -> Self {
        Self {
            allocation_strategy: MediaAllocationStrategy::WeightedRoundRobin,
            health_check_interval_secs: 3,
            unhealthy_threshold: 3,
            nodes: Vec::new(),
        }
    }
}

/// 集群配置校验错误。
#[derive(Debug, PartialEq, Eq)]
pub enum ClusterConfigError {
    EmptyNodeId,
    InvalidNodeDiscovery,
    MissingSharedInfrastructure,
    InvalidHeartbeatTimeout,
    EmptyMediaNodes,
    InvalidMediaNode(String),
    MultipleLocalMediaNodes,
    DuplicateMediaNode(String),
    InvalidMediaPortRange(String),
    InvalidMediaWeight(String),
    OverlappingMediaPortRange { left: String, right: String },
}

impl std::fmt::Display for ClusterConfigError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyNodeId => write!(formatter, "启用 SIP 集群时 node_id 不能为空"),
            Self::InvalidNodeDiscovery => write!(
                formatter,
                "启用 SIP 集群时 advertised_addr 和 node_key_prefix 不能为空"
            ),
            Self::MissingSharedInfrastructure => write!(
                formatter,
                "启用 SIP 集群时必须配置 connections.redis 和 connections.nats"
            ),
            Self::InvalidHeartbeatTimeout => write!(
                formatter,
                "node_timeout_secs 必须大于 heartbeat_interval_secs"
            ),
            Self::EmptyMediaNodes => write!(formatter, "sip_edge.media.nodes 至少需要一个媒体节点"),
            Self::InvalidMediaNode(id) => {
                write!(formatter, "媒体节点 {id} 的配置无效")
            }
            Self::MultipleLocalMediaNodes => write!(formatter, "最多只能配置一个 local 媒体节点"),
            Self::DuplicateMediaNode(id) => write!(formatter, "媒体节点 id 重复: {id}"),
            Self::InvalidMediaPortRange(id) => {
                write!(formatter, "媒体节点 {id} 的 RTP 端口范围无效")
            }
            Self::InvalidMediaWeight(id) => {
                write!(formatter, "媒体节点 {id} 的 weight 必须大于零")
            }
            Self::OverlappingMediaPortRange { left, right } => {
                write!(formatter, "媒体节点 {left} 与 {right} 的 RTP 端口范围重叠")
            }
        }
    }
}

impl std::error::Error for ClusterConfigError {}

impl ClusterConfig {
    /// 校验 SIP 集群及媒体节点池配置。
    pub fn validate(
        &self,
        redis_url: Option<&str>,
        nats_url: Option<&str>,
        media: &MediaClusterConfig,
    ) -> Result<(), ClusterConfigError> {
        if self.enabled {
            if self.node_id.trim().is_empty() {
                return Err(ClusterConfigError::EmptyNodeId);
            }
            if self.advertised_addr.trim().is_empty() || self.node_key_prefix.trim().is_empty() {
                return Err(ClusterConfigError::InvalidNodeDiscovery);
            }
            if redis_url.is_none() || nats_url.is_none() {
                return Err(ClusterConfigError::MissingSharedInfrastructure);
            }
            if self.node_timeout_secs <= self.heartbeat_interval_secs {
                return Err(ClusterConfigError::InvalidHeartbeatTimeout);
            }
        }
        validate_media_nodes(&media.nodes)
    }
}

fn validate_media_nodes(nodes: &[MediaNodeConfig]) -> Result<(), ClusterConfigError> {
    if nodes.is_empty() {
        return Err(ClusterConfigError::EmptyMediaNodes);
    }
    let mut identifiers = HashSet::with_capacity(nodes.len());
    let mut local_nodes = 0_u8;
    for (index, node) in nodes.iter().enumerate() {
        if node.id.trim().is_empty() || node.advertised_addr.trim().is_empty() {
            return Err(ClusterConfigError::InvalidMediaNode(node.id.clone()));
        }
        match node.node_type {
            MediaNodeType::Local => {
                local_nodes = local_nodes.saturating_add(1);
                if local_nodes > 1 {
                    return Err(ClusterConfigError::MultipleLocalMediaNodes);
                }
                if node
                    .control_url
                    .as_deref()
                    .is_some_and(|url| !url.trim().is_empty())
                {
                    return Err(ClusterConfigError::InvalidMediaNode(node.id.clone()));
                }
            }
            MediaNodeType::Remote => {
                let valid_url = node.control_url.as_deref().is_some_and(|url| {
                    url.starts_with("http://")
                        || url.starts_with("https://")
                        || url.starts_with("uds://")
                });
                if !valid_url {
                    return Err(ClusterConfigError::InvalidMediaNode(node.id.clone()));
                }
            }
        }
        if !identifiers.insert(node.id.as_str()) {
            return Err(ClusterConfigError::DuplicateMediaNode(node.id.clone()));
        }
        if node.port_min < 1024
            || node.port_min % 2 != 0
            || node.port_max % 2 != 0
            || node.port_max <= node.port_min
        {
            return Err(ClusterConfigError::InvalidMediaPortRange(node.id.clone()));
        }
        if node.weight == 0 {
            return Err(ClusterConfigError::InvalidMediaWeight(node.id.clone()));
        }
        for other in &nodes[..index] {
            if node.port_min <= other.port_max && other.port_min <= node.port_max {
                return Err(ClusterConfigError::OverlappingMediaPortRange {
                    left: other.id.clone(),
                    right: node.id.clone(),
                });
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn media_node(id: &str, port_min: u16, port_max: u16) -> MediaNodeConfig {
        MediaNodeConfig {
            id: id.to_string(),
            node_type: MediaNodeType::Remote,
            control_url: Some(format!("http://{id}:3030")),
            advertised_addr: "203.0.113.10".to_string(),
            port_min,
            port_max,
            weight: 1,
            control_token: String::new(),
        }
    }

    #[test]
    fn test_validate_cluster_with_shared_infrastructure_succeeds() {
        let cluster = ClusterConfig {
            enabled: true,
            ..ClusterConfig::default()
        };
        let media = MediaClusterConfig {
            nodes: vec![media_node("media-a", 40_000, 40_100)],
            ..MediaClusterConfig::default()
        };

        assert_eq!(
            cluster.validate(Some("redis://redis"), Some("nats://nats"), &media),
            Ok(())
        );
    }

    #[test]
    fn test_validate_media_nodes_with_overlapping_ranges_fails() {
        let media = MediaClusterConfig {
            nodes: vec![
                media_node("media-a", 40_000, 40_100),
                media_node("media-b", 40_100, 40_200),
            ],
            ..MediaClusterConfig::default()
        };

        assert!(matches!(
            ClusterConfig::default().validate(None, None, &media),
            Err(ClusterConfigError::OverlappingMediaPortRange { .. })
        ));
    }

    #[test]
    fn test_validate_empty_media_nodes_fails() {
        assert_eq!(
            ClusterConfig::default().validate(None, None, &MediaClusterConfig::default()),
            Err(ClusterConfigError::EmptyMediaNodes)
        );
    }

    #[test]
    fn test_validate_local_node_without_control_url_succeeds() {
        let media = MediaClusterConfig {
            nodes: vec![MediaNodeConfig {
                id: "local-media".to_string(),
                node_type: MediaNodeType::Local,
                control_url: None,
                advertised_addr: "127.0.0.1".to_string(),
                port_min: 40_000,
                port_max: 40_100,
                weight: 1,
                control_token: String::new(),
            }],
            ..MediaClusterConfig::default()
        };
        assert_eq!(
            ClusterConfig::default().validate(None, None, &media),
            Ok(())
        );
    }
}
