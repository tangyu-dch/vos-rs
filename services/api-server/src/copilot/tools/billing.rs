//! Copilot 工具实现：计费账户、资费与防刷风控规则

use cdr_core::CreditAccountOutcome;
use rust_decimal::Decimal;
use serde_json::{json, Value};

use crate::copilot::TelecomCopilotEngine;

impl<'a> TelecomCopilotEngine<'a> {
    pub(crate) async fn tool_list_anti_fraud_rules(&self) -> Value {
        let rules = self.state.store.list_anti_fraud_rules().await.unwrap_or_default();
        json!({ "rules": rules })
    }

    pub(crate) async fn tool_list_billing_accounts(&self) -> Value {
        match self.state.store.list_accounts().await {
            Ok(accs) => json!({ "accounts": accs }),
            Err(e) => json!({ "error": e.to_string() }),
        }
    }

    pub(crate) async fn tool_recharge_billing_account(&self, args: &Value) -> Value {
        let acc_id = args.get("account_id").and_then(|v| v.as_str()).unwrap_or("");
        let amount = args.get("amount").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let desc = args.get("description").and_then(|v| v.as_str()).unwrap_or("Copilot 充值操作");
        if acc_id.is_empty() || amount == 0.0 {
            return json!({ "success": false, "error": "账户 ID 与变动金额不能为空或 0" });
        }
        let amount_dec = Decimal::from_f64_retain(amount).unwrap_or_default();
        match self.state.store.credit_account(acc_id, amount_dec, desc).await {
            Ok(CreditAccountOutcome::Applied(new_bal)) | Ok(CreditAccountOutcome::Replayed(new_bal)) => {
                json!({ "success": true, "message": format!("账户 {} 成功变动金额 {}, 当前新余额: {}", acc_id, amount, new_bal) })
            }
            Ok(CreditAccountOutcome::Conflict) => {
                json!({ "success": false, "error": "重复的充值幂等请求" })
            }
            Err(e) => json!({ "success": false, "error": e.to_string() }),
        }
    }

    pub(crate) async fn tool_list_rates(&self) -> Value {
        match self.state.store.list_rates().await {
            Ok(rates) => json!({ "rates": rates }),
            Err(e) => json!({ "error": e.to_string() }),
        }
    }

    pub(crate) async fn tool_upsert_rate(&self, args: &Value) -> Value {
        let id = args.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let prefix = args.get("prefix").and_then(|v| v.as_str()).unwrap_or("");
        let rate = args.get("rate_per_minute").and_then(|v| v.as_f64()).unwrap_or(0.0);
        if id.is_empty() || prefix.is_empty() {
            return json!({ "success": false, "error": "费率 ID 与前缀不能为空" });
        }
        let rate_dec = Decimal::from_f64_retain(rate).unwrap_or_default();
        match self.state.store.upsert_rate(id, prefix, rate_dec, 60, rate_dec, None).await {
            Ok(_) => json!({ "success": true, "message": format!("费率规则 {} (前缀: {}, 费率: {}/分) 设置成功", id, prefix, rate) }),
            Err(e) => json!({ "success": false, "error": e.to_string() }),
        }
    }

    pub(crate) async fn tool_delete_rate(&self, args: &Value) -> Value {
        let id = args.get("id").and_then(|v| v.as_str()).unwrap_or("");
        match self.state.store.delete_rate(id).await {
            Ok(true) => json!({ "success": true, "message": format!("费率 {} 已成功删除", id) }),
            Ok(false) => json!({ "success": false, "error": format!("费率 {} 不存在", id) }),
            Err(e) => json!({ "error": e.to_string() }),
        }
    }

    pub(crate) async fn tool_create_anti_fraud_rule(&self, args: &Value) -> Value {
        let id = args.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let name = args.get("name").or_else(|| args.get("pattern")).and_then(|v| v.as_str()).unwrap_or("");
        let pattern = args.get("pattern").and_then(|v| v.as_str()).unwrap_or("");
        let limit = args.get("limit_count").and_then(|v| v.as_i64()).unwrap_or(60) as i32;
        if id.is_empty() || pattern.is_empty() {
            return json!({ "success": false, "error": "风控规则 ID 与 Pattern 模式不能为空" });
        }
        let res = sqlx::query(
            "INSERT INTO anti_fraud_rules (id, rule_type, target_value, limit_number, enabled) VALUES ($1, $2, $3, $4, true) \
             ON CONFLICT (id) DO UPDATE SET target_value = EXCLUDED.target_value, limit_number = EXCLUDED.limit_number"
        )
        .bind(id)
        .bind("rate_limit")
        .bind(pattern)
        .bind(limit)
        .execute(self.state.store.pool())
        .await;
        match res {
            Ok(_) => json!({ "success": true, "message": format!("防刷风控规则 {} (模式: {}, 上限: {}) 创建成功", name, pattern, limit) }),
            Err(e) => json!({ "success": false, "error": e.to_string() }),
        }
    }

    pub(crate) async fn tool_delete_anti_fraud_rule(&self, args: &Value) -> Value {
        let id = args.get("id").and_then(|v| v.as_str()).unwrap_or("");
        match self.state.store.delete_anti_fraud_rule(id).await {
            Ok(true) => json!({ "success": true, "message": format!("风控规则 {} 已成功删除", id) }),
            Ok(false) => json!({ "success": false, "error": format!("风控规则 {} 不存在", id) }),
            Err(e) => json!({ "error": e.to_string() }),
        }
    }
}
