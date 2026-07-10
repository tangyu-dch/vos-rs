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

impl StorageConfig {
    /// 从环境变量加载配置。
    pub fn from_env() -> Self {
        let backend = std::env::var("VOS_RS_STORAGE_BACKEND")
            .unwrap_or_else(|_| "local".to_string())
            .parse()
            .unwrap_or(StorageBackendKind::Local);

        let rules = serde_json::from_str(
            &std::env::var("VOS_RS_STORAGE_RULES").unwrap_or_else(|_| "[]".to_string()),
        )
        .unwrap_or_default();

        Self {
            backend,
            local_dir: std::env::var("VOS_RS_RECORDING_DIR")
                .unwrap_or_else(|_| default_local_dir()),
            oss_endpoint: std::env::var("VOS_RS_OSS_ENDPOINT").ok(),
            oss_bucket: std::env::var("VOS_RS_OSS_BUCKET").ok(),
            oss_access_key: std::env::var("VOS_RS_OSS_ACCESS_KEY").ok(),
            oss_secret_key: std::env::var("VOS_RS_OSS_SECRET_KEY").ok(),
            oss_key_prefix: std::env::var("VOS_RS_OSS_KEY_PREFIX").ok(),
            oss_region: std::env::var("VOS_RS_OSS_REGION").ok(),
            rules,
            max_upload_size: std::env::var("VOS_RS_STORAGE_MAX_UPLOAD_SIZE")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0),
            presign_ttl_secs: std::env::var("VOS_RS_STORAGE_PRESIGN_TTL")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(default_presign_ttl()),
        }
    }
}

impl std::str::FromStr for StorageBackendKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "local" => Ok(StorageBackendKind::Local),
            "oss" => Ok(StorageBackendKind::Oss),
            "dual" => Ok(StorageBackendKind::Dual),
            _ => Err(format!("未知的存储后端: {s}，可选: local, oss, dual")),
        }
    }
}
