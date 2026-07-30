use axum::http::HeaderMap;
use cdr_core::BillingAccountType;
use rust_decimal::Decimal;
use serde::Deserialize;

use crate::{deserialize_optional_i64_from_str, ApiError, AppState, PageQuery};

#[derive(Debug, Deserialize)]
pub struct AccountListQuery {
    #[serde(flatten)]
    pub(super) page: PageQuery,
    pub(super) q: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AccountBody {
    pub(super) username: String,
    pub(super) credit_limit: Decimal,
    pub(super) billing_interval_secs: i32,
    pub(super) price_per_interval: Decimal,
    pub(super) enabled: bool,
    #[serde(default)]
    pub(super) gateway_ids: Vec<String>,
    /// 关联租户 ID（对接账户归属租户，落地账户通常为空）。
    /// 空字符串或未传视为无关联。
    #[serde(default)]
    pub(super) tenant_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AccountCreditBody {
    pub(super) amount: Decimal,
    #[serde(default)]
    pub(super) remark: String,
}

#[derive(Debug, Deserialize)]
pub struct JournalQuery {
    #[serde(flatten)]
    pub(super) page: PageQuery,
    pub(super) account_type: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_i64_from_str")]
    pub(super) account_id: Option<i64>,
    pub(super) entry_type: Option<String>,
    pub(super) q: Option<String>,
    pub(super) start_time: Option<String>,
    pub(super) end_time: Option<String>,
}

pub(super) fn validate_account(body: &AccountBody) -> Result<(), ApiError> {
    let username = body.username.trim();
    if username.is_empty() || username.chars().count() > 128 {
        return Err(ApiError::bad_request(
            "参数无效: 账户名称不能为空且不能超过 128 个字符",
        ));
    }
    if body.credit_limit < Decimal::ZERO || body.price_per_interval < Decimal::ZERO {
        return Err(ApiError::bad_request(
            "参数无效: 信用额度和周期价格不能为负数",
        ));
    }
    if !(1..=86_400).contains(&body.billing_interval_secs) {
        return Err(ApiError::bad_request(
            "参数无效: 计费周期必须在 1 到 86400 秒之间",
        ));
    }
    let mut seen = std::collections::HashSet::new();
    for raw_id in &body.gateway_ids {
        let trimmed = raw_id.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !seen.insert(trimmed) {
            return Err(ApiError::bad_request("参数无效: 关联中继列表中存在重复项"));
        }
    }
    Ok(())
}

pub(super) fn idempotency_key(headers: &HeaderMap) -> Result<&str, ApiError> {
    headers
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty() && value.len() <= 128)
        .ok_or_else(|| ApiError::bad_request("参数无效: 充值必须提供有效的 Idempotency-Key"))
}

pub(super) fn parse_optional_account_type(
    value: Option<&str>,
) -> Result<Option<BillingAccountType>, ApiError> {
    match value {
        None | Some("") => Ok(None),
        Some("access") => Ok(Some(BillingAccountType::Access)),
        Some("egress") => Ok(Some(BillingAccountType::Egress)),
        Some(_) => Err(ApiError::bad_request(
            "参数无效: 账户类型只能是 access 或 egress",
        )),
    }
}

pub(super) fn database_error(error: sqlx::Error) -> ApiError {
    if error
        .as_database_error()
        .is_some_and(|database| database.is_unique_violation())
    {
        ApiError::bad_request("参数无效: 账户名称已存在")
    } else {
        ApiError::internal(error.to_string())
    }
}

/// 校验所有关联网关类型匹配且未被其他账户占用，中继为空列表时直接通过。
pub(super) async fn validate_gateway_links(
    state: &AppState,
    gateway_ids: &[String],
    account_type: BillingAccountType,
    current_account_id: Option<i64>,
) -> Result<(), ApiError> {
    for raw_id in gateway_ids {
        let gateway_id = raw_id.trim();
        if gateway_id.is_empty() {
            continue;
        }
        let available = state
            .store
            .gateway_available_for_account(gateway_id, account_type, current_account_id)
            .await
            .map_err(database_error)?;
        if !available {
            return Err(ApiError::bad_request(
                "参数无效: 网关不存在、类型不匹配、已关联其他账户，或对接网关未关联启用租户",
            ));
        }
    }
    Ok(())
}
