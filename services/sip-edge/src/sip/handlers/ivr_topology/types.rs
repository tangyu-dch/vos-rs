//! IVR 拓扑数据结构定义
//!
//! 与前端 `web/src/components/ivr/types.ts` 拓扑画布模型对齐。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// IVR 节点类型 (与前端 web/src/components/ivr/types.ts 对齐)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum IvrNodeType {
    Start,
    Prompt,
    Tts,
    CollectDtmf,
    Menu,
    Condition,
    Route,
    TransferQueue,
    TransferExt,
    TransferPstn,
    Voicemail,
    Record,
    HttpWebhook,
    SetVar,
    Asr,
    AiAgent,
    Loop,
    Hangup,
}

impl IvrNodeType {
    /// 从字符串解析节点类型（snake_case）
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "start" => Some(Self::Start),
            "prompt" => Some(Self::Prompt),
            "tts" => Some(Self::Tts),
            "collect_dtmf" => Some(Self::CollectDtmf),
            "menu" => Some(Self::Menu),
            "condition" => Some(Self::Condition),
            "route" => Some(Self::Route),
            "transfer_queue" => Some(Self::TransferQueue),
            "transfer_ext" => Some(Self::TransferExt),
            "transfer_pstn" => Some(Self::TransferPstn),
            "voicemail" => Some(Self::Voicemail),
            "record" => Some(Self::Record),
            "http_webhook" => Some(Self::HttpWebhook),
            "set_var" => Some(Self::SetVar),
            "asr" => Some(Self::Asr),
            "ai_agent" => Some(Self::AiAgent),
            "loop" => Some(Self::Loop),
            "hangup" => Some(Self::Hangup),
            _ => None,
        }
    }
}

/// 画布坐标
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    pub x: f64,
    pub y: f64,
}

/// 拓扑节点
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopologyNode {
    pub id: String,
    #[serde(rename = "type")]
    pub node_type: String,
    pub title: String,
    pub description: Option<String>,
    pub position: Option<Position>,
    pub config: serde_json::Value,
}

/// 拓扑边
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopologyEdge {
    pub id: String,
    pub source: String,
    pub target: String,
    pub source_port: Option<String>,
    pub label: Option<String>,
}

/// 完整拓扑图
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IvrTopology {
    pub nodes: Vec<TopologyNode>,
    pub edges: Vec<TopologyEdge>,
}

/// 图索引：加速节点查找和边遍历
#[derive(Debug, Clone)]
pub struct TopologyGraph {
    /// node_id -> node
    pub nodes: HashMap<String, TopologyNode>,
    /// (source_node_id, source_port) -> target_node_id
    pub edges_by_source: HashMap<(String, String), String>,
    /// source_node_id -> [(source_port, target_node_id)]
    pub outgoing: HashMap<String, Vec<(String, String)>>,
    /// start 节点 id
    pub start_node_id: Option<String>,
}

impl TopologyGraph {
    /// 从拓扑数据构建图索引
    pub fn build(topology: &IvrTopology) -> Self {
        let mut graph = Self {
            nodes: HashMap::new(),
            edges_by_source: HashMap::new(),
            outgoing: HashMap::new(),
            start_node_id: None,
        };
        for node in &topology.nodes {
            if node.node_type == "start" && graph.start_node_id.is_none() {
                graph.start_node_id = Some(node.id.clone());
            }
            graph.nodes.insert(node.id.clone(), node.clone());
        }
        for edge in &topology.edges {
            let port = edge
                .source_port
                .clone()
                .unwrap_or_else(|| "default".to_string());
            graph
                .edges_by_source
                .insert((edge.source.clone(), port.clone()), edge.target.clone());
            graph
                .outgoing
                .entry(edge.source.clone())
                .or_default()
                .push((port, edge.target.clone()));
        }
        graph
    }

    /// 按 id 查找节点
    pub fn get_node(&self, id: &str) -> Option<&TopologyNode> {
        self.nodes.get(id)
    }

    /// 按 source_port 查找下一个节点
    pub fn next_node(&self, source_id: &str, port: &str) -> Option<&TopologyNode> {
        let key = (source_id.to_string(), port.to_string());
        self.edges_by_source
            .get(&key)
            .and_then(|tid| self.nodes.get(tid))
    }

    /// 获取节点的所有出边
    pub fn outgoing_edges(&self, node_id: &str) -> Option<&Vec<(String, String)>> {
        self.outgoing.get(node_id)
    }
}

/// IVR 执行上下文 (per-call)
#[derive(Debug, Clone)]
pub struct IvrExecutionContext {
    /// 当前通话 Call-ID
    pub call_id: String,
    /// 主叫号码
    pub caller_id: String,
    /// 被叫号码 (DID)
    pub did: String,
    /// 上下文变量 (用于模板渲染与条件判断)
    pub variables: HashMap<String, serde_json::Value>,
    /// 当前正在执行的节点 id
    pub current_node_id: Option<String>,
    /// 循环计数器 (node_id -> 已迭代次数)
    pub loop_counters: HashMap<String, u32>,
    /// 最近收集到的 DTMF 字符串
    pub collected_dtmf: String,
    /// 最近一次 Webhook 响应
    pub last_webhook_response: Option<serde_json::Value>,
}

impl IvrExecutionContext {
    /// 创建新的执行上下文
    pub fn new(call_id: String, caller_id: String, did: String) -> Self {
        Self {
            call_id,
            caller_id,
            did,
            variables: HashMap::new(),
            current_node_id: None,
            loop_counters: HashMap::new(),
            collected_dtmf: String::new(),
            last_webhook_response: None,
        }
    }

    /// 设置上下文变量
    pub fn set_var(&mut self, key: &str, value: serde_json::Value) {
        self.variables.insert(key.to_string(), value);
    }

    /// 读取上下文变量
    pub fn get_var(&self, key: &str) -> Option<&serde_json::Value> {
        self.variables.get(key)
    }

    /// 渲染 `{{var}}` 模板占位符
    pub fn render_template(&self, template: &str) -> String {
        let mut result = template.to_string();
        for (k, v) in &self.variables {
            let placeholder = format!("{{{{{k}}}}}");
            let value_str = match v {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            result = result.replace(&placeholder, &value_str);
        }
        result
    }
}

/// 节点执行结果
#[derive(Debug, Clone)]
pub enum NodeExecuteResult {
    /// 跳转到指定端口 (如 "key_1", "match", "success", "default")
    Continue { port: String },
    /// 挂断呼叫
    Hangup { reason: String },
    /// 转接到外部目标 (extension/pstn/queue), 转接后 IVR 结束
    Transfer {
        target: String,
        transfer_type: String,
    },
    /// 等待 DTMF 输入 (用于 collect_dtmf / menu 节点)
    WaitForDtmf {
        max_digits: u8,
        timeout_secs: u32,
        terminator: Option<char>,
    },
    /// 等待 ASR 输入
    WaitForAsr { timeout_secs: u32 },
    /// 错误
    Error { message: String },
}
