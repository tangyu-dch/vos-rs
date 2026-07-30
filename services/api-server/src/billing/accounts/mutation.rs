use axum::{
    extract::{Extension, Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use cdr_core::{
    BillingAccountRecord, BillingAccountType, BillingCreditOutcome, CreateBillingAccountInput,
    DeleteBillingAccountOutcome, UpdateBillingAccountInput,
};
use rust_decimal::Decimal;

use crate::{
    system::{auth::Claims, hot_cache},
    ApiError, AppState,
};

use super::support::{
    database_error, idempotency_key, validate_account, validate_gateway_links, AccountBody,
    AccountCreditBody,
};

pub async fn create_access_account(
    state: State<AppState>,
    body: Json<AccountBody>,
) -> Result<(StatusCode, Json<BillingAccountRecord>), ApiError> {
    create_account(state, body, BillingAccountType::Access).await
}

pub async fn create_egress_account(
    state: State<AppState>,
    body: Json<AccountBody>,
) -> Result<(StatusCode, Json<BillingAccountRecord>), ApiError> {
    create_account(state, body, BillingAccountType::Egress).await
}

pub async fn update_access_account(
    state: State<AppState>,
    path: Path<i64>,
    body: Json<AccountBody>,
) -> Result<Json<BillingAccountRecord>, ApiError> {
    update_account(state, path, body, BillingAccountType::Access).await
}

pub async fn update_egress_account(
    state: State<AppState>,
    path: Path<i64>,
    body: Json<AccountBody>,
) -> Result<Json<BillingAccountRecord>, ApiError> {
    update_account(state, path, body, BillingAccountType::Egress).await
}

pub async fn delete_access_account(
    state: State<AppState>,
    path: Path<i64>,
) -> Result<StatusCode, ApiError> {
    delete_account(state, path, BillingAccountType::Access).await
}

pub async fn delete_egress_account(
    state: State<AppState>,
    path: Path<i64>,
) -> Result<StatusCode, ApiError> {
    delete_account(state, path, BillingAccountType::Egress).await
}

pub async fn credit_access_account(
    state: State<AppState>,
    path: Path<i64>,
    headers: HeaderMap,
    claims: Extension<Claims>,
    body: Json<AccountCreditBody>,
) -> Result<Response, ApiError> {
    credit_account(
        state,
        path,
        headers,
        claims,
        body,
        BillingAccountType::Access,
    )
    .await
}

pub async fn credit_egress_account(
    state: State<AppState>,
    path: Path<i64>,
    headers: HeaderMap,
    claims: Extension<Claims>,
    body: Json<AccountCreditBody>,
) -> Result<Response, ApiError> {
    credit_account(
        state,
        path,
        headers,
        claims,
        body,
        BillingAccountType::Egress,
    )
    .await
}

async fn create_account(
    State(state): State<AppState>,
    Json(body): Json<AccountBody>,
    account_type: BillingAccountType,
) -> Result<(StatusCode, Json<BillingAccountRecord>), ApiError> {
    validate_account(&body)?;
    validate_gateway_links(&state, &body.gateway_ids, account_type, None).await?;
    let tenant_id = normalize_tenant_id(body.tenant_id.as_deref());
    validate_tenant_link(&state, tenant_id, account_type).await?;
    let input = CreateBillingAccountInput {
        username: body.username.trim(),
        account_type,
        credit_limit: body.credit_limit,
        billing_interval_secs: body.billing_interval_secs,
        price_per_interval: body.price_per_interval,
        enabled: body.enabled,
        gateway_ids: &body.gateway_ids,
        tenant_id,
    };
    let account = state
        .store
        .create_billing_account(&input)
        .await
        .map_err(database_error)?;
    refresh_account_cache(&state, &account).await;
    Ok((StatusCode::CREATED, Json(account)))
}

async fn update_account(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(body): Json<AccountBody>,
    account_type: BillingAccountType,
) -> Result<Json<BillingAccountRecord>, ApiError> {
    validate_account(&body)?;
    validate_gateway_links(&state, &body.gateway_ids, account_type, Some(id)).await?;
    let tenant_id = normalize_tenant_id(body.tenant_id.as_deref());
    validate_tenant_link(&state, tenant_id, account_type).await?;
    let input = UpdateBillingAccountInput {
        username: body.username.trim(),
        credit_limit: body.credit_limit,
        billing_interval_secs: body.billing_interval_secs,
        price_per_interval: body.price_per_interval,
        enabled: body.enabled,
        gateway_ids: &body.gateway_ids,
        tenant_id,
    };
    let account = state
        .store
        .update_billing_account(id, account_type, &input)
        .await
        .map_err(database_error)?
        .ok_or_else(|| ApiError::not_found("计费账户不存在"))?;
    refresh_account_cache(&state, &account).await;
    Ok(Json(account))
}

/// 将前端传入的 tenant_id 规整为 None（空）或 Some(trimmed)。
fn normalize_tenant_id(raw: Option<&str>) -> Option<&str> {
    raw.map(str::trim).filter(|value| !value.is_empty())
}

/// 校验关联租户存在且启用。仅对接账户要求关联启用租户；落地账户允许无关联。
async fn validate_tenant_link(
    state: &AppState,
    tenant_id: Option<&str>,
    account_type: BillingAccountType,
) -> Result<(), ApiError> {
    if tenant_id.is_none() {
        if account_type == BillingAccountType::Access {
            // 对接账户未关联租户时不强制拦截，但业务上建议关联。保持兼容。
        }
        return Ok(());
    }
    let exists = state
        .store
        .tenant_exists_and_enabled(tenant_id)
        .await
        .map_err(database_error)?;
    if !exists {
        return Err(ApiError::bad_request("参数无效: 关联的租户不存在或未启用"));
    }
    Ok(())
}

async fn delete_account(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    account_type: BillingAccountType,
) -> Result<StatusCode, ApiError> {
    // 删除前先查出 username，用于清理 Redis 缓存。
    let username = state
        .store
        .get_billing_account_username(id, account_type)
        .await
        .map_err(database_error)?;
    match state
        .store
        .delete_billing_account(id, account_type)
        .await
        .map_err(database_error)?
    {
        DeleteBillingAccountOutcome::Deleted => {
            if let Some(name) = username {
                let _ = hot_cache::delete_billing_account(&state, &name).await;
            }
            Ok(StatusCode::NO_CONTENT)
        }
        DeleteBillingAccountOutcome::NotFound => Err(ApiError::not_found("计费账户不存在")),
        DeleteBillingAccountOutcome::InUse => Err(ApiError::bad_request(
            "参数无效: 账户仍被网关引用，不能删除",
        )),
    }
}

async fn credit_account(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    headers: HeaderMap,
    Extension(claims): Extension<Claims>,
    Json(body): Json<AccountCreditBody>,
    account_type: BillingAccountType,
) -> Result<Response, ApiError> {
    if body.amount <= Decimal::ZERO || body.amount > Decimal::from(100_000_000) {
        return Err(ApiError::bad_request(
            "参数无效: 充值金额必须大于零且不超过一亿元",
        ));
    }
    if body.remark.chars().count() > 500 {
        return Err(ApiError::bad_request(
            "参数无效: 充值备注不能超过 500 个字符",
        ));
    }
    let key = idempotency_key(&headers)?;
    match state
        .store
        .credit_billing_account(
            id,
            account_type,
            body.amount,
            key,
            &claims.sub,
            body.remark.trim(),
        )
        .await
        .map_err(database_error)?
    {
        BillingCreditOutcome::Applied(record) => {
            // 充值只影响余额，更新 Redis 余额缓存即可。
            let _ = hot_cache::set_account_balance(&state, &record.username, record.balance_after)
                .await;
            Ok((StatusCode::CREATED, Json(record)).into_response())
        }
        BillingCreditOutcome::Replayed(record) => Ok(Json(record).into_response()),
        BillingCreditOutcome::AccountNotFound => Err(ApiError::not_found("计费账户不存在或未启用")),
        BillingCreditOutcome::Conflict => {
            Err(ApiError::bad_request("参数无效: 幂等键已用于其他充值请求"))
        }
    }
}

/// 刷新单个账户的 Redis 计费缓存（余额、授信、周期、价格）。
async fn refresh_account_cache(state: &AppState, account: &BillingAccountRecord) {
    let interval_secs = u32::try_from(account.billing_interval_secs).unwrap_or(60);
    let _ = hot_cache::set_billing_account(
        state,
        &account.username,
        account.balance,
        account.credit_limit,
        interval_secs,
        account.price_per_interval,
    )
    .await;
}
