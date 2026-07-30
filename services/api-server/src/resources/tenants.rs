//! 租户管理资源 API
//!
//! 提供多租户的 CRUD 接口，并支持将租户关联到 `billing_accounts.id` 实现
//! 租户级统一计费。路由挂载于 `/api/v1/tenants`。

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use uuid::Uuid;

use crate::{
    deserialize_optional_bool_from_str, normalize_page, ApiError, AppState, PageQuery,
    PaginatedResponse,
};
use cdr_core::{TenantBillingSummary, TenantRecord, UpsertTenantInput};

/// 租户列表项：租户记录 + 关联对接账户聚合摘要（含欠费状态）
#[derive(Debug, Serialize)]
pub(crate) struct TenantListItem {
    #[serde(flatten)]
    pub tenant: TenantRecord,
    /// 关联对接账户的聚合摘要（总余额、欠费账户数、状态等）。
    /// 无关联账户时为空摘要（status=no_accounts）。
    pub billing_summary: TenantBillingSummary,
}

/// 创建租户请求：复用 `UpsertTenantInput`，但 `id` 由服务端生成
#[derive(Debug, Deserialize)]
pub(crate) struct CreateTenantRequest {
    pub name: String,
    pub domain: String,
    #[serde(default)]
    pub max_concurrent_calls: i32,
    #[serde(default)]
    pub max_cps: i32,
    #[serde(default = "default_cross_tenant_policy")]
    pub cross_tenant_policy: String,
    #[serde(default)]
    pub recording_enabled: Option<bool>,
    #[serde(default)]
    pub allowed_gateway_ids: Option<Vec<String>>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// 可选：调用方指定 ID（缺省由服务端生成 UUID v4）
    #[serde(default)]
    pub id: Option<String>,
}

fn default_cross_tenant_policy() -> String {
    "allow_if_same_domain".to_string()
}

fn default_enabled() -> bool {
    true
}

impl From<CreateTenantRequest> for UpsertTenantInput {
    fn from(req: CreateTenantRequest) -> Self {
        Self {
            name: req.name,
            domain: req.domain,
            max_concurrent_calls: req.max_concurrent_calls,
            max_cps: req.max_cps,
            cross_tenant_policy: req.cross_tenant_policy,
            recording_enabled: req.recording_enabled,
            allowed_gateway_ids: req.allowed_gateway_ids,
            billing_account_id: None,
            enabled: req.enabled,
        }
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct TenantQuery {
    #[serde(flatten)]
    pub page: PageQuery,
    /// 按名称或域模糊过滤
    pub q: Option<String>,
    /// 仅返回启用/禁用租户
    #[serde(default, deserialize_with = "deserialize_optional_bool_from_str")]
    pub enabled: Option<bool>,
}

/// 列出租户（含关联对接账户聚合摘要与欠费状态）
pub(crate) async fn list_tenants(
    State(state): State<AppState>,
    Query(query): Query<TenantQuery>,
) -> Result<axum::response::Response, ApiError> {
    let (page, page_size, offset) = normalize_page(&query.page);

    // q 与 enabled 在 SQL 层过滤，避免分页后内存过滤导致 total 不准
    let q_trim = query.q.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let tenants = state
        .store
        .list_tenants_page(page_size, offset, q_trim, query.enabled)
        .await
        .map_err(|e| ApiError::internal(format!("查询租户列表失败: {e}")))?;
    let total = state
        .store
        .count_tenants(q_trim, query.enabled)
        .await
        .map_err(|e| ApiError::internal(format!("统计租户总数失败: {e}")))?;

    // 批量加载所有租户的对接账户摘要，按 tenant_id 分组组装聚合摘要
    let tenant_ids: Vec<String> = tenants.iter().map(|t| t.id.clone()).collect();
    let summaries = state
        .store
        .list_account_summaries_by_tenants(&tenant_ids)
        .await
        .map_err(|e| ApiError::internal(format!("加载租户计费摘要失败: {e}")))?;
    let mut grouped: std::collections::HashMap<String, Vec<cdr_core::TenantAccountSummary>> =
        std::collections::HashMap::new();
    for (tenant_id, summary) in summaries {
        grouped.entry(tenant_id).or_default().push(summary);
    }

    // 导出 CSV
    if query.page.export.unwrap_or(false) {
        let headers = vec![
            "ID",
            "名称",
            "域",
            "最大并发",
            "最大CPS",
            "跨租户策略",
            "录音",
            "账户数",
            "总余额",
            "欠费账户数",
            "状态",
            "启用",
            "更新时间",
        ];
        let mut rows = Vec::new();
        for t in &tenants {
            let summary = TenantBillingSummary::from_accounts(
                grouped.get(&t.id).cloned().unwrap_or_default(),
            );
            rows.push(vec![
                t.id.clone(),
                t.name.clone(),
                t.domain.clone(),
                t.max_concurrent_calls.to_string(),
                t.max_cps.to_string(),
                t.cross_tenant_policy.clone(),
                t.recording_enabled
                    .map(|b| b.to_string())
                    .unwrap_or_default(),
                summary.account_count.to_string(),
                summary.total_balance.to_string(),
                summary.overdue_count.to_string(),
                summary.status.to_string(),
                t.enabled.to_string(),
                t.updated_at.to_string(),
            ]);
        }
        return Ok(crate::system::utils::to_csv_response(
            "tenants.csv",
            &headers,
            &rows,
        ));
    }

    let items: Vec<TenantListItem> = tenants
        .into_iter()
        .map(|t| {
            let accounts = grouped.get(&t.id).cloned().unwrap_or_default();
            let billing_summary = TenantBillingSummary::from_accounts(accounts);
            TenantListItem {
                tenant: t,
                billing_summary,
            }
        })
        .collect();

    Ok(Json(PaginatedResponse {
        items,
        total,
        page,
        page_size,
    })
    .into_response())
}

/// 获取单个租户详情（含关联对接账户聚合摘要）
pub(crate) async fn get_tenant(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<TenantListItem>, ApiError> {
    let tenant = state
        .store
        .get_tenant(&id)
        .await
        .map_err(|e| ApiError::internal(format!("查询租户失败: {e}")))?
        .ok_or_else(|| ApiError::not_found(format!("租户不存在: {id}")))?;

    let accounts = state
        .store
        .list_access_accounts_by_tenant(&id)
        .await
        .map_err(|e| ApiError::internal(format!("加载租户计费账户失败: {e}")))?
        .into_iter()
        .map(|account| cdr_core::TenantAccountSummary {
            id: account.id,
            username: account.username,
            balance: account.balance,
            credit_limit: account.credit_limit,
            price_per_interval: account.price_per_interval,
            enabled: account.enabled,
        })
        .collect();
    let billing_summary = TenantBillingSummary::from_accounts(accounts);

    Ok(Json(TenantListItem {
        tenant,
        billing_summary,
    }))
}

/// 创建租户
pub(crate) async fn create_tenant(
    State(state): State<AppState>,
    Json(req): Json<CreateTenantRequest>,
) -> Result<(StatusCode, Json<TenantRecord>), ApiError> {
    validate_tenant_input(&req.name, &req.domain)?;

    // 校验 domain 唯一性
    if let Some(existing) = state
        .store
        .get_tenant_by_domain(&req.domain)
        .await
        .map_err(|e| ApiError::internal(format!("校验域唯一性失败: {e}")))?
    {
        return Err(ApiError::bad_request(format!(
            "参数无效: 域名 {} 已被租户 {} 占用",
            req.domain, existing.id
        )));
    }

    let id = req
        .id
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let input: UpsertTenantInput = req.into();

    let record = state
        .store
        .create_tenant(&id, &input)
        .await
        .map_err(map_tenant_db_error)?;

    tracing::info!(tenant_id = %record.id, domain = %record.domain, "租户已创建");
    Ok((StatusCode::CREATED, Json(record)))
}

/// 更新租户
pub(crate) async fn update_tenant(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<CreateTenantRequest>,
) -> Result<Json<TenantRecord>, ApiError> {
    validate_tenant_input(&req.name, &req.domain)?;

    // 校验 domain 唯一性（排除自身）
    if let Some(existing) = state
        .store
        .get_tenant_by_domain(&req.domain)
        .await
        .map_err(|e| ApiError::internal(format!("校验域唯一性失败: {e}")))?
    {
        if existing.id != id {
            return Err(ApiError::bad_request(format!(
                "参数无效: 域名 {} 已被租户 {} 占用",
                req.domain, existing.id
            )));
        }
    }

    let input: UpsertTenantInput = req.into();
    let record = state
        .store
        .update_tenant(&id, &input)
        .await
        .map_err(map_tenant_db_error)?
        .ok_or_else(|| ApiError::not_found(format!("租户不存在: {id}")))?;

    tracing::info!(tenant_id = %record.id, domain = %record.domain, "租户已更新");
    Ok(Json(record))
}

/// 删除租户
pub(crate) async fn delete_tenant(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let deleted = state
        .store
        .delete_tenant(&id)
        .await
        .map_err(|e| ApiError::internal(format!("删除租户失败: {e}")))?;
    if deleted {
        tracing::info!(tenant_id = %id, "租户已删除");
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::not_found(format!("租户不存在: {id}")))
    }
}

/// 切换租户启用状态
#[derive(Debug, Deserialize)]
pub(crate) struct ToggleEnabledBody {
    pub enabled: bool,
}

pub(crate) async fn toggle_tenant_enabled(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<ToggleEnabledBody>,
) -> Result<Json<JsonValue>, ApiError> {
    let pool = state.store.pool();
    let row = sqlx::query(
        "UPDATE tenants SET enabled = $2, updated_at = now() \
         WHERE id = $1 \
         RETURNING id, name, domain, enabled, billing_account_id, updated_at",
    )
    .bind(&id)
    .bind(body.enabled)
    .fetch_optional(pool)
    .await
    .map_err(|e| ApiError::internal(format!("切换租户启用状态失败: {e}")))?
    .ok_or_else(|| ApiError::not_found(format!("租户不存在: {id}")))?;

    use sqlx::Row;
    Ok(Json(serde_json::json!({
        "id": row.get::<String, _>("id"),
        "name": row.get::<String, _>("name"),
        "domain": row.get::<String, _>("domain"),
        "enabled": row.get::<bool, _>("enabled"),
        "billing_account_id": row.get::<Option<i64>, _>("billing_account_id"),
        "updated_at": row.get::<time::OffsetDateTime, _>("updated_at"),
    })))
}

// ===== 内部辅助函数 =====

fn validate_tenant_input(name: &str, domain: &str) -> Result<(), ApiError> {
    if name.trim().is_empty() || name.len() > 128 {
        return Err(ApiError::bad_request(
            "参数无效: 租户名称不能为空且长度不超过 128 字符",
        ));
    }
    if domain.trim().is_empty() || domain.len() > 253 {
        return Err(ApiError::bad_request(
            "参数无效: 域名不能为空且长度不超过 253 字符",
        ));
    }
    // 简单校验域名字符（字母、数字、点、连字符）
    if !domain
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_')
    {
        return Err(ApiError::bad_request(
            "参数无效: 域名只能包含字母、数字、点、连字符和下划线",
        ));
    }
    Ok(())
}

/// 将 sqlx 错误映射为业务错误
fn map_tenant_db_error(e: sqlx::Error) -> ApiError {
    if let Some(db_err) = e.as_database_error() {
        if db_err.is_unique_violation() {
            return ApiError::bad_request("参数无效: 域名或 ID 已存在");
        }
        if db_err.is_foreign_key_violation() {
            return ApiError::bad_request("参数无效: 关联的计费账户不存在");
        }
    }
    ApiError::internal(format!("租户数据库操作失败: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_tenant_input_accepts_valid() {
        assert!(validate_tenant_input("租户A", "example.com").is_ok());
        assert!(validate_tenant_input("Tenant 1", "tenant-1.example.com").is_ok());
    }

    #[test]
    fn validate_tenant_input_rejects_empty_name() {
        assert!(validate_tenant_input("", "example.com").is_err());
    }

    #[test]
    fn validate_tenant_input_rejects_empty_domain() {
        assert!(validate_tenant_input("name", "").is_err());
    }

    #[test]
    fn validate_tenant_input_rejects_invalid_domain_chars() {
        assert!(validate_tenant_input("name", "exa mple.com").is_err());
        assert!(validate_tenant_input("name", "exa;mple.com").is_err());
    }

    #[test]
    fn validate_tenant_input_rejects_overlong_name() {
        let long_name = "a".repeat(129);
        assert!(validate_tenant_input(&long_name, "example.com").is_err());
    }

    #[test]
    fn from_create_request_preserves_fields() {
        let req = CreateTenantRequest {
            name: "测试".to_string(),
            domain: "test.example.com".to_string(),
            max_concurrent_calls: 100,
            max_cps: 10,
            cross_tenant_policy: "deny".to_string(),
            recording_enabled: Some(true),
            allowed_gateway_ids: Some(vec!["gw1".to_string()]),
            enabled: false,
            id: None,
        };
        let input: UpsertTenantInput = req.into();
        assert_eq!(input.name, "测试");
        assert_eq!(input.domain, "test.example.com");
        assert_eq!(input.max_concurrent_calls, 100);
        assert_eq!(input.max_cps, 10);
        assert_eq!(input.cross_tenant_policy, "deny");
        assert_eq!(input.recording_enabled, Some(true));
        assert!(!input.enabled);
    }
}
