use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

/// 独立的计费账户类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BillingAccountType {
    Access,
    Egress,
}

impl BillingAccountType {
    /// 返回数据库和 HTTP API 使用的稳定字符串。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Access => "access",
            Self::Egress => "egress",
        }
    }
}

/// 对接或落地计费账户。
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct BillingAccountRecord {
    pub id: i64,
    pub username: String,
    pub account_type: String,
    pub balance: Decimal,
    pub credit_limit: Decimal,
    pub billing_interval_secs: i32,
    pub price_per_interval: Decimal,
    pub enabled: bool,
    /// 关联租户 ID（对接账户归属租户，落地账户通常为空）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}

/// 创建计费账户所需字段。余额只能通过充值接口变更。
#[derive(Debug, Clone)]
pub struct CreateBillingAccountInput<'a> {
    pub username: &'a str,
    pub account_type: BillingAccountType,
    pub credit_limit: Decimal,
    pub billing_interval_secs: i32,
    pub price_per_interval: Decimal,
    pub enabled: bool,
    pub gateway_ids: &'a [String],
    /// 创建时关联租户 ID（可选）。
    pub tenant_id: Option<&'a str>,
}

/// 修改计费账户所需字段。账户类型和余额不可直接修改。
#[derive(Debug, Clone)]
pub struct UpdateBillingAccountInput<'a> {
    pub username: &'a str,
    pub credit_limit: Decimal,
    pub billing_interval_secs: i32,
    pub price_per_interval: Decimal,
    pub enabled: bool,
    pub gateway_ids: &'a [String],
    /// 更新时关联租户 ID（None 表示清除关联，Some 表示设置归属）。
    pub tenant_id: Option<&'a str>,
}

/// 账户删除结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteBillingAccountOutcome {
    Deleted,
    NotFound,
    InUse,
}

/// 不可变的财务流水记录。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, sqlx::FromRow)]
pub struct BillingJournalRecord {
    pub id: i64,
    pub account_id: i64,
    pub username: String,
    pub account_type: String,
    pub entry_type: String,
    pub amount: Decimal,
    pub balance_before: Decimal,
    pub balance_after: Decimal,
    pub call_id: Option<String>,
    pub operator_username: String,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    pub remark: String,
    pub idempotency_key: Option<String>,
}

/// 充值幂等处理结果。
#[derive(Debug, Clone, PartialEq)]
pub enum BillingCreditOutcome {
    Applied(BillingJournalRecord),
    Replayed(BillingJournalRecord),
    AccountNotFound,
    Conflict,
}

/// 财务流水筛选条件。
#[derive(Debug, Clone)]
pub struct BillingJournalFilter<'a> {
    pub account_type: Option<BillingAccountType>,
    pub account_id: Option<i64>,
    pub entry_type: Option<&'a str>,
    pub query: Option<&'a str>,
    pub start_time: Option<OffsetDateTime>,
    pub end_time: Option<OffsetDateTime>,
}

/// 租户下关联账户的摘要（含欠费判断）。
///
/// `is_overdue` 沿用 Redis 热路径判断规则：
/// `balance + credit_limit < price_per_interval`（且 price_per_interval > 0）。
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
pub struct TenantAccountSummary {
    pub id: i64,
    pub username: String,
    pub balance: Decimal,
    pub credit_limit: Decimal,
    pub price_per_interval: Decimal,
    pub enabled: bool,
}

impl TenantAccountSummary {
    /// 判断该账户是否欠费。
    ///
    /// 沿用 `edge_state::billing::build_balance_check` 的规则：
    /// 周期价格为零时不视为欠费；否则要求 `balance + credit_limit >= price_per_interval`。
    pub fn is_overdue(&self) -> bool {
        self.price_per_interval > Decimal::ZERO
            && self.balance + self.credit_limit < self.price_per_interval
    }
}

/// 租户计费聚合摘要：总余额、总信用额度、欠费账户数等。
#[derive(Debug, Clone, Serialize)]
pub struct TenantBillingSummary {
    pub accounts: Vec<TenantAccountSummary>,
    pub total_balance: Decimal,
    pub total_credit_limit: Decimal,
    pub account_count: i32,
    pub overdue_count: i32,
    /// `normal` / `overdue` / `no_accounts`
    pub status: &'static str,
}

impl TenantBillingSummary {
    pub fn from_accounts(accounts: Vec<TenantAccountSummary>) -> Self {
        let account_count = i32::try_from(accounts.len()).unwrap_or(i32::MAX);
        let total_balance = accounts
            .iter()
            .fold(Decimal::ZERO, |acc, item| acc + item.balance);
        let total_credit_limit = accounts
            .iter()
            .fold(Decimal::ZERO, |acc, item| acc + item.credit_limit);
        let overdue_count =
            i32::try_from(accounts.iter().filter(|a| a.is_overdue()).count()).unwrap_or(i32::MAX);
        let status = if accounts.is_empty() {
            "no_accounts"
        } else if overdue_count > 0 {
            "overdue"
        } else {
            "normal"
        };
        Self {
            accounts,
            total_balance,
            total_credit_limit,
            account_count,
            overdue_count,
            status,
        }
    }
}
