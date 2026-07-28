//! 租户注册表：内存中的 domain → TenantRecord 映射，支持周期刷新。

use super::context::TenantContext;
use super::store::{TenantRecord, TenantStore};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info};

/// 租户注册表：从 SIP 域名查找租户配置。
///
/// 内部使用 `Arc<RwLock<HashMap>>` 存储所有启用的租户记录，
/// 由后台任务周期从 PostgreSQL 刷新。
#[derive(Debug, Clone)]
pub struct TenantRegistry {
    inner: Arc<RwLock<HashMap<String, TenantRecord>>>,
    store: TenantStore,
}

impl TenantRegistry {
    /// 创建空注册表（启动时使用，待后台任务填充）。
    pub fn new(store: TenantStore) -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
            store,
        }
    }

    /// 从内存注册表按域查找租户，构造 TenantContext。
    ///
    /// 若未找到或租户已被禁用则返回 `TenantContext::from_domain(domain)`（保持向后兼容）。
    pub async fn context_for_domain(&self, domain: &str) -> TenantContext {
        let map = self.inner.read().await;
        if let Some(record) = map.get(&domain.to_ascii_lowercase()) {
            if !record.is_active() {
                debug!(domain = %record.domain, tenant_id = %record.id, "tenant disabled, falling back to anonymous");
                return TenantContext::from_domain(domain.to_string());
            }
            TenantContext {
                tenant_id: Some(record.id.clone()),
                tenant_name: Some(record.name.clone()),
                domain: Some(record.domain.clone()),
                policy: record.policy.clone(),
            }
        } else {
            TenantContext::from_domain(domain.to_string())
        }
    }

    /// 从内存注册表按 SIP From 头解析的域查找租户。
    ///
    /// `from_header` 为 SIP From 头的值，例如 `<sip:1001@acme.com:5060>` 或
    /// `"Alice" <sip:alice@acme.com>`。
    pub async fn context_for_from_header(&self, from_header: &str) -> TenantContext {
        let domain = extract_domain_from_from(from_header);
        match domain {
            Some(d) => self.context_for_domain(&d).await,
            None => TenantContext::anonymous(),
        }
    }

    /// 从 PostgreSQL 全量刷新内存注册表。
    pub async fn refresh(&self) -> usize {
        let loaded = self.store.load_all().await;
        let count = loaded.len();
        let mut map = self.inner.write().await;
        *map = loaded;
        debug!(tenant_count = count, "tenant registry refreshed");
        count
    }

    /// 启动后台周期刷新任务。
    ///
    /// 每 `interval_secs` 秒从 PostgreSQL 重新加载所有租户记录。
    /// 首次刷新立即执行。
    pub fn spawn_refresh_loop(self: Arc<Self>, interval_secs: u64) {
        if interval_secs == 0 {
            info!("tenant registry auto-refresh disabled (interval_secs=0)");
            return;
        }
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
            loop {
                interval.tick().await;
                let count = self.refresh().await;
                debug!(
                    tenant_count = count,
                    "periodic tenant registry refresh done"
                );
            }
        });
        info!(interval_secs, "tenant registry refresh loop started");
    }

    /// 返回当前注册表中的租户数量（用于测试与监控）。
    pub async fn tenant_count(&self) -> usize {
        self.inner.read().await.len()
    }

    /// 返回所有已加载租户的简要信息（用于管理 API 与可观测性）。
    pub async fn list_tenants(&self) -> Vec<TenantSummary> {
        let map = self.inner.read().await;
        map.values()
            .map(|record| TenantSummary {
                id: record.id.clone(),
                name: record.name.clone(),
                domain: record.domain.clone(),
                enabled: record.enabled,
                max_concurrent_calls: record.policy.max_concurrent_calls,
                max_cps: record.policy.max_cps,
                cross_tenant_policy: record.policy.cross_tenant_policy,
                recording_enabled: record.policy.recording_enabled,
                billing_account_id: record.policy.billing_account_id,
            })
            .collect()
    }
}

/// 租户简要信息（管理 API 返回结构）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct TenantSummary {
    pub id: String,
    pub name: String,
    pub domain: String,
    pub enabled: bool,
    pub max_concurrent_calls: u32,
    pub max_cps: u32,
    pub cross_tenant_policy: super::policy::CrossTenantPolicy,
    pub recording_enabled: Option<bool>,
    pub billing_account_id: Option<i64>,
}

/// 从 SIP From 头值中提取域部分。
///
/// 支持以下格式：
/// - `<sip:user@domain:port>`
/// - `<sip:user@domain>`
/// - `"Display" <sip:user@domain>`
/// - `sip:user@domain`
pub(crate) fn extract_domain_from_from(from_header: &str) -> Option<String> {
    // 找到 `<...>` 内的内容，否则使用整个字符串
    let uri_part = if let Some(start) = from_header.find('<') {
        let end = from_header.find('>')?;
        &from_header[start + 1..end]
    } else {
        from_header.trim()
    };

    // 跳过 "sip:" 或 "sips:" 前缀
    let after_scheme = if let Some(rest) = uri_part.strip_prefix("sips:") {
        rest
    } else if let Some(rest) = uri_part.strip_prefix("sip:") {
        rest
    } else {
        uri_part
    };

    // 找到 '@'，之后的就是 user@domain:port
    let at_index = after_scheme.find('@')?;
    let domain_with_port = &after_scheme[at_index + 1..];
    // 去掉端口
    let end_pos = domain_with_port.find(':').unwrap_or(domain_with_port.len());
    // 去掉 URI 参数（;user=phone 等）
    let end_pos = domain_with_port[..end_pos].find(';').unwrap_or(end_pos);
    let domain = &domain_with_port[..end_pos];
    if domain.is_empty() {
        None
    } else {
        Some(domain.to_ascii_lowercase())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_domain_from_simple_uri() {
        let domain = extract_domain_from_from("sip:1001@acme.com");
        assert_eq!(domain.as_deref(), Some("acme.com"));
    }

    #[test]
    fn extracts_domain_from_angle_brackets() {
        let domain = extract_domain_from_from("<sip:1001@acme.com:5060>");
        assert_eq!(domain.as_deref(), Some("acme.com"));
    }

    #[test]
    fn extracts_domain_from_display_name_format() {
        let domain = extract_domain_from_from("\"Alice\" <sip:alice@acme.com>");
        assert_eq!(domain.as_deref(), Some("acme.com"));
    }

    #[test]
    fn extracts_domain_with_sips_scheme() {
        let domain = extract_domain_from_from("<sips:bob@secure.example.org:5061>");
        assert_eq!(domain.as_deref(), Some("secure.example.org"));
    }

    #[test]
    fn extracts_domain_with_uri_params() {
        let domain = extract_domain_from_from("<sip:1001@acme.com:5060;user=phone>");
        assert_eq!(domain.as_deref(), Some("acme.com"));
    }

    #[test]
    fn returns_none_for_empty_domain() {
        let domain = extract_domain_from_from("sip:1001@");
        assert!(domain.is_none());
    }

    #[test]
    fn lowercases_domain() {
        let domain = extract_domain_from_from("sip:user@ACME.COM");
        assert_eq!(domain.as_deref(), Some("acme.com"));
    }

    #[test]
    fn handles_malformed_from_header_gracefully() {
        let domain = extract_domain_from_from("not a sip header");
        assert!(domain.is_none());
    }
}
