use cdr_core::PostgresCdrStore;
use tracing::info;

use crate::config::EdgeConfig;
use crate::routing::parse_gateway_target;

pub type AnyError = Box<dyn std::error::Error + Send + Sync>;

pub fn validate_bootstrap_config() -> Result<(), AnyError> {
    let path = std::env::var("VOS_RS_CONFIG_FILE").unwrap_or_else(|_| "config.yaml".to_string());
    let content = std::fs::read_to_string(&path)
        .map_err(|error| std::io::Error::other(format!("读取配置文件 {path} 失败: {error}")))?;
    serde_yaml::from_str::<serde_yaml::Value>(&content)
        .map_err(|error| std::io::Error::other(format!("解析配置文件 {path} 失败: {error}")))?;
    Ok(())
}

pub fn validate_runtime_security(config: &EdgeConfig) -> Result<(), AnyError> {
    let production =
        std::env::var("VOS_RS_ENV").is_ok_and(|value| value.eq_ignore_ascii_case("production"));
    validate_runtime_security_for_environment(config, production)
}

pub(crate) fn validate_runtime_security_for_environment(
    config: &EdgeConfig,
    production: bool,
) -> Result<(), AnyError> {
    if !production {
        return Ok(());
    }
    if config.internal_secret.len() < 24
        || matches!(
            config.internal_secret.as_str(),
            "internal-dev-secret" | "compose-internal-secret"
        )
    {
        return Err(std::io::Error::other(
            "生产环境 VOS_RS_INTERNAL_SECRET 必须是至少 24 字符的随机密钥",
        )
        .into());
    }
    if config.auth.secret_key.len() < 24 || config.auth.secret_key.contains("change-me") {
        return Err(std::io::Error::other(
            "生产环境 SIP Digest secret_key 必须是至少 24 字符的随机密钥",
        )
        .into());
    }
    if config.tls_insecure_skip_verify || config.tls_allow_test_certificate {
        return Err(std::io::Error::other("生产环境禁止跳过 TLS 校验或启用测试证书").into());
    }
    Ok(())
}

pub fn init_tracing(filter: &str) {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new(filter))
        .init();
}

pub fn config_logging_filter(default: &str) -> String {
    let path = std::env::var("VOS_RS_CONFIG_FILE").unwrap_or_else(|_| "config.yaml".to_string());
    std::fs::read_to_string(path)
        .ok()
        .and_then(|content| serde_yaml::from_str::<serde_yaml::Value>(&content).ok())
        .and_then(|root| {
            root.get("logging")?
                .get("filter")?
                .as_str()
                .map(str::to_owned)
        })
        .filter(|filter| !filter.trim().is_empty())
        .unwrap_or_else(|| default.to_string())
}

pub async fn seed_database_defaults(
    db: &PostgresCdrStore,
    edge_config: &EdgeConfig,
) -> Result<(), AnyError> {
    let has_users = sqlx::query("SELECT 1 FROM sip_users LIMIT 1")
        .fetch_optional(db.pool())
        .await?
        .is_some();
    if !has_users {
        if let Some(raw_users) = edge_config.bootstrap_auth_users.as_deref() {
            for entry in raw_users.split(',') {
                let entry = entry.trim();
                if let Some((username, password)) =
                    entry.split_once(':').or_else(|| entry.split_once('='))
                {
                    let username = username.trim();
                    let password = password.trim();
                    if !username.is_empty() {
                        // 引导用户不关联租户，保持向后兼容。
                        db.insert_user(username, password, None).await?;
                        info!(username, "seeded SIP user into database");
                    }
                }
            }
        }
    }

    if edge_config.database_routes_enabled {
        let has_gateways = sqlx::query("SELECT 1 FROM sip_gateways LIMIT 1")
            .fetch_optional(db.pool())
            .await?
            .is_some();
        if !has_gateways && !edge_config.default_gateway.trim().is_empty() {
            let raw_gateway = edge_config.default_gateway.trim();
            if let Ok(target) = parse_gateway_target("default", raw_gateway) {
                db.insert_gateway("default", &target.host, target.port, "udp")
                    .await?;
                db.insert_route("default", "", 100, "default").await?;
                info!(gateway = raw_gateway, "seeded default gateway and route");
            }
        }
    }
    Ok(())
}
