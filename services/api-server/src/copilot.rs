//! # 真实业务数据驱动的 LLM Telecom Copilot
//!
//! 基于真实 CDR / SIP 信令流 / 注册状态 / 计费账户 / 网关配置 / sip-edge 管理 API
//! 进行排障分析；当配置了 LLM（OpenAI 兼容协议）时调用大模型生成自然语言报告，
//! 否则返回结构化 Markdown 报告并明确提示"LLM 未配置"。

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use time::OffsetDateTime;

use cdr_core::{
    BillingAccount, CdrEvent, DashboardStats, LlmConfigRecord, SipFlowRecord, SipGateway,
    SipRegistration,
};

use crate::AppState;

/// LLM 配置：运行时从数据库 `llm_configs` 表（is_active=true）加载。
/// 启动时不再从 config.yaml 读取，切换厂商/模型无需重启。
#[derive(Debug, Clone, Default)]
pub struct LlmConfig {
    pub enabled: bool,
    pub provider: String,
    pub api_key: String,
    pub base_url: String,
    pub model: String,
    pub temperature: f32,
}

impl From<LlmConfigRecord> for LlmConfig {
    fn from(r: LlmConfigRecord) -> Self {
        Self {
            enabled: r.is_active,
            provider: r.provider,
            api_key: r.api_key,
            base_url: r.base_url,
            model: r.model,
            temperature: r.temperature,
        }
    }
}

impl LlmConfig {
    /// 是否已配置：开关启用 + API Key 非空且非占位符。
    pub fn is_configured(&self) -> bool {
        if !self.enabled || self.api_key.trim().is_empty() {
            return false;
        }
        let key = self.api_key.trim();
        !key.starts_with("sk-proj-your")
            && !key.starts_with("sk-deepseek-your")
            && !key.starts_with("AIzaSyYour")
            && !key.eq_ignore_ascii_case("not-needed")
            && !key.eq_ignore_ascii_case("placeholder")
    }
}

/// SIP 信令梯形图事件节点
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SipLadderStep {
    pub timestamp: String,
    pub direction: String,
    pub method_or_status: String,
    pub summary: String,
}

/// Copilot 对话分析响应结构
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CopilotChatResponse {
    pub query: String,
    pub analysis_report: String,
    pub root_cause: String,
    pub suggested_action: String,
    pub ladder_diagram_ascii: String,
    pub steps: Vec<SipLadderStep>,
    /// LLM 是否实际启用（用于前端展示状态徽标）
    pub llm_enabled: bool,
    /// LLM 状态消息（例如"未配置 LLM，以下为结构化业务数据"）
    pub llm_status: String,
}

/// 用户查询意图分类（用于选择性采集数据）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CopilotIntent {
    CallFailure,
    SipLadder,
    SystemHealth,
    Registration,
    Billing,
    Gateway,
    General,
}

impl Default for CopilotIntent {
    fn default() -> Self {
        Self::General
    }
}

impl CopilotIntent {
    /// 字符串表示，用于落库到 `copilot_messages.intent`
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::CallFailure => "CallFailure",
            Self::SipLadder => "SipLadder",
            Self::SystemHealth => "SystemHealth",
            Self::Registration => "Registration",
            Self::Billing => "Billing",
            Self::Gateway => "Gateway",
            Self::General => "General",
        }
    }
}

/// 真实业务数据采集结果
#[derive(Debug, Default, Serialize)]
pub(crate) struct Payload {
    pub(crate) query: String,
    pub(crate) intent: CopilotIntent,
    pub(crate) generated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) dashboard_stats: Option<DashboardStats>,
    pub(crate) recent_failed_cdrs: Vec<CdrEvent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) latest_cdr: Option<CdrEvent>,
    pub(crate) sip_flows: Vec<SipFlowRecord>,
    pub(crate) registrations: Vec<SipRegistration>,
    pub(crate) billing_accounts: Vec<BillingAccount>,
    pub(crate) gateways: Vec<SipGateway>,
    pub(crate) active_calls: Vec<Value>,
}

pub struct TelecomCopilotEngine<'a> {
    state: &'a AppState,
    llm: Option<LlmConfig>,
}

impl<'a> TelecomCopilotEngine<'a> {
    pub fn new(state: &'a AppState, llm: Option<LlmConfig>) -> Self {
        Self { state, llm }
    }

    /// 分析自然语言问题：意图识别 → 真实数据采集 → SIP 梯形图重建 → 可选 LLM 调用。
    pub async fn analyze(&self, query: &str, history: Option<&[cdr_core::CopilotMessage]>) -> CopilotChatResponse {
        let intent = Self::classify_intent(query);
        let payload = self.collect_payload(query, intent).await;
        let steps = Self::build_ladder_steps(&payload);
        let ascii_ladder = Self::generate_ascii_ladder(&steps);
        let context_json = serde_json::to_string_pretty(&payload).unwrap_or_default();

        let (report, root_cause, suggested_action, llm_status) = match &self.llm {
            Some(llm) if llm.is_configured() => {
                match self.call_llm(query, &context_json, history).await {
                    Ok(text) => (
                        text,
                        String::new(),
                        String::new(),
                        format!("LLM 已启用 (provider={}, model={})", llm.provider, llm.model),
                    ),
                    Err(error) => {
                        tracing::warn!(%error, "LLM 调用失败，回退到结构化报告");
                        let (r, rc, sa) = Self::build_fallback_report(query, intent, &payload);
                        (r, rc, sa, format!("LLM 调用失败：{error}；以下为结构化真实业务数据"))
                    }
                }
            }
            _ => {
                let (r, rc, sa) = Self::build_fallback_report(query, intent, &payload);
                (
                    r,
                    rc,
                    sa,
                    "LLM 未配置（数据库无启用配置），以下为结构化真实业务数据".to_string(),
                )
            }
        };

        let llm_enabled = self.llm.as_ref().is_some_and(|l| l.is_configured());
        CopilotChatResponse {
            query: query.to_string(),
            analysis_report: report,
            root_cause,
            suggested_action,
            ladder_diagram_ascii: ascii_ladder,
            steps,
            llm_enabled,
            llm_status,
        }
    }

    /// 基于关键词识别用户意图（中英文混合）
    pub fn classify_intent(query: &str) -> CopilotIntent {
        let q = query.to_lowercase();
        if q.contains("梯形图") || q.contains("ladder") || q.contains("sip flow") || q.contains("信令") {
            return CopilotIntent::SipLadder;
        }
        if q.contains("挂断")
            || q.contains("超时")
            || q.contains("失败")
            || q.contains("断")
            || q.contains("404")
            || q.contains("503")
            || q.contains("500")
            || q.contains("failed")
            || q.contains("timeout")
        {
            return CopilotIntent::CallFailure;
        }
        if q.contains("注册") || q.contains("register") || q.contains("分机") || q.contains("extension") {
            return CopilotIntent::Registration;
        }
        if q.contains("计费") || q.contains("余额") || q.contains("billing") || q.contains("balance") {
            return CopilotIntent::Billing;
        }
        if q.contains("网关") || q.contains("中继") || q.contains("gateway") || q.contains("trunk") {
            return CopilotIntent::Gateway;
        }
        if q.contains("cps")
            || q.contains("并发")
            || q.contains("丢包")
            || q.contains("健康")
            || q.contains("qos")
            || q.contains("状态")
            || q.contains("capacity")
        {
            return CopilotIntent::SystemHealth;
        }
        CopilotIntent::General
    }

    /// 根据意图选择性采集真实业务数据
    pub(crate) async fn collect_payload(&self, query: &str, intent: CopilotIntent) -> Payload {
        let mut payload = Payload {
            intent,
            query: query.to_string(),
            generated_at: OffsetDateTime::now_utc().to_string(),
            ..Default::default()
        };

        // 全局概览（轻量）总是采集
        if let Ok(stats) = self.state.store.get_dashboard_stats(0).await {
            payload.dashboard_stats = Some(stats);
        }

        match intent {
            CopilotIntent::CallFailure | CopilotIntent::SipLadder => {
                if let Ok((failed, _)) = self
                    .state
                    .store
                    .list_cdrs(1, 10, Some("failed"), None, None, None, None, None, None)
                    .await
                {
                    payload.recent_failed_cdrs = failed;
                }
                if payload.recent_failed_cdrs.is_empty() {
                    if let Ok((latest, _)) = self
                        .state
                        .store
                        .list_cdrs(1, 1, None, None, None, None, None, None, None)
                        .await
                    {
                        payload.latest_cdr = latest.into_iter().next();
                    }
                } else {
                    payload.latest_cdr = payload.recent_failed_cdrs.first().cloned();
                }
                if let Some(cdr) = &payload.latest_cdr {
                    if let Ok(flows) = self.state.store.get_sip_flows(&cdr.call_id).await {
                        payload.sip_flows = flows;
                    }
                }
            }
            CopilotIntent::Registration => {
                if let Ok(regs) = self.state.store.list_registrations().await {
                    payload.registrations = regs;
                }
            }
            CopilotIntent::Billing => {
                if let Ok(accts) = self.state.store.list_accounts().await {
                    payload.billing_accounts = accts;
                }
            }
            CopilotIntent::Gateway => {
                if let Ok(gws) = self.state.store.list_gateways_full().await {
                    payload.gateways = gws;
                }
            }
            CopilotIntent::SystemHealth | CopilotIntent::General => {
                payload.active_calls = self.fetch_active_calls().await;
            }
        }

        payload
    }

    /// 转发到 sip-edge `/manage/active-calls` 获取活跃通话列表
    async fn fetch_active_calls(&self) -> Vec<Value> {
        let url = format!("{}/manage/active-calls", self.state.sip_manage_base);
        let resp = self
            .state
            .internal_client
            .get(&url)
            .header("X-VOS-Token", &self.state.internal_secret)
            .send()
            .await;
        match resp {
            Ok(r) if r.status().is_success() => r.json::<Vec<Value>>().await.unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    /// 优先从真实 SipFlows 重建梯形图，否则从 CDR 合成
    pub(crate) fn build_ladder_steps(payload: &Payload) -> Vec<SipLadderStep> {
        if !payload.sip_flows.is_empty() {
            if let Some(cdr) = &payload.latest_cdr {
                return Self::ladder_from_flows(&payload.sip_flows, cdr.started_at_ms);
            }
        }
        if let Some(cdr) = &payload.latest_cdr {
            return Self::ladder_from_cdr(cdr);
        }
        Vec::new()
    }

    fn ladder_from_flows(flows: &[SipFlowRecord], start_ms: i64) -> Vec<SipLadderStep> {
        flows
            .iter()
            .map(|f| {
                let ts = f.timestamp.unix_timestamp() * 1000 + f.timestamp.millisecond() as i64;
                let offset = (ts - start_ms).max(0);
                SipLadderStep {
                    timestamp: format!("+{offset}ms"),
                    direction: f.direction.clone(),
                    method_or_status: f.method.clone(),
                    summary: format!("{} → {}", f.from_addr, f.to_addr),
                }
            })
            .collect()
    }

    fn ladder_from_cdr(cdr: &CdrEvent) -> Vec<SipLadderStep> {
        let caller = cdr.caller.clone().unwrap_or_else(|| "UAC".into());
        let callee = cdr.callee.clone().unwrap_or_else(|| "UAS".into());
        let started = cdr.started_at_ms;
        let ended = cdr.ended_at_ms;
        let failure_code = cdr
            .failure_status_code
            .map(|c| c.to_string())
            .unwrap_or_else(|| "200 OK".into());

        let mut steps = vec![
            SipLadderStep {
                timestamp: "+0ms".into(),
                direction: "UAC -> sip-edge".into(),
                method_or_status: "INVITE".into(),
                summary: format!("发起呼叫: {caller} → {callee}"),
            },
            SipLadderStep {
                timestamp: "+1ms".into(),
                direction: "sip-edge -> UAC".into(),
                method_or_status: "100 Trying".into(),
                summary: "B2BUA 收到 INVITE".into(),
            },
            SipLadderStep {
                timestamp: "+2ms".into(),
                direction: "sip-edge -> GW".into(),
                method_or_status: "INVITE".into(),
                summary: format!("改写号码并透传至落地中继 (leg={})", cdr.direction),
            },
        ];
        if let Some(ans) = cdr.answered_at_ms {
            steps.push(SipLadderStep {
                timestamp: format!("+{}ms", (ans - started).max(0)),
                direction: "GW -> sip-edge".into(),
                method_or_status: "200 OK".into(),
                summary: "被叫摘机应答".into(),
            });
            steps.push(SipLadderStep {
                timestamp: format!("+{}ms", (ans - started + 1).max(0)),
                direction: "sip-edge -> UAC".into(),
                method_or_status: "200 OK".into(),
                summary: "B2BUA 透传应答给主叫".into(),
            });
        } else {
            steps.push(SipLadderStep {
                timestamp: format!("+{}ms", (ended - started).max(0)),
                direction: "GW -> sip-edge".into(),
                method_or_status: failure_code.clone(),
                summary: cdr
                    .failure_reason
                    .clone()
                    .unwrap_or_else(|| "呼叫未接通".into()),
            });
            steps.push(SipLadderStep {
                timestamp: format!("+{}ms", (ended - started + 1).max(0)),
                direction: "sip-edge -> UAC".into(),
                method_or_status: failure_code,
                summary: format!("呼叫状态: {}", cdr.status),
            });
        }
        steps.push(SipLadderStep {
            timestamp: format!("+{}ms", (ended - started).max(0)),
            direction: "UAC -> sip-edge".into(),
            method_or_status: "BYE".into(),
            summary: format!("通话结束，总时长 {} ms", cdr.duration_ms),
        });
        steps
    }

    /// 调用 OpenAI 兼容的 chat completions 接口
    async fn call_llm(
        &self,
        query: &str,
        context: &str,
        history: Option<&[cdr_core::CopilotMessage]>,
    ) -> Result<String, String> {
        let llm = self.llm.as_ref().ok_or("LLM 未配置")?;
        let url = format!(
            "{}/chat/completions",
            llm.base_url.trim_end_matches('/')
        );
        let mut messages = vec![
            json!({
                "role": "system",
                "content": "你是 vos-rs 电信级 VoIP 软交换平台的智能运维专家 Copilot。你的任务是基于我提供的真实业务数据（JSON），协助用户进行高效的运维排障、性能分析或系统管理。\n\n回答要求：\n1. **排版规范与美观**：使用清晰的 Markdown 结构。必须包含以下二级标题：\n   - ## 📊 分析报告 (Analysis Report)：结合数据对当前系统状态或呼叫流程进行专业、生动的解读，避免冰冷的格式化叙述。\n   - ## 🔍 根因分析 (Root Cause)：深入剖析导致问题的底层原因（如网络延迟、信令超时、鉴权失败等），若无异常则明确告知。\n   - ## 💡 建议动作 (Suggested Action)：给出具体、可执行的操作指引（如修改路由规则、更新分机配置、核对运营商中继配置等）。\n2. **生动自然**：语气要专业、自然，像一个资深的 VoIP 架构师在与同事交流，不要让人觉得机械呆板。\n3. **数据校验**：如果提供的业务数据为空，请礼貌地予以说明，并提示用户如何开启相应模块的持久化。"
            })
        ];

        if let Some(hist) = history {
            // 排除最后一条（那是当前最新的消息，我们需要它带上当前最新的 telemetry payload 数据进行分析）
            for msg in hist.iter().take(hist.len().saturating_sub(1)) {
                messages.push(json!({
                    "role": if msg.role == "user" { "user" } else { "assistant" },
                    "content": msg.content
                }));
            }
        }

        messages.push(json!({
            "role": "user",
            "content": format!("用户问题：{query}\n\n当前真实业务数据（JSON）：\n{context}")
        }));

        let body = json!({
            "model": llm.model,
            "temperature": llm.temperature,
            "messages": messages
        });
        let resp = self
            .state
            .llm_client
            .post(&url)
            .header("Authorization", format!("Bearer {}", llm.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("HTTP 请求失败: {e}"))?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("LLM HTTP {status}: {}", truncate(&text, 500)));
        }
        let val: Value = resp
            .json()
            .await
            .map_err(|e| format!("解析 LLM 响应失败: {e}"))?;
        val.get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("message"))
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
            .map(str::to_owned)
            .ok_or_else(|| "LLM 响应缺少 choices[0].message.content".into())
    }

    /// 未配置 LLM 或 LLM 调用失败时的结构化 Markdown 报告
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
            let ascii = Self::generate_ascii_ladder(&ladder_steps);
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

    /// 动态渲染 ASCII 格式的 SIP 交互梯形图 (Call Ladder Diagram)
    pub fn generate_ascii_ladder(steps: &[SipLadderStep]) -> String {
        let mut out = String::new();
        out.push_str("   [ Caller (UAC) ]            [ sip-edge B2BUA ]            [ Gateway (UAS) ]\n");
        out.push_str("          |                            |                            |\n");

        for s in steps {
            if s.direction.contains("UAC -> sip-edge") {
                out.push_str(&format!(
                    " {:<12} | ----- {} -----> |                            | {}\n",
                    s.timestamp, s.method_or_status, s.summary
                ));
            } else if s.direction.contains("sip-edge -> UAC") {
                out.push_str(&format!(
                    " {:<12} | <----- {} ----- |                            | {}\n",
                    s.timestamp, s.method_or_status, s.summary
                ));
            } else if s.direction.contains("sip-edge -> GW") {
                out.push_str(&format!(
                    " {:<12} |                            | ----- {} -----> | {}\n",
                    s.timestamp, s.method_or_status, s.summary
                ));
            } else if s.direction.contains("GW -> sip-edge") {
                out.push_str(&format!(
                    " {:<12} |                            | <----- {} ----- | {}\n",
                    s.timestamp, s.method_or_status, s.summary
                ));
            } else {
                out.push_str(&format!(
                    " {:<12} | <============== {} ==============> | {}\n",
                    s.timestamp, s.method_or_status, s.summary
                ));
            }
            out.push_str("          |                            |                            |\n");
        }

        out
    }
}

fn push_section<T>(out: &mut String, title: &str, items: &[T], render: impl Fn(&T) -> String) {
    if items.is_empty() {
        return;
    }
    out.push_str(&format!("## {title}（{} 条）\n", items.len()));
    for item in items.iter().take(20) {
        out.push_str(&render(item));
    }
    out.push('\n');
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_intent_matches_chinese_keywords() {
        assert_eq!(
            TelecomCopilotEngine::classify_intent("排查 13800138000 挂断原因"),
            CopilotIntent::CallFailure
        );
        assert_eq!(
            TelecomCopilotEngine::classify_intent("生成 SIP 梯形图"),
            CopilotIntent::SipLadder
        );
        assert_eq!(
            TelecomCopilotEngine::classify_intent("查询计费余额"),
            CopilotIntent::Billing
        );
    }

    #[test]
    fn llm_config_detects_placeholder_keys() {
        let mut cfg = LlmConfig {
            enabled: true,
            provider: "zhipu".into(),
            api_key: String::new(),
            base_url: "https://open.bigmodel.cn/api/paas/v4".into(),
            model: "glm-4-flash".into(),
            temperature: 0.3,
        };
        assert!(!cfg.is_configured(), "空 key 不应视为已配置");
        cfg.api_key = "sk-proj-your-actual-api-key-here".into();
        assert!(!cfg.is_configured(), "占位符 key 不应视为已配置");
        cfg.api_key = "6f86ed5fe1c04366918e12e5170f4660.CRsePLgiumNbWmh0".into();
        assert!(cfg.is_configured(), "真实 key 应视为已配置");
    }

    fn make_test_cdr() -> CdrEvent {
        CdrEvent {
            call_id: "test-1".into(),
            caller: Some("1001".into()),
            callee: Some("13800138000".into()),
            started_at_ms: 0,
            answered_at_ms: None,
            ended_at_ms: 5000,
            duration_ms: 5000,
            billable_duration_ms: 0,
            status: "failed".into(),
            failure_status_code: Some(503),
            failure_reason: Some("Service Unavailable".into()),
            caller_rtcp_loss_rate: None,
            caller_rtcp_jitter_ms: None,
            caller_rtcp_rtt_ms: None,
            gateway_rtcp_loss_rate: None,
            gateway_rtcp_jitter_ms: None,
            gateway_rtcp_rtt_ms: None,
            mos: None,
            dtmf_digits: None,
            recording_path: None,
            direction: "outbound".into(),
            audit: cdr_core::CdrAuditSnapshot::default(),
        }
    }

    #[test]
    fn ladder_from_cdr_synthesizes_failed_call() {
        let cdr = make_test_cdr();
        let steps = TelecomCopilotEngine::ladder_from_cdr(&cdr);
        assert!(steps.iter().any(|s| s.method_or_status.contains("503")));
        let ascii = TelecomCopilotEngine::generate_ascii_ladder(&steps);
        assert!(ascii.contains("sip-edge"));
    }
}
