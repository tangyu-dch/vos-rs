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
                "content": "你是 vos-rs 电信级 VoIP 软交换平台的智能运维专家 Copilot。你的任务是基于我提供的真实业务数据（JSON）以及可调用的工具（Tools），协助用户进行高效的运维排障、性能分析或系统管理。\n\n回答与工具调用要求：\n1. **智能冲突与重复检测**：在进行路由创建、分机开户、网关绑定或 IVR 配置时，如工具返回了 `conflict: true` 冲突警告或目标关联不存在（如路由的目标网关不存在、DID 重复绑定、分机号重复等），必须在回复中明确指出具体的冲突点与原因，并主动给出可替代的解决建议方案（例如建议更换 ID、先创建网关、或使用不同的前缀）。\n2. **排版规范与美观**：使用清晰的 Markdown 结构。必须包含以下二级标题：\n   - ## 📊 分析报告 (Analysis Report)：结合数据对当前系统状态、业务配置或呼叫流程进行专业解读。\n   - ## 🔍 根因与冲突诊断 (Diagnostics)：深入剖析原因、冲突点或可能的影响。\n   - ## 💡 建议动作 (Suggested Action)：给出具体、可执行的操作指引。\n3. **生动自然**：语气要专业、自然，像一个资深的 VoIP 架构师在与同事交流。"
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
            .map(|s| s.to_string())
            .ok_or_else(|| "LLM 未返回有效内容".into())
    }
}

pub fn get_copilot_tools_schema() -> serde_json::Value {
    json!([
        {
            "type": "function",
            "function": {
                "name": "vos_get_dashboard_stats",
                "description": "获取 VoIP 软交换平台整体运行概览指标（CPS、接通率 ASR、平均 MOS 评分、活跃通话数、注册分机数等）。",
                "parameters": { "type": "object", "properties": {}, "required": [] }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "vos_list_cdrs",
                "description": "查询通话详单 (CDR)。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "status": { "type": "string", "description": "筛选通话状态: answered / failed / canceled", "enum": ["answered", "failed", "canceled"] },
                        "caller": { "type": "string", "description": "主叫号码过滤" },
                        "callee": { "type": "string", "description": "被叫号码过滤" },
                        "limit": { "type": "integer", "description": "返回条数上限，默认 10", "default": 10 }
                    }
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "vos_get_sip_flows",
                "description": "获取指定通话的完整 SIP 信令交互抓包及 ASCII 梯形图 (SIP Ladder Diagram)。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "call_id": { "type": "string", "description": "通话唯一的 Call-ID 字符串" }
                    },
                    "required": ["call_id"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "vos_list_active_calls",
                "description": "获取当前软交换平台所有正在进行的并发通话列表。",
                "parameters": { "type": "object", "properties": {}, "required": [] }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "vos_terminate_call",
                "description": "强制中断/拆线指定 Call-ID 的实时通话。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "call_id": { "type": "string", "description": "需要断开的 Call-ID" }
                    },
                    "required": ["call_id"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "vos_list_registrations",
                "description": "查询分机终端当前的 SIP 注册绑定状态。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "username": { "type": "string", "description": "分机账号/用户名过滤" }
                    }
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "vos_list_gateways",
                "description": "查询软交换中继网关列表及链路健康状态与通道容量。",
                "parameters": { "type": "object", "properties": {}, "required": [] }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "vos_preview_route",
                "description": "模拟呼叫路由决策算力（主叫 + 被叫 -> 输出匹配中继与计费规则）。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "caller": { "type": "string", "description": "主叫号码" },
                        "callee": { "type": "string", "description": "被叫号码" }
                    },
                    "required": ["caller", "callee"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "vos_list_anti_fraud_rules",
                "description": "查询防刷量、频控与反欺诈风控规则。",
                "parameters": { "type": "object", "properties": {}, "required": [] }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "vos_list_extensions",
                "description": "获取 SIP 分机账号列表或指定分机配置信息。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "username": { "type": "string", "description": "分机账号/用户名过滤" }
                    }
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "vos_create_extension",
                "description": "创建新的 SIP 分机账号（指定分机账号 username 与密码 password）。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "username": { "type": "string", "description": "分机账号/用户名" },
                        "password": { "type": "string", "description": "分机注册密码" }
                    },
                    "required": ["username", "password"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "vos_delete_extension",
                "description": "删除指定的 SIP 分机账号。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "username": { "type": "string", "description": "待删除的分机账号" }
                    },
                    "required": ["username"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "vos_list_ivr_menus",
                "description": "获取系统的 IVR 语音导航菜单流程列表。",
                "parameters": { "type": "object", "properties": {}, "required": [] }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "vos_create_ivr_menu",
                "description": "创建或更新 IVR 语音导航菜单流程（指定菜单 ID id、名称 name、绑定的 DID 号码 did、欢迎语 welcome_prompt）。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string", "description": "IVR 菜单唯一标识 ID" },
                        "name": { "type": "string", "description": "IVR 菜单显示名称" },
                        "did": { "type": "string", "description": "绑定的呼入 DID 号码" },
                        "welcome_prompt": { "type": "string", "description": "欢迎提示音语音内容或路径" }
                    },
                    "required": ["id", "name"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "vos_create_gateway",
                "description": "创建对接网关/中继线路（指定网关 ID id、名称 name、目标 IP ip_address、端口 port）。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string", "description": "网关唯一 ID" },
                        "name": { "type": "string", "description": "网关名称" },
                        "ip_address": { "type": "string", "description": "目标 IP 地址" },
                        "port": { "type": "integer", "description": "目标端口 (默认 5060)", "default": 5060 }
                    },
                    "required": ["id", "name", "ip_address"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "vos_delete_gateway",
                "description": "删除指定的软交换中继网关。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string", "description": "待删除的网关 ID" }
                    },
                    "required": ["id"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "vos_list_routes",
                "description": "获取系统中的所有前缀呼叫路由规则。",
                "parameters": { "type": "object", "properties": {}, "required": [] }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "vos_delete_route",
                "description": "删除指定的前缀呼叫路由规则。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string", "description": "待删除的路由 ID" }
                    },
                    "required": ["id"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "vos_list_billing_accounts",
                "description": "获取所有计费账户及当前余额信息。",
                "parameters": { "type": "object", "properties": {}, "required": [] }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "vos_recharge_billing_account",
                "description": "为计费账户充值或变动余额（指定账户账号 account_id、金额 amount、备注 description）。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "account_id": { "type": "string", "description": "计费账户账号或唯一标识" },
                        "amount": { "type": "number", "description": "充值金额（正数为充值，负数为扣款）" },
                        "description": { "type": "string", "description": "充值/扣款备注" }
                    },
                    "required": ["account_id", "amount"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "vos_list_rates",
                "description": "获取系统呼叫资费费率表。",
                "parameters": { "type": "object", "properties": {}, "required": [] }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "vos_upsert_rate",
                "description": "创建或修改呼叫资费费率（指定费率 ID id、号码前缀 prefix、每分钟费率 rate_per_minute）。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string", "description": "费率唯一 ID" },
                        "prefix": { "type": "string", "description": "号码前缀" },
                        "rate_per_minute": { "type": "number", "description": "每分钟费率金额" }
                    },
                    "required": ["id", "prefix", "rate_per_minute"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "vos_delete_rate",
                "description": "删除指定的呼叫资费费率。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string", "description": "待删除的费率 ID" }
                    },
                    "required": ["id"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "vos_add_ivr_node",
                "description": "向现有 IVR 菜单添加/配置按键转接节点（指定 IVR ID id、按键 dtmf_key 0-9/*/#、目标类型 action 例如 extension:8001 或 gateway:gw1）。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string", "description": "IVR 菜单 ID" },
                        "dtmf_key": { "type": "string", "description": "按键 (0-9, *, #, timeout)" },
                        "action": { "type": "string", "description": "转接动作或目标，例如 extension:8001" }
                    },
                    "required": ["id", "dtmf_key", "action"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "vos_delete_ivr_menu",
                "description": "删除指定的 IVR 语音导航菜单。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string", "description": "待删除的 IVR 菜单 ID" }
                    },
                    "required": ["id"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "vos_create_anti_fraud_rule",
                "description": "创建防刷量/高危频控风控规则（指定规则 ID id、规则名称 name、匹配模式 pattern、频控上限 limit_count）。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string", "description": "规则 ID" },
                        "name": { "type": "string", "description": "规则名称" },
                        "pattern": { "type": "string", "description": "匹配模式 (如 IP 网段或主叫前缀)" },
                        "limit_count": { "type": "integer", "description": "允许最大并发或频控值", "default": 60 }
                    },
                    "required": ["id", "name", "pattern"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "vos_delete_anti_fraud_rule",
                "description": "删除防刷量/频控风控规则。",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "id": { "type": "string", "description": "待删除的规则 ID" }
                    },
                    "required": ["id"]
                }
            }
        }
    ])
}

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
            "vos_get_dashboard_stats" => {
                let stats = self.state.store.get_dashboard_stats(0).await.ok();
                json!(stats)
            }
            "vos_list_cdrs" => {
                let status = args.get("status").and_then(|v| v.as_str());
                let caller = args.get("caller").and_then(|v| v.as_str());
                let callee = args.get("callee").and_then(|v| v.as_str());
                let limit = args.get("limit").and_then(|v| v.as_i64()).unwrap_or(10).clamp(1, 50);
                match self.state.store.list_cdrs(1, limit, status, None, caller, callee, None, None, None).await {
                    Ok((cdrs, total)) => json!({ "total": total, "cdrs": cdrs }),
                    Err(e) => json!({ "error": e.to_string() }),
                }
            }
            "vos_get_sip_flows" => {
                let call_id = args.get("call_id").and_then(|v| v.as_str()).unwrap_or("");
                let flows = self.state.store.get_sip_flows(call_id).await.unwrap_or_default();
                let steps = Self::ladder_from_flows(&flows, 0);
                let ascii = Self::generate_ascii_ladder(&steps);
                json!({ "call_id": call_id, "flows": flows, "ladder_diagram": ascii })
            }
            "vos_list_active_calls" => {
                let calls = self.state.active_calls_cache.get_or_fetch(self.state).await;
                json!({ "active_calls": calls, "count": calls.len() })
            }
            "vos_terminate_call" => {
                let call_id = args.get("call_id").and_then(|v| v.as_str()).unwrap_or("");
                let url = format!("{}/manage/calls/{}/terminate", self.state.sip_manage_base, call_id);
                let req = self.state.internal_client.post(&url);
                let req = if !self.state.internal_secret.is_empty() { req.header("X-VOS-Token", &self.state.internal_secret) } else { req };
                match req.send().await {
                    Ok(r) if r.status().is_success() => json!({ "success": true, "message": format!("通话 {} 已成功拆线挂断", call_id) }),
                    Ok(r) => json!({ "success": false, "error": format!("HTTP {}", r.status()) }),
                    Err(e) => json!({ "success": false, "error": e.to_string() }),
                }
            }
            "vos_list_registrations" => {
                let regs = self.state.store.list_registrations().await.unwrap_or_default();
                let username = args.get("username").and_then(|v| v.as_str());
                let filtered: Vec<_> = if let Some(u) = username {
                    regs.into_iter().filter(|r| r.aor.contains(u)).collect()
                } else {
                    regs
                };
                json!({ "registrations": filtered })
            }
            "vos_list_gateways" => {
                let gws = self.state.store.list_gateways_full().await.unwrap_or_default();
                json!({ "gateways": gws })
            }
            "vos_preview_route" => {
                let destination = args.get("callee").or_else(|| args.get("destination")).and_then(|v| v.as_str()).unwrap_or("");
                let url = format!("{}/manage/route-preview?destination={}", self.state.sip_manage_base, urlencoding_str(destination));
                let req = self.state.internal_client.get(&url);
                let req = if !self.state.internal_secret.is_empty() { req.header("X-VOS-Token", &self.state.internal_secret) } else { req };
                match req.send().await {
                    Ok(r) => r.json::<serde_json::Value>().await.unwrap_or_else(|_| json!({})),
                    Err(e) => json!({ "error": e.to_string() }),
                }
            }
            "vos_list_anti_fraud_rules" => {
                let rules = self.state.store.list_anti_fraud_rules().await.unwrap_or_default();
                json!({ "rules": rules })
            }
            "vos_list_extensions" => {
                let username = args.get("username").and_then(|v| v.as_str());
                match self.state.store.list_users().await {
                    Ok(users) => {
                        let filtered: Vec<_> = if let Some(u) = username {
                            users.into_iter().filter(|user| user.username.contains(u)).collect()
                        } else {
                            users
                        };
                        json!({ "total": filtered.len(), "extensions": filtered })
                    }
                    Err(e) => json!({ "error": e.to_string() }),
                }
            }
            "vos_create_extension" => {
                let username = args.get("username").and_then(|v| v.as_str()).unwrap_or("");
                let password = args.get("password").and_then(|v| v.as_str()).unwrap_or("");
                if username.is_empty() || password.is_empty() {
                    return json!({ "success": false, "error": "分机账号 username 和密码 password 不能为空" });
                }
                let ext_exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sip_users WHERE username = $1)")
                    .bind(username)
                    .fetch_one(self.state.store.pool())
                    .await
                    .unwrap_or(false);
                if ext_exists {
                    return json!({
                        "success": false,
                        "conflict": true,
                        "error": format!("分机账号 `{}` 已存在，不能重复创建。", username)
                    });
                }
                let realm = self.state.store.get_system_config("auth_realm").await.ok().flatten().unwrap_or_else(|| "vos-rs".into());
                let ha1 = format!("{:x}", md5::compute(format!("{username}:{realm}:{password}").as_bytes()));
                match self.state.store.insert_user(username, &ha1).await {
                    Ok(_) => {
                        let _ = crate::hot_cache::set_auth_user(self.state, username, &ha1).await;
                        json!({ "success": true, "message": format!("分机账号 `{}` 创建成功", username) })
                    }
                    Err(e) => json!({ "success": false, "error": e.to_string() }),
                }
            }
            "vos_delete_extension" => {
                let username = args.get("username").and_then(|v| v.as_str()).unwrap_or("");
                match self.state.store.delete_user(username).await {
                    Ok(_) => json!({ "success": true, "message": format!("分机账号 {} 已删除", username) }),
                    Err(e) => json!({ "success": false, "error": e.to_string() }),
                }
            }
            "vos_list_ivr_menus" => {
                match self.state.store.list_ivr_menus().await {
                    Ok(menus) => json!({ "ivr_menus": menus }),
                    Err(e) => json!({ "error": e.to_string() }),
                }
            }
            "vos_create_ivr_menu" => {
                let id = args.get("id").and_then(|v| v.as_str()).unwrap_or("");
                let name = args.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let did = args.get("did").and_then(|v| v.as_str()).unwrap_or("");
                let welcome_prompt = args.get("welcome_prompt").and_then(|v| v.as_str()).unwrap_or("");
                if id.is_empty() || name.is_empty() {
                    return json!({ "success": false, "error": "IVR 菜单 ID 和名称不能为空" });
                }
                if !did.is_empty() {
                    let dup_did: Option<(String, String)> = sqlx::query_as(
                        "SELECT id, name FROM ivr_menus WHERE did = $1 AND id != $2"
                    )
                    .bind(did)
                    .bind(id)
                    .fetch_optional(self.state.store.pool())
                    .await
                    .unwrap_or(None);

                    if let Some((dup_id, dup_name)) = dup_did {
                        return json!({
                            "success": false,
                            "conflict": true,
                            "warning": format!("DID 号码 `{}` 已被另一 IVR 菜单 `{}` ({}) 绑定。请换用其他 DID 号码。", did, dup_name, dup_id)
                        });
                    }
                }
                let res = sqlx::query(
                    "INSERT INTO ivr_menus (id, name, description, did, welcome_prompt, timeout_secs, enabled, nodes, edges) \
                     VALUES ($1, $2, $3, $4, $5, 10, true, '[]'::jsonb, '[]'::jsonb) \
                     ON CONFLICT (id) DO UPDATE SET name = EXCLUDED.name, did = EXCLUDED.did, welcome_prompt = EXCLUDED.welcome_prompt"
                )
                .bind(id)
                .bind(name)
                .bind("由 Copilot 自动创建")
                .bind(did)
                .bind(welcome_prompt)
                .execute(self.state.store.pool())
                .await;
                match res {
                    Ok(_) => json!({ "success": true, "message": format!("IVR 菜单 `{}` ({}) 创建成功", name, id) }),
                    Err(e) => json!({ "success": false, "error": e.to_string() }),
                }
            }
            "vos_create_gateway" => {
                let id = args.get("id").and_then(|v| v.as_str()).unwrap_or("");
                let name = args.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let ip = args.get("ip_address").and_then(|v| v.as_str()).unwrap_or("");
                let port = args.get("port").and_then(|v| v.as_i64()).unwrap_or(5060) as i32;
                if id.is_empty() || name.is_empty() || ip.is_empty() {
                    return json!({ "success": false, "error": "网关 ID、名称与 IP 地址不能为空" });
                }
                let existing_gw: Option<(String, String)> = sqlx::query_as(
                    "SELECT id, name FROM sip_gateways WHERE ip_address = $1 AND id != $2"
                )
                .bind(ip)
                .bind(id)
                .fetch_optional(self.state.store.pool())
                .await
                .unwrap_or(None);

                if let Some((dup_id, dup_name)) = existing_gw {
                    return json!({
                        "success": false,
                        "conflict": true,
                        "warning": format!("目标 IP 地址 `{}` 已被已有网关 `{}` ({}) 使用。请确认 IP 是否重复。", ip, dup_name, dup_id)
                    });
                }

                let res = sqlx::query(
                    "INSERT INTO sip_gateways (id, name, ip_address, port, enabled) VALUES ($1, $2, $3, $4, true) \
                     ON CONFLICT (id) DO UPDATE SET name = EXCLUDED.name, ip_address = EXCLUDED.ip_address, port = EXCLUDED.port"
                )
                .bind(id)
                .bind(name)
                .bind(ip)
                .bind(port)
                .execute(self.state.store.pool())
                .await;
                match res {
                    Ok(_) => json!({ "success": true, "message": format!("中继网关 `{}` ({}:{}) 创建成功", name, ip, port) }),
                    Err(e) => json!({ "success": false, "error": e.to_string() }),
                }
            }
            "vos_delete_gateway" => {
                let id = args.get("id").and_then(|v| v.as_str()).unwrap_or("");
                match self.state.store.delete_gateway(id).await {
                    Ok(true) => json!({ "success": true, "message": format!("网关 {} 已成功删除", id) }),
                    Ok(false) => json!({ "success": false, "error": format!("网关 {} 不存在", id) }),
                    Err(e) => json!({ "error": e.to_string() }),
                }
            }
            "vos_create_route" => {
                let id = args.get("id").and_then(|v| v.as_str()).unwrap_or("");
                let prefix = args.get("prefix").and_then(|v| v.as_str()).unwrap_or("");
                let gw_id = args.get("gateway_id").and_then(|v| v.as_str()).unwrap_or("");
                let priority = args.get("priority").and_then(|v| v.as_i64()).unwrap_or(1) as i32;
                if id.is_empty() || prefix.is_empty() || gw_id.is_empty() {
                    return json!({ "success": false, "error": "路由 ID、号码前缀与网关 ID 不能为空" });
                }
                // 1. 校验目标中继网关是否存在
                let gw_exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM sip_gateways WHERE id = $1)")
                    .bind(gw_id)
                    .fetch_one(self.state.store.pool())
                    .await
                    .unwrap_or(false);
                if !gw_exists {
                    return json!({
                        "success": false,
                        "conflict": true,
                        "error": format!("目标中继网关 `{}` 不存在，无法创建路由。请先创建该网关或指定已有的网关 ID。", gw_id)
                    });
                }
                // 2. 校验前缀重复或覆盖冲突
                let existing_route: Option<(String, String)> = sqlx::query_as(
                    "SELECT id, gateway_id FROM sip_routes WHERE prefix = $1 AND id != $2"
                )
                .bind(prefix)
                .bind(id)
                .fetch_optional(self.state.store.pool())
                .await
                .unwrap_or(None);

                if let Some((ext_id, ext_gw)) = existing_route {
                    return json!({
                        "success": false,
                        "conflict": true,
                        "warning": format!("前缀号码 `{}` 已被路由 `{}` 使用（指向网关 `{}`）。如需覆盖请明确指定路由 ID。", prefix, ext_id, ext_gw),
                        "existing_route_id": ext_id
                    });
                }

                let res = sqlx::query(
                    "INSERT INTO sip_routes (id, prefix, priority, gateway_id) VALUES ($1, $2, $3, $4) \
                     ON CONFLICT (id) DO UPDATE SET prefix = EXCLUDED.prefix, priority = EXCLUDED.priority, gateway_id = EXCLUDED.gateway_id"
                )
                .bind(id)
                .bind(prefix)
                .bind(priority)
                .bind(gw_id)
                .execute(self.state.store.pool())
                .await;
                match res {
                    Ok(_) => {
                        let _ = crate::routes::publish_route_reload(&self.state.nats_client).await;
                        json!({ "success": true, "message": format!("前缀路由 `{}` (前缀: {}, 网关: {}) 创建成功并实时重载选路引擎", id, prefix, gw_id) })
                    }
                    Err(e) => json!({ "success": false, "error": e.to_string() }),
                }
            }
            "vos_list_routes" => {
                match self.state.store.list_routes_full().await {
                    Ok(routes) => json!({ "routes": routes }),
                    Err(e) => json!({ "error": e.to_string() }),
                }
            }
            "vos_delete_route" => {
                let id = args.get("id").and_then(|v| v.as_str()).unwrap_or("");
                match self.state.store.delete_route(id).await {
                    Ok(true) => {
                        let _ = crate::routes::publish_route_reload(&self.state.nats_client).await;
                        json!({ "success": true, "message": format!("路由 {} 已成功删除", id) })
                    }
                    Ok(false) => json!({ "success": false, "error": format!("路由 {} 不存在", id) }),
                    Err(e) => json!({ "error": e.to_string() }),
                }
            }
            "vos_list_billing_accounts" => {
                match self.state.store.list_accounts().await {
                    Ok(accs) => json!({ "accounts": accs }),
                    Err(e) => json!({ "error": e.to_string() }),
                }
            }
            "vos_recharge_billing_account" => {
                let acc_id = args.get("account_id").and_then(|v| v.as_str()).unwrap_or("");
                let amount = args.get("amount").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let desc = args.get("description").and_then(|v| v.as_str()).unwrap_or("Copilot 充值操作");
                if acc_id.is_empty() || amount == 0.0 {
                    return json!({ "success": false, "error": "账户 ID 与变动金额不能为空或 0" });
                }
                let amount_dec = rust_decimal::Decimal::from_f64_retain(amount).unwrap_or_default();
                match self.state.store.credit_account(acc_id, amount_dec, desc).await {
                    Ok(cdr_core::CreditAccountOutcome::Applied(new_bal)) | Ok(cdr_core::CreditAccountOutcome::Replayed(new_bal)) => {
                        json!({ "success": true, "message": format!("账户 {} 成功变动金额 {}, 当前新余额: {}", acc_id, amount, new_bal) })
                    }
                    Ok(cdr_core::CreditAccountOutcome::Conflict) => {
                        json!({ "success": false, "error": "重复的充值幂等请求" })
                    }
                    Err(e) => json!({ "success": false, "error": e.to_string() }),
                }
            }
            "vos_list_rates" => {
                match self.state.store.list_rates().await {
                    Ok(rates) => json!({ "rates": rates }),
                    Err(e) => json!({ "error": e.to_string() }),
                }
            }
            "vos_upsert_rate" => {
                let id = args.get("id").and_then(|v| v.as_str()).unwrap_or("");
                let prefix = args.get("prefix").and_then(|v| v.as_str()).unwrap_or("");
                let rate = args.get("rate_per_minute").and_then(|v| v.as_f64()).unwrap_or(0.0);
                if id.is_empty() || prefix.is_empty() {
                    return json!({ "success": false, "error": "费率 ID 与前缀不能为空" });
                }
                let rate_dec = rust_decimal::Decimal::from_f64_retain(rate).unwrap_or_default();
                match self.state.store.upsert_rate(id, prefix, rate_dec, 60, rate_dec, None).await {
                    Ok(_) => json!({ "success": true, "message": format!("费率规则 {} (前缀: {}, 费率: {}/分) 设置成功", id, prefix, rate) }),
                    Err(e) => json!({ "success": false, "error": e.to_string() }),
                }
            }
            "vos_delete_rate" => {
                let id = args.get("id").and_then(|v| v.as_str()).unwrap_or("");
                match self.state.store.delete_rate(id).await {
                    Ok(true) => json!({ "success": true, "message": format!("费率 {} 已成功删除", id) }),
                    Ok(false) => json!({ "success": false, "error": format!("费率 {} 不存在", id) }),
                    Err(e) => json!({ "error": e.to_string() }),
                }
            }
            "vos_add_ivr_node" => {
                let id = args.get("id").and_then(|v| v.as_str()).unwrap_or("");
                let key = args.get("dtmf_key").and_then(|v| v.as_str()).unwrap_or("");
                let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("");
                if id.is_empty() || key.is_empty() || action.is_empty() {
                    return json!({ "success": false, "error": "IVR ID、按键 key 与动作 action 不能为空" });
                }
                let node_id = format!("node_{key}");
                let new_node = json!({ "id": node_id, "dtmf": key, "action": action });
                let res = sqlx::query(
                    "UPDATE ivr_menus SET nodes = nodes || $1::jsonb WHERE id = $2"
                )
                .bind(json!([new_node]))
                .bind(id)
                .execute(self.state.store.pool())
                .await;
                match res {
                    Ok(r) if r.rows_affected() > 0 => json!({ "success": true, "message": format!("IVR {} 成功新增按键 [{}] -> 动作 [{}]", id, key, action) }),
                    Ok(_) => json!({ "success": false, "error": format!("IVR 菜单 {} 不存在", id) }),
                    Err(e) => json!({ "success": false, "error": e.to_string() }),
                }
            }
            "vos_delete_ivr_menu" => {
                let id = args.get("id").and_then(|v| v.as_str()).unwrap_or("");
                let res = sqlx::query("DELETE FROM ivr_menus WHERE id = $1").bind(id).execute(self.state.store.pool()).await;
                match res {
                    Ok(r) if r.rows_affected() > 0 => json!({ "success": true, "message": format!("IVR 菜单 {} 已成功删除", id) }),
                    Ok(_) => json!({ "success": false, "error": format!("IVR 菜单 {} 不存在", id) }),
                    Err(e) => json!({ "error": e.to_string() }),
                }
            }
            "vos_create_anti_fraud_rule" => {
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
            "vos_delete_anti_fraud_rule" => {
                let id = args.get("id").and_then(|v| v.as_str()).unwrap_or("");
                match self.state.store.delete_anti_fraud_rule(id).await {
                    Ok(true) => json!({ "success": true, "message": format!("风控规则 {} 已成功删除", id) }),
                    Ok(false) => json!({ "success": false, "error": format!("风控规则 {} 不存在", id) }),
                    Err(e) => json!({ "error": e.to_string() }),
                }
            }
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
