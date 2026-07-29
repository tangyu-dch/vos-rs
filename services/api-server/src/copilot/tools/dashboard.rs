//! Copilot 工具实现：查询类（dashboard/cdr/sip_flows/active_calls/terminate/registrations）

use serde_json::{json, Value};

use crate::copilot::{generate_ascii_ladder, ladder_from_flows, TelecomCopilotEngine};

impl<'a> TelecomCopilotEngine<'a> {
    pub(crate) async fn tool_get_dashboard_stats(&self) -> Value {
        let stats = self.state.store.get_dashboard_stats(0).await.ok();
        json!(stats)
    }

    /// 每日汇报工具：返回当日总结、分小时通话趋势、失败原因分布、Top 失败主被叫对。
    ///
    /// 供 LLM 生成"每日汇报/今日运行情况/日报"风格的结构化回答，包含：
    /// - 当日总结（总通话/接通率/平均时长/计费分钟/音质）
    /// - 呼叫情况（分小时趋势）
    /// - 问题原因分析（失败原因 Top N + Top 失败主被叫对）
    pub(crate) async fn tool_get_daily_report(&self, args: &Value) -> Value {
        let date = args.get("date").and_then(|v| v.as_str());
        let top_n = args.get("top_n").and_then(|v| v.as_i64()).map(|n| n as i32);
        match self.state.store.get_daily_report(date, top_n).await {
            Ok(report) => json!(report),
            Err(e) => json!({ "error": e.to_string() }),
        }
    }

    pub(crate) async fn tool_list_cdrs(&self, args: &Value) -> Value {
        let status = args.get("status").and_then(|v| v.as_str());
        let caller = args.get("caller").and_then(|v| v.as_str());
        let callee = args.get("callee").and_then(|v| v.as_str());
        let limit = args
            .get("limit")
            .and_then(|v| v.as_i64())
            .unwrap_or(10)
            .clamp(1, 50);
        match self
            .state
            .store
            .list_cdrs(1, limit, status, None, caller, callee, None, None, None)
            .await
        {
            Ok((cdrs, total)) => json!({ "total": total, "cdrs": cdrs }),
            Err(e) => json!({ "error": e.to_string() }),
        }
    }

    pub(crate) async fn tool_get_sip_flows(&self, args: &Value) -> Value {
        let call_id = args.get("call_id").and_then(|v| v.as_str()).unwrap_or("");
        let flows = self
            .state
            .store
            .get_sip_flows(call_id)
            .await
            .unwrap_or_default();
        let steps = ladder_from_flows(&flows, 0);
        let ascii = generate_ascii_ladder(&steps);
        json!({ "call_id": call_id, "flows": flows, "ladder_diagram": ascii })
    }

    pub(crate) async fn tool_list_active_calls(&self) -> Value {
        let calls = self.state.active_calls_cache.get_or_fetch(self.state).await;
        json!({ "active_calls": calls, "count": calls.len() })
    }

    pub(crate) async fn tool_terminate_call(&self, args: &Value) -> Value {
        let call_id = args.get("call_id").and_then(|v| v.as_str()).unwrap_or("");
        let url = format!(
            "{}/manage/calls/{}/terminate",
            self.state.sip_manage_base, call_id
        );
        let req = self.state.internal_client.post(&url);
        let req = if !self.state.internal_secret.is_empty() {
            req.header("X-VOS-Token", &self.state.internal_secret)
        } else {
            req
        };
        match req.send().await {
            Ok(r) if r.status().is_success() => {
                json!({ "success": true, "message": format!("通话 {} 已成功拆线挂断", call_id) })
            }
            Ok(r) => json!({ "success": false, "error": format!("HTTP {}", r.status()) }),
            Err(e) => json!({ "success": false, "error": e.to_string() }),
        }
    }

    pub(crate) async fn tool_list_registrations(&self, args: &Value) -> Value {
        let regs = self
            .state
            .store
            .list_registrations()
            .await
            .unwrap_or_default();
        let username = args.get("username").and_then(|v| v.as_str());
        let filtered: Vec<_> = if let Some(u) = username {
            regs.into_iter().filter(|r| r.aor.contains(u)).collect()
        } else {
            regs
        };
        json!({ "registrations": filtered })
    }
}
