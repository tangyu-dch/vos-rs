use axum::{
    extract::{Query, State},
    response::{IntoResponse, Response},
    Json,
};
use cdr_core::{BillingAccountRecord, BillingAccountType, BillingJournalFilter};
use sqlx::Row;

use crate::{normalize_page, parse_dt, ApiError, AppState, PaginatedResponse};

use super::support::{database_error, parse_optional_account_type, AccountListQuery, JournalQuery};

pub async fn list_access_accounts(
    state: State<AppState>,
    query: Query<AccountListQuery>,
) -> Result<Response, ApiError> {
    list_accounts(state, query, BillingAccountType::Access).await
}

pub async fn list_egress_accounts(
    state: State<AppState>,
    query: Query<AccountListQuery>,
) -> Result<Response, ApiError> {
    list_accounts(state, query, BillingAccountType::Egress).await
}

async fn list_accounts(
    State(state): State<AppState>,
    Query(query): Query<AccountListQuery>,
    account_type: BillingAccountType,
) -> Result<Response, ApiError> {
    let (page, page_size, offset) = normalize_page(&query.page);
    let keyword = query
        .q
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let (items, total) = tokio::try_join!(
        state
            .store
            .list_billing_accounts_page(account_type, page_size, offset, keyword),
        state.store.count_billing_accounts(account_type, keyword),
    )
    .map_err(database_error)?;
    if query.page.export.unwrap_or(false) {
        let rows = items
            .iter()
            .map(|item| {
                vec![
                    item.id.to_string(),
                    item.username.clone(),
                    item.balance.to_string(),
                    item.credit_limit.to_string(),
                    item.created_at.to_string(),
                ]
            })
            .collect::<Vec<_>>();
        return Ok(crate::system::utils::to_csv_response(
            "billing-accounts.csv",
            &["账户编号", "账户名称", "余额", "信用额度", "创建时间"],
            &rows,
        ));
    }
    let items = attach_gateway_links(&state, items).await?;
    Ok(Json(PaginatedResponse {
        items,
        total,
        page,
        page_size,
    })
    .into_response())
}

async fn attach_gateway_links(
    state: &AppState,
    accounts: Vec<BillingAccountRecord>,
) -> Result<Vec<serde_json::Value>, ApiError> {
    let account_ids = accounts
        .iter()
        .map(|account| account.id)
        .collect::<Vec<_>>();
    let rows = sqlx::query(
        "SELECT account_id, id FROM sip_gateways WHERE account_id = ANY($1) ORDER BY id",
    )
    .bind(&account_ids)
    .fetch_all(state.store.pool())
    .await
    .map_err(database_error)?;
    let mut links = std::collections::HashMap::<i64, Vec<String>>::new();
    for row in rows {
        links
            .entry(row.get("account_id"))
            .or_default()
            .push(row.get("id"));
    }
    Ok(accounts
        .into_iter()
        .map(|account| {
            let gateways = links.get(&account.id).cloned().unwrap_or_default();
            let trunk_id = gateways.first().cloned();
            serde_json::json!({
                "id": account.id, "name": account.username, "username": account.username,
                "account_type": account.account_type,
                "tenant_id": account.tenant_id,
                "trunk_id": trunk_id,
                "trunk_ids": gateways,
                "balance": account.balance, "credit_limit": account.credit_limit,
                "enabled": account.enabled,
                "billing_interval_secs": account.billing_interval_secs,
                "price_per_interval": account.price_per_interval,
                "created_at": account.created_at, "updated_at": account.updated_at,
            })
        })
        .collect())
}

pub async fn list_journal(
    State(state): State<AppState>,
    Query(query): Query<JournalQuery>,
) -> Result<Response, ApiError> {
    let account_type = parse_optional_account_type(query.account_type.as_deref())?;
    let start = query.start_time.as_deref().and_then(parse_dt);
    let end = query.end_time.as_deref().and_then(parse_dt);
    let (page, page_size, offset) = normalize_page(&query.page);
    let entry_type = query.entry_type.as_deref().or(Some("credit"));
    let keyword = query
        .q
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let filter = BillingJournalFilter {
        account_type,
        account_id: query.account_id,
        entry_type,
        query: keyword,
        start_time: start,
        end_time: end,
    };
    let (items, total) = tokio::try_join!(
        state
            .store
            .list_billing_journal_page(&filter, page_size, offset),
        state.store.count_billing_journal(&filter),
    )
    .map_err(database_error)?;
    if query.page.export.unwrap_or(false) {
        let rows = items
            .iter()
            .map(|item| {
                vec![
                    item.id.to_string(),
                    item.username.clone(),
                    item.entry_type.clone(),
                    item.amount.to_string(),
                    item.balance_before.to_string(),
                    item.balance_after.to_string(),
                    item.operator_username.clone(),
                    item.created_at.to_string(),
                    item.remark.clone(),
                ]
            })
            .collect::<Vec<_>>();
        return Ok(crate::system::utils::to_csv_response(
            "billing-journal.csv",
            &[
                "流水号",
                "账户",
                "类型",
                "金额",
                "变动前余额",
                "变动后余额",
                "操作人",
                "发生时间",
                "备注",
            ],
            &rows,
        ));
    }
    Ok(Json(PaginatedResponse {
        items,
        total,
        page,
        page_size,
    })
    .into_response())
}
