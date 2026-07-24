use crate::PageQuery;
use std::env;
use time::OffsetDateTime;

pub(crate) fn normalize_page(query: &PageQuery) -> (i64, i64, i64) {
    if query.export.unwrap_or(false) {
        return (1, 100000, 0);
    }
    let page = query.page.unwrap_or(1).max(1);
    let page_size = query.page_size.unwrap_or(20).clamp(1, 100);
    let offset = (page - 1).saturating_mul(page_size);
    (page, page_size, offset)
}

pub(crate) fn parse_dt(s: &str) -> Option<OffsetDateTime> {
    OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339).ok()
}

pub(crate) fn validate_runtime_secrets(
    production: bool,
    jwt_secret: &str,
    internal_secret: &str,
    admin_password: &str,
    operator_password: &str,
    financier_password: &str,
) -> anyhow::Result<()> {
    validate_runtime_secrets_for_environment(
        production,
        jwt_secret,
        internal_secret,
        admin_password,
        operator_password,
        financier_password,
    )
}

pub(crate) fn validate_runtime_secrets_for_environment(
    production: bool,
    jwt_secret: &str,
    internal_secret: &str,
    admin_password: &str,
    operator_password: &str,
    financier_password: &str,
) -> anyhow::Result<()> {
    if !production {
        return Ok(());
    }
    if jwt_secret.len() < 32 || jwt_secret.contains("change-in-production") {
        anyhow::bail!("生产环境 VOS_RS_API_JWT_SECRET 必须是至少 32 字符的随机密钥");
    }
    if internal_secret.len() < 24
        || matches!(
            internal_secret,
            "internal-dev-secret" | "compose-internal-secret"
        )
    {
        anyhow::bail!("生产环境 VOS_RS_INTERNAL_SECRET 必须是至少 24 字符的随机密钥");
    }
    for (name, value, default) in [
        ("VOS_RS_ADMIN_PASSWORD", admin_password, "admin"),
        ("VOS_RS_OPERATOR_PASSWORD", operator_password, "operator"),
        ("VOS_RS_FINANCIER_PASSWORD", financier_password, "financier"),
    ] {
        if value.len() < 12 || value == default || value.ends_with("-change-me") {
            anyhow::bail!("生产环境 {name} 必须是至少 12 字符的非默认密码");
        }
    }
    Ok(())
}

pub(crate) fn config_logging_filter(default: &str) -> String {
    let path = env::var("VOS_RS_CONFIG_FILE").unwrap_or_else(|_| "config.yaml".to_string());
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

#[cfg(test)]
mod tests {
    use super::{normalize_page, validate_runtime_secrets, validate_runtime_secrets_for_environment};
    use crate::PageQuery;

    #[test]
    fn management_page_parameters_are_bounded() {
        let query = PageQuery {
            page: Some(0),
            page_size: Some(10_000),
            gateway_type: None,
            role: None,
            export: None,
        };

        assert_eq!(normalize_page(&query), (1, 100, 0));
    }

    #[test]
    fn production_rejects_default_runtime_secrets() {
        let error = validate_runtime_secrets_for_environment(
            true,
            "api-jwt-change-in-production",
            "internal-dev-secret",
            "admin",
            "operator",
            "financier",
        )
        .expect_err("production defaults must be rejected");

        assert!(error.to_string().contains("VOS_RS_API_JWT_SECRET"));
    }

    #[test]
    fn development_allows_local_runtime_defaults() {
        assert!(validate_runtime_secrets_for_environment(
            false,
            "development",
            "internal-dev-secret",
            "admin",
            "operator",
            "financier",
        )
        .is_ok());
    }

    #[test]
    fn public_bind_rejects_default_runtime_secrets() {
        let addr: std::net::SocketAddr = "0.0.0.0:8080".parse().unwrap();
        let is_public = !addr.ip().is_loopback();
        assert!(is_public);
        let production = is_public;

        let error = validate_runtime_secrets(
            production,
            "vos-rs-secret-key-change-in-production",
            "internal-dev-secret",
            "admin",
            "operator",
            "financier",
        );
        assert!(error.is_err());
    }
}

