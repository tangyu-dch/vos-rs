//! Copilot 工具执行入口与降级报告生成
//!
//! 拆分自 copilot.rs，包含 execute_tool 与 build_fallback_report 方法
//! （impl TelecomCopilotEngine 扩展块）。
//! Rust 允许同一类型在多个文件用多个 impl 块。

mod billing;
mod dashboard;
mod extensions;
mod io;
mod network;

use serde_json::json;

use super::{
    generate_ascii_ladder, CopilotIntent, Payload, TelecomCopilotEngine, push_section, truncate,
};

#[allow(dead_code)]
fn urlencoding_str(s: &str) -> String {
    s.as_bytes()
        .iter()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
                (*byte as char).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}

impl<'a> TelecomCopilotEngine<'a> {
    #[allow(dead_code)]
    pub async fn execute_tool(&self, name: &str, args: &serde_json::Value) -> serde_json::Value {
        match name {
            "vos_get_dashboard_stats" => self.tool_get_dashboard_stats().await,
            "vos_list_cdrs" => self.tool_list_cdrs(args).await,
            "vos_get_sip_flows" => self.tool_get_sip_flows(args).await,
            "vos_list_active_calls" => self.tool_list_active_calls().await,
            "vos_terminate_call" => self.tool_terminate_call(args).await,
            "vos_list_registrations" => self.tool_list_registrations(args).await,
            "vos_list_gateways" => self.tool_list_gateways().await,
            "vos_preview_route" => self.tool_preview_route(args).await,
            "vos_list_anti_fraud_rules" => self.tool_list_anti_fraud_rules().await,
            "vos_list_extensions" => self.tool_list_extensions(args).await,
            "vos_create_extension" => self.tool_create_extension(args).await,
            "vos_delete_extension" => self.tool_delete_extension(args).await,
            "vos_list_ivr_menus" => self.tool_list_ivr_menus().await,
            "vos_create_ivr_menu" => self.tool_create_ivr_menu(args).await,
            "vos_create_gateway" => self.tool_create_gateway(args).await,
            "vos_delete_gateway" => self.tool_delete_gateway(args).await,
            "vos_create_route" => self.tool_create_route(args).await,
            "vos_list_routes" => self.tool_list_routes().await,
            "vos_delete_route" => self.tool_delete_route(args).await,
            "vos_list_billing_accounts" => self.tool_list_billing_accounts().await,
            "vos_recharge_billing_account" => self.tool_recharge_billing_account(args).await,
            "vos_list_rates" => self.tool_list_rates().await,
            "vos_upsert_rate" => self.tool_upsert_rate(args).await,
            "vos_delete_rate" => self.tool_delete_rate(args).await,
            "vos_add_ivr_node" => self.tool_add_ivr_node(args).await,
            "vos_delete_ivr_menu" => self.tool_delete_ivr_menu(args).await,
            "vos_create_anti_fraud_rule" => self.tool_create_anti_fraud_rule(args).await,
            "vos_delete_anti_fraud_rule" => self.tool_delete_anti_fraud_rule(args).await,
            "vos_export_cdrs" => self.tool_export_cdrs(args).await,
            "vos_export_extensions" => self.tool_export_extensions().await,
            "vos_export_gateways" => self.tool_export_gateways().await,
            "vos_export_routes" => self.tool_export_routes().await,
            "vos_export_rates" => self.tool_export_rates().await,
            "vos_export_billing_accounts" => self.tool_export_billing_accounts().await,
            "vos_import_extensions" => self.tool_import_extensions(args).await,
            "vos_import_gateways" => self.tool_import_gateways(args).await,
            "vos_import_routes" => self.tool_import_routes(args).await,
            "vos_import_rates" => self.tool_import_rates(args).await,
            _ => json!({ "error": format!("未知工具: {name}") }),
        }
    }

    pub(crate) fn build_fallback_report(
        query: &str,
        intent: CopilotIntent,
        payload: &Payload,
    ) -> (String, String, String) {
        let mut report = String::new();
        report.push_str("> ⚠️ **智能运维提示**：当前大模型分析通道繁忙（系统已自动激活**本地智能诊断模式**，实时提取软交换遥测数据，确保运维分析不中断）。\n\n");
        report.push_str("# 🛠️ 本地诊断与结构化遥测报告\n\n");
        report.push_str(&format!("- **用户查询指令**：`{}`\n", query));
        report.push_str(&format!("- **系统意图识别**：`{:?}`\n", intent));
        report.push_str(&format!("- **数据生成时间**：`{}`\n\n", payload.generated_at));

        if let Some(stats) = &payload.dashboard_stats {
            report.push_str("## 📊 软交换集群核心指标\n\n");
            report.push_str("| 监控指标 | 当前数值 | 运行状态 | 指标解读 |\n");
            report.push_str("| :--- | :--- | :---: | :--- |\n");

            let calls_str = format!("{} 次 (应答: {}, 取消: {}, 失败: {})", stats.today_total_calls, stats.today_answered_calls, stats.today_canceled_calls, stats.today_failed_calls);
            report.push_str(&format!("| **今日通话总数** | {} | 🟢 正常 | 今日平台承载呼叫总量 |\n", calls_str));

            let asr_status = if stats.answer_rate >= 0.85 { "🟢 优秀" } else if stats.answer_rate >= 0.60 { "🟡 一般" } else { "🔴 异常" };
            report.push_str(&format!("| **呼叫接通率 (ASR)** | {:.2}% | {} | 平台整体接通比例 |\n", stats.answer_rate * 100.0, asr_status));

            report.push_str(&format!("| **在线分机 / 活跃网关** | {} / {} | 🟢 在线 | 终端注册与中继链路活跃数 |\n", stats.registered_users, stats.active_gateways));

            let mos_val = stats.avg_mos.unwrap_or(4.4);
            let mos_status = if mos_val >= 4.0 { "🟢 极佳" } else if mos_val >= 3.0 { "🟡 一般" } else { "🔴 差" };
            report.push_str(&format!("| **平均通话质量 (MOS)** | {:.2} / 5 | {} | RTP 语音传输主观质量评分 |\n", mos_val, mos_status));

            let loss_val = stats.avg_loss_rate.unwrap_or(0.0);
            let loss_status = if loss_val < 0.01 { "🟢 极低" } else if loss_val < 0.05 { "🟡 轻微" } else { "🔴 严重" };
            report.push_str(&format!("| **媒体流平均丢包率** | {:.2}% | {} | 网络抖动丢包比例 |\n\n", loss_val * 100.0, loss_status));
        }

        if !payload.recent_failed_cdrs.is_empty() {
            report.push_str("## 🚨 异常释放呼叫列表 (Top 10)\n\n");
            report.push_str("| 呼叫 ID | 主叫号码 | 被叫号码 | 呼叫状态 | 挂断响应码 | 失败原因 |\n");
            report.push_str("| :--- | :--- | :--- | :---: | :---: | :--- |\n");
            for cdr in &payload.recent_failed_cdrs {
                report.push_str(&format!(
                    "| `{}` | `{}` | `{}` | `{}` | `{}` | `{}` |\n",
                    truncate(&cdr.call_id, 8),
                    cdr.caller.as_deref().unwrap_or("-"),
                    cdr.callee.as_deref().unwrap_or("-"),
                    cdr.status,
                    cdr.failure_status_code.map(|c| c.to_string()).unwrap_or_else(|| "-".to_string()),
                    cdr.failure_reason.as_deref().unwrap_or("-"),
                ));
            }
            report.push_str("\n\n");
        }

        if let Some(cdr) = &payload.latest_cdr {
            report.push_str("## 📄 最新通话原始 CDR 元数据\n");
            report.push_str(&format!(
                "```json\n{}\n```\n\n",
                serde_json::to_string_pretty(cdr).unwrap_or_default()
            ));
        }

        if !payload.sip_flows.is_empty() {
            report.push_str("## 📡 实时信令抓包轨迹 (Sip Flows)\n\n");
            report.push_str("| 抓包时间 | 信令方法 | 传输方向 | 源端 AOR | 目的端 AOR |\n");
            report.push_str("| :--- | :--- | :---: | :--- | :--- |\n");
            for f in payload.sip_flows.iter().take(15) {
                report.push_str(&format!(
                    "| `{}` | **{}** | `{}` | `{}` | `{}` |\n",
                    f.timestamp, f.method, f.direction, f.from_addr, f.to_addr
                ));
            }
            report.push_str("\n\n");
        }

        // SIP 信令交互梯形图：作为 markdown 代码块内嵌到报告中
        let ladder_steps = Self::build_ladder_steps(payload);
        if !ladder_steps.is_empty() {
            let ascii = generate_ascii_ladder(&ladder_steps);
            report.push_str("## 📊 SIP Call Flow Sequence Diagram (时序梯形图)\n");
            report.push_str(&format!("```text\n{ascii}```\n\n"));
        }

        push_section(&mut report, "🔑 SIP 在线注册状态", &payload.registrations, |r| {
            format!("- **分机地址 (AOR)**: `{}` | **联系人地址**: `{}` | **过期时间**: `{}`\n", r.aor, r.contact_uri, r.expires_at)
        });
        push_section(&mut report, "💳 计费账户余额", &payload.billing_accounts, |a| {
            format!(
                "- **账户名称**: `{}` | **余额**: `{} {}` | **信用额度**: `{}`\n",
                a.username, a.balance, a.currency, a.credit_limit
            )
        });
        push_section(&mut report, "🌐 对接网关配置", &payload.gateways, |g| {
            format!(
                "- **网关 ID**: `{}` | **中继地址**: `{}:{}` | **网关类型**: `{:?}` | **网关角色**: `{:?}`\n",
                g.id,
                g.host,
                g.port.unwrap_or(5060),
                g.gateway_type,
                g.role
            )
        });

        if !payload.active_calls.is_empty() {
            report.push_str(&format!("## 📞 当前活跃通话 ({})\n\n", payload.active_calls.len()));
            for c in payload.active_calls.iter().take(10) {
                report.push_str(&format!("- `{}`\n", c));
            }
            report.push('\n');
        }

        let (root_cause, suggested_action) = match intent {
            CopilotIntent::CallFailure if payload.recent_failed_cdrs.is_empty() => (
                "数据库中无失败通话记录。".to_string(),
                "无需处理；如需排查请确认 CDR 持久化开关已开启。".to_string(),
            ),
            CopilotIntent::CallFailure => {
                let cdr = &payload.recent_failed_cdrs[0];
                (
                    format!(
                        "最近失败通话 `{}` 状态为 `{}`，失败响应码为 `{:?}`。",
                        cdr.call_id, cdr.status, cdr.failure_status_code
                    ),
                    format!(
                        "建议检查 `failure_reason`（**{}**）；必要时联系中继运营商进行信令联调。",
                        cdr.failure_reason.as_deref().unwrap_or("未记录原因")
                    ),
                )
            }
            CopilotIntent::SipLadder if payload.sip_flows.is_empty() => (
                "系统中无当前呼叫的真实 SIP 抓包记录。".to_string(),
                "请确认信令网关 `sip-edge` 已开启信令持久化配置（检查数据库中的 `sip_flows` 表是否正常写入）。".to_string(),
            ),
            CopilotIntent::SipLadder => (
                format!("已成功从底层抓包数据加载 {} 条真实信令交互记录。", payload.sip_flows.len()),
                "请结合上方的交互时序梯形图，重点核对各请求的时间间隔与响应状态码（如 4xx/5xx）是否符合预期。".to_string(),
            ),
            CopilotIntent::SystemHealth => (
                "已基于系统底层核心指标进行健康性评估。".to_string(),
                "若当前 CPS、接通率或丢包率等核心指标异常，请排查 `sip-edge` 节点负载、Redis 鉴权缓存响应时间以及数据库连接池容量。".to_string(),
            ),
            _ => ("已在上方生成该场景的结构化真实业务数据。".to_string(), "请根据上述数据和呼叫情况，结合具体业务逻辑和运营商中继链路进行判断。".to_string()),
        };

        report.push_str("## 🔍 根因分析 (Root Cause)\n");
        report.push_str(&format!("{}\n\n", root_cause));
        report.push_str("## 💡 建议动作 (Suggested Action)\n");
        report.push_str(&format!("{}\n\n", suggested_action));

        (report, String::new(), String::new())
    }
}
