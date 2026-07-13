use serde::{Deserialize, Serialize};

/// 存储后端类型。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "snake_case")]
pub enum StorageBackendKind {
    /// 本地文件系统
    #[default]
    Local,
    /// OSS 兼容对象存储（阿里云 OSS、MinIO 等）
    Oss,
    /// 双写模式：主写 OSS，回退本地
    Dual,
}

/// 存储规则：按文件名前缀匹配，决定存储行为。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageRule {
    /// 匹配的 key 前缀（如 "wav/"、"recordings/part-"）
    pub prefix: String,
    /// 该前缀的文件是否仅写主存储（不回退本地）
    #[serde(default = "default_true")]
    pub primary_only: bool,
    /// 过期天数（0 = 永不过期）
    #[serde(default)]
    pub retention_days: u32,
}

fn default_true() -> bool {
    true
}

impl StorageRule {
    pub fn matches(&self, key: &str) -> bool {
        key.starts_with(&self.prefix)
    }
}

/// 存储配置，支持从环境变量或配置文件加载。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StorageConfig {
    /// 存储后端类型：local / oss / dual
    #[serde(default)]
    pub backend: StorageBackendKind,

    // --- 本地存储配置 ---
    /// 本地存储根目录
    #[serde(default = "default_local_dir")]
    pub local_dir: String,

    // --- OSS 配置 ---
    /// OSS endpoint（如 https://oss-cn-hangzhou.aliyuncs.com）
    pub oss_endpoint: Option<String>,
    /// OSS bucket 名称
    pub oss_bucket: Option<String>,
    /// OSS access key ID
    pub oss_access_key: Option<String>,
    /// OSS access key secret
    pub oss_secret_key: Option<String>,
    /// OSS key 前缀（目录），如 "vos-rs/recordings/"
    pub oss_key_prefix: Option<String>,
    /// OSS 区域
    pub oss_region: Option<String>,

    // --- 存储规则 ---
    /// 自定义存储规则列表
    #[serde(default)]
    pub rules: Vec<StorageRule>,

    // --- 通用 ---
    /// 最大上传大小（字节），0 = 不限制
    #[serde(default)]
    pub max_upload_size: u64,
    /// 预签名 URL 有效期（秒），默认 3600
    #[serde(default = "default_presign_ttl")]
    pub presign_ttl_secs: u64,
}

fn default_local_dir() -> String {
    "recordings".to_string()
}

fn default_presign_ttl() -> u64 {
    3600
}

#[derive(serde::Deserialize, Debug, Default)]
struct UnifiedYamlConfigForStorage {
    connections: Option<ConnectionsSectionForStorage>,
}

#[derive(serde::Deserialize, Debug, Default)]
struct ConnectionsSectionForStorage {
    s3: Option<RustFsSection>,
}

#[derive(serde::Deserialize, Debug, Default)]
struct RustFsSection {
    backend: Option<String>,
    local_dir: Option<String>,
    endpoint: Option<String>,
    bucket: Option<String>,
    access_key: Option<String>,
    secret_key: Option<String>,
    region: Option<String>,
    key_prefix: Option<String>,
}

impl StorageConfig {
    pub fn load() -> Self {
        let config_file_path =
            std::env::var("VOS_RS_CONFIG_FILE").unwrap_or_else(|_| "config.yaml".to_string());
        let content = std::fs::read_to_string(&config_file_path).unwrap_or_default();
        let u_config: UnifiedYamlConfigForStorage =
            serde_yaml::from_str(&content).unwrap_or_default();
        let s3 = u_config.connections.and_then(|c| c.s3).unwrap_or_default();
        let backend = s3
            .backend
            .unwrap_or_else(|| "local".to_string())
            .parse()
            .unwrap_or(StorageBackendKind::Local);
        Self {
            backend,
            local_dir: s3.local_dir.unwrap_or_else(default_local_dir),
            oss_endpoint: s3.endpoint,
            oss_bucket: s3.bucket,
            oss_access_key: s3.access_key,
            oss_secret_key: s3.secret_key,
            oss_region: s3.region,
            oss_key_prefix: s3.key_prefix,
            rules: Vec::new(),
            max_upload_size: 0,
            presign_ttl_secs: default_presign_ttl(),
        }
    }

    /// 从环境变量/配置文件加载配置。
    pub fn from_env() -> Self {
        Self::load()
    }
}

impl std::str::FromStr for StorageBackendKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "local" => Ok(StorageBackendKind::Local),
            "oss" | "s3" => Ok(StorageBackendKind::Oss),
            "dual" => Ok(StorageBackendKind::Dual),
            _ => Err(format!("未知的存储后端: {s}，可选: local, oss, s3, dual")),
        }
    }
}
