use serde::Deserialize;
use std::env;

#[derive(Deserialize, Debug, Default)]
pub(crate) struct ApiServerConfig {
    pub(crate) connections: Option<ConnectionsSection>,
    pub(crate) api_server: Option<ApiServerSection>,
    pub(crate) sip_edge: Option<SipEdgeConfigSection>,
}

#[derive(Deserialize, Debug, Default, Clone)]
pub(crate) struct ConnectionsSection {
    pub(crate) database: Option<DatabaseSection>,
    pub(crate) redis: Option<RedisSection>,
    pub(crate) nats: Option<NatsSection>,
}

#[derive(Deserialize, Debug, Default, Clone)]
pub(crate) struct RedisSection {
    pub(crate) host: Option<String>,
    pub(crate) port: Option<u16>,
    pub(crate) password: Option<String>,
    pub(crate) database: Option<u16>,
}

#[derive(Deserialize, Debug, Default, Clone)]
pub(crate) struct DatabaseSection {
    pub(crate) host: Option<String>,
    pub(crate) port: Option<u16>,
    pub(crate) username: Option<String>,
    pub(crate) password: Option<String>,
    pub(crate) database: Option<String>,
    pub(crate) max_connections: Option<u32>,
}

#[derive(Deserialize, Debug, Default, Clone)]
pub(crate) struct NatsSection {
    pub(crate) url: Option<String>,
}

#[derive(Deserialize, Debug, Default)]
pub(crate) struct ApiServerSection {
    pub(crate) network: Option<ApiNetworkSection>,
    pub(crate) security: Option<ApiSecuritySection>,
    pub(crate) admin_credentials: Option<AdminCredentialsSection>,
}

#[derive(Deserialize, Debug, Default)]
pub(crate) struct ApiNetworkSection {
    pub(crate) host: Option<String>,
    pub(crate) port: Option<u16>,
    pub(crate) allowed_origins: Option<String>,
}

#[derive(Deserialize, Debug, Default)]
pub(crate) struct ApiSecuritySection {
    pub(crate) jwt_secret: Option<String>,
    pub(crate) internal_secret: Option<String>,
}

#[derive(Deserialize, Debug, Default)]
pub(crate) struct AdminCredentialsSection {
    pub(crate) admin_password: Option<String>,
    pub(crate) operator_password: Option<String>,
    pub(crate) financier_password: Option<String>,
}

#[derive(Deserialize, Debug, Default)]
pub(crate) struct SipEdgeConfigSection {
    pub(crate) network: Option<SipEdgeNetworkSection>,
    pub(crate) cluster: Option<SipEdgeClusterSection>,
    pub(crate) auth: Option<SipEdgeAuthSection>,
}

#[derive(Deserialize, Debug, Default)]
pub(crate) struct SipEdgeNetworkSection {
    pub(crate) manage_bind: Option<String>,
}

#[derive(Deserialize, Debug, Default)]
pub(crate) struct SipEdgeClusterSection {
    pub(crate) enabled: Option<bool>,
    pub(crate) node_key_prefix: Option<String>,
    pub(crate) management_url: Option<String>,
}

#[derive(Deserialize, Debug, Default)]
pub(crate) struct SipEdgeAuthSection {
    pub(crate) realm: Option<String>,
}

/// 从 `VOS_RS_CONFIG_FILE` 指定的 YAML 文件加载 API Server 配置。
pub(crate) fn load_config() -> anyhow::Result<ApiServerConfig> {
    let config_file_path =
        env::var("VOS_RS_CONFIG_FILE").unwrap_or_else(|_| "config.yaml".to_string());
    let config_content = std::fs::read_to_string(&config_file_path)
        .map_err(|error| anyhow::anyhow!("读取配置文件 {config_file_path} 失败: {error}"))?;
    serde_yaml::from_str(&config_content)
        .map_err(|error| anyhow::anyhow!("解析配置文件 {config_file_path} 失败: {error}"))
}
