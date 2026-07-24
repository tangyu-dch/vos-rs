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

pub(crate) mod history;
mod ladder;
mod schema;
pub(crate) mod stream;
mod tools;

pub use ladder::{generate_ascii_ladder, ladder_from_cdr, ladder_from_flows, SipLadderStep};
pub use schema::get_copilot_tools_schema;

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
    pub(crate) state: &'a AppState,
    pub(crate) llm: Option<LlmConfig>,
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
        let ascii_ladder = generate_ascii_ladder(&steps);
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
            || q.contains("通话")
            || q.contains("记录")
            || q.contains("cdr")
            || q.contains("呼叫")
            || q.contains("最新")
            || q.contains("详情")
            || q.contains("call")
            || q.contains("record")
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
                // 1. 总是获取绝对最新的通话记录（不论成功与否）
                if let Ok((latest, _)) = self
                    .state
                    .store
                    .list_cdrs(1, 1, None, None, None, None, None, None, None)
                    .await
                {
                    payload.latest_cdr = latest.into_iter().next();
                }

                // 2. 获取最近 10 条释放异常（失败）的通话记录
                if let Ok((failed, _)) = self
                    .state
                    .store
                    .list_cdrs(1, 10, Some("failed"), None, None, None, None, None, None)
                    .await
                {
                    payload.recent_failed_cdrs = failed;
                }

                // 3. 关联获取最新通话记录的真实 SIP 信令流抓包
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
                return ladder_from_flows(&payload.sip_flows, cdr.started_at_ms);
            }
        }
        if let Some(cdr) = &payload.latest_cdr {
            return ladder_from_cdr(cdr);
        }
        Vec::new()
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
                "content": "你是 vos-rs 电信级 VoIP 软交换平台的智能运维专家 Copilot。你的任务是基于我提供的真实业务数据（JSON）以及可调用的工具（Tools），协助用户进行高效的运维排障、性能分析或系统管理。\n\n回答与工具调用要求：\n1. **智能数据整理与格式清洗 (AI Reformatting)**：当用户以不规整的自然语言、微信聊天文本、非标准表格或杂乱文本粘贴数据（如分机开户、网关列表、路由规则、资费表等）要求导入时，你必须发挥 AI 智能分析能力，提取出其中的有效字段，将其整理清洗为标准的 CSV 结构文本（如分机: `username,password`，网关: `id,name,ip_address,port`，路由: `id,prefix,gateway_id,priority`，资费: `prefix,rate_per_minute`），然后将清洗好的 CSV 文本传入对应的导入工具 (`vos_import_extensions`, `vos_import_gateways`, `vos_import_routes`, `vos_import_rates`) 进行精准执行！并在回复中向用户展示你清洗好的标准表格明细。\n2. **智能冲突与重复检测**：在进行路由创建、分机开户、网关绑定或 IVR 配置时，如工具返回了 `conflict: true` 冲突警告或目标关联不存在（如路由的目标网关不存在、DID 重复绑定、分机号重复等），必须在回复中明确指出具体的冲突点与原因，并主动给出可替代的解决建议方案（例如建议更换 ID、先创建网关、或使用不同的前缀）。\n3. **排版规范与美观**：使用清晰的 Markdown 结构。必须包含以下二级标题：\n   - ## 📊 分析与处理报告 (Report)：结合数据对当前系统状态、业务配置或数据导入/导出结果进行专业解读。\n   - ## 🔍 数据清洗与整理明细 (Cleaned Records)：展示提取清洗后的标准结构表格。\n   - ## 💡 建议动作 (Suggested Action)：给出具体、可执行的操作指引。\n4. **生动自然**：语气要专业、自然，像一个资深的 VoIP 架构师在与同事交流。"
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
            .header("Accept-Encoding", "identity")
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("HTTP 请求失败 (无法连接目标域名 {}, 请检查网络/代理/APIKey): {e}", llm.base_url))?;
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
            .map(|s| s.to_string())
            .ok_or_else(|| "LLM 未返回有效内容".into())
    }
}

pub(crate) fn push_section<T>(out: &mut String, title: &str, items: &[T], render: impl Fn(&T) -> String) {
    if items.is_empty() {
        return;
    }
    out.push_str(&format!("## {title}（{} 条）\n", items.len()));
    for item in items.iter().take(20) {
        out.push_str(&render(item));
    }
    out.push('\n');
}

pub(crate) fn truncate(s: &str, max: usize) -> String {
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
        let steps = ladder_from_cdr(&cdr);
        assert!(steps.iter().any(|s| s.method_or_status.contains("503")));
        let ascii = generate_ascii_ladder(&steps);
        assert!(ascii.contains("sip-edge"));
    }
}
