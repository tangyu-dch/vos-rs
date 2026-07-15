//! SIP 与媒体集群的共享配置和运行时基础设施。

mod config;
mod inter_node;
mod node;
mod registration_sync;

pub(crate) use config::{
    ClusterConfig, ClusterConfigError, MediaAllocationStrategy, MediaClusterConfig,
    MediaNodeConfig, MediaNodeType, RouterMode,
};
pub(crate) use inter_node::{flow_key, start_inter_node_egress, ClusterEgress, FlowRecord};
pub(crate) use node::spawn_node_heartbeat;
pub(crate) use registration_sync::{
    start_registration_sync, RegistrationSyncCommand, RegistrationSyncSender,
};
