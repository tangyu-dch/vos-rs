//! 流程控制节点执行器：condition / route / set_var / loop / http_webhook
//!
//! 这些节点负责拓扑图中的分支、跳转、变量操作与外部 HTTP 调用。

use super::super::types::*;
use tracing::{info, warn};

/// 默认 HTTP 调用超时秒数
const DEFAULT_WEBHOOK_TIMEOUT_SECS: u64 = 10;

/// 默认 loop 节点最大迭代次数
const DEFAULT_LOOP_MAX_ITERATIONS: u32 = 10;

/// 执行 condition 节点：根据变量比较结果选择 match / nomatch 端口
///
/// 读取配置：
/// - `variable`：待比较的变量名
/// - `operator`：运算符（eq/ne/gt/lt/contains/starts_with/ends_with）
/// - `value`：比较目标值
///
/// 返回 `Continue { port: "match" }` 或 `Continue { port: "nomatch" }`。
pub fn execute_condition(node: &TopologyNode, context: &IvrExecutionContext) -> NodeExecuteResult {
    let variable = node
        .config
        .get("variable")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let operator = node
        .config
        .get("operator")
        .and_then(|v| v.as_str())
        .unwrap_or("eq");
    let target_value = node.config.get("value");

    let actual = context.get_var(variable);
    let matched = match (actual, target_value) {
        (Some(actual), Some(target)) => compare_values(actual, target, operator),
        (None, Some(_)) => false,
        // 无目标值视为 always-match（用于"变量已定义即匹配"的语义）
        (Some(_), None) => true,
        (None, None) => false,
    };

    let port = if matched { "match" } else { "nomatch" };
    info!(
        call_id = %context.call_id,
        node_id = %node.id,
        variable,
        operator,
        matched,
        "IVR condition 节点"
    );
    NodeExecuteResult::Continue {
        port: port.to_string(),
    }
}

/// 比较两个 JSON 值
fn compare_values(actual: &serde_json::Value, target: &serde_json::Value, operator: &str) -> bool {
    match operator {
        "eq" => actual == target,
        "ne" => actual != target,
        "gt" => compare_ordered(actual, target).is_some_and(|o| o.is_gt()),
        "lt" => compare_ordered(actual, target).is_some_and(|o| o.is_lt()),
        "contains" => json_contains(actual, target),
        "starts_with" => json_str_op(actual, target, |a, b| a.starts_with(b)),
        "ends_with" => json_str_op(actual, target, |a, b| a.ends_with(b)),
        _ => {
            warn!(operator, "未知的 condition 运算符, 视为不匹配");
            false
        }
    }
}

/// 对数值/字符串进行有序比较
fn compare_ordered(
    actual: &serde_json::Value,
    target: &serde_json::Value,
) -> Option<std::cmp::Ordering> {
    use serde_json::Value;
    match (actual, target) {
        (Value::Number(a), Value::Number(b)) => {
            let av = a.as_f64()?;
            let bv = b.as_f64()?;
            Some(av.partial_cmp(&bv)?)
        }
        (Value::String(a), Value::String(b)) => Some(a.cmp(b)),
        _ => None,
    }
}

/// 字符串包含判断
fn json_contains(actual: &serde_json::Value, target: &serde_json::Value) -> bool {
    json_str_op(actual, target, |a, b| a.contains(b))
}

/// 对两个 JSON 值作为字符串执行操作
fn json_str_op<F: Fn(&str, &str) -> bool>(
    actual: &serde_json::Value,
    target: &serde_json::Value,
    op: F,
) -> bool {
    let a_owned = json_to_string(actual);
    let b_owned = json_to_string(target);
    op(&a_owned, &b_owned)
}

/// 将 JSON 值转为字符串（字符串原样返回，其余序列化为 JSON 文本）
fn json_to_string(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// 执行 route 节点：发起转接
///
/// 读取配置：
/// - `target_type`：extension / pstn / queue（默认 extension）
/// - `target`：转接目标（分机号 / PSTN 号码 / 队列 ID）
///
/// 返回 [`NodeExecuteResult::Transfer`]，由拓扑引擎执行实际转接。
pub fn execute_route(node: &TopologyNode, context: &IvrExecutionContext) -> NodeExecuteResult {
    let target_type = node
        .config
        .get("target_type")
        .and_then(|v| v.as_str())
        .unwrap_or("extension")
        .to_string();
    let target = node
        .config
        .get("target")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if target.is_empty() {
        return NodeExecuteResult::Error {
            message: "route 节点未配置 target".to_string(),
        };
    }
    let rendered_target = context.render_template(target);
    info!(
        call_id = %context.call_id,
        node_id = %node.id,
        target_type = %target_type,
        target = %rendered_target,
        "IVR route 节点发起转接"
    );
    NodeExecuteResult::Transfer {
        target: rendered_target,
        transfer_type: target_type,
    }
}

/// 执行 set_var 节点：批量设置上下文变量
///
/// 读取配置：
/// - `variables`：JSON 对象，键为变量名，值为变量值
///
/// 值若为字符串则进行 `{{var}}` 模板渲染；其余类型原样写入。
/// 完成后返回 `Continue { port: "default" }`。
pub fn execute_set_var(
    node: &TopologyNode,
    context: &mut IvrExecutionContext,
) -> NodeExecuteResult {
    let vars = match node.config.get("variables").and_then(|v| v.as_object()) {
        Some(o) => o,
        None => {
            warn!(
                call_id = %context.call_id,
                node_id = %node.id,
                "set_var 节点未配置 variables 对象, 跳过"
            );
            return NodeExecuteResult::Continue {
                port: "default".to_string(),
            };
        }
    };
    for (k, v) in vars {
        let rendered = match v {
            serde_json::Value::String(s) => serde_json::Value::String(context.render_template(s)),
            other => other.clone(),
        };
        context.set_var(k, rendered);
    }
    info!(
        call_id = %context.call_id,
        node_id = %node.id,
        count = vars.len(),
        "IVR set_var 节点完成"
    );
    NodeExecuteResult::Continue {
        port: "default".to_string(),
    }
}

/// 执行 loop 节点：按 loop_id 计数，未超限走 loop 端口，超限走 exit 端口
///
/// 读取配置：
/// - `loop_id`：循环标识（默认使用 node.id）
/// - `max_iterations`：最大迭代次数（默认 10）
///
/// 返回 `Continue { port: "loop" }` 或 `Continue { port: "exit" }`。
pub fn execute_loop(node: &TopologyNode, context: &mut IvrExecutionContext) -> NodeExecuteResult {
    let loop_id = node
        .config
        .get("loop_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| node.id.clone());
    let max_iter = node
        .config
        .get("max_iterations")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32)
        .unwrap_or(DEFAULT_LOOP_MAX_ITERATIONS);

    let counter = context.loop_counters.entry(loop_id.clone()).or_insert(0);
    *counter += 1;
    let current = *counter;
    let port = if current <= max_iter { "loop" } else { "exit" };
    info!(
        call_id = %context.call_id,
        node_id = %node.id,
        loop_id = %loop_id,
        current,
        max_iter,
        port,
        "IVR loop 节点"
    );
    NodeExecuteResult::Continue {
        port: port.to_string(),
    }
}

/// 执行 http_webhook 节点：发起 HTTP 调用并存储响应
///
/// 读取配置：
/// - `url`：请求地址（必填，支持模板渲染）
/// - `method`：HTTP 方法（默认 GET）
/// - `headers`：请求头对象（键值对，可选）
/// - `body`：请求体（JSON 值，可选）
/// - `timeout_secs`：超时秒数（默认 10）
///
/// 成功返回 `Continue { port: "success" }`，失败返回 `Continue { port: "error" }`。
/// 响应体（JSON 解析成功则原样存储，否则作为字符串存储）写入 `context.last_webhook_response`。
pub async fn execute_webhook(
    node: &TopologyNode,
    context: &mut IvrExecutionContext,
) -> NodeExecuteResult {
    let url = match node.config.get("url").and_then(|v| v.as_str()) {
        Some(s) if !s.is_empty() => context.render_template(s),
        _ => {
            return NodeExecuteResult::Error {
                message: "http_webhook 节点未配置 url".to_string(),
            };
        }
    };
    let method = read_method(node);
    let timeout_secs = read_timeout_secs(node);

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            warn!(call_id = %context.call_id, node_id = %node.id, error = %e, "webhook 构建 client 失败");
            return error_port();
        }
    };

    let request = match build_webhook_request(&client, &method, &url, node, context) {
        Some(r) => r,
        None => return error_port(),
    };

    info!(
        call_id = %context.call_id,
        node_id = %node.id,
        method = %method,
        url = %url,
        "IVR http_webhook 节点发起请求"
    );

    match request.send().await {
        Ok(resp) => handle_webhook_response(resp, node, context).await,
        Err(e) => {
            warn!(
                call_id = %context.call_id,
                node_id = %node.id,
                error = %e,
                "webhook 请求失败"
            );
            error_port()
        }
    }
}

/// 读取 webhook 节点的 method 配置（默认 GET，大写化）
fn read_method(node: &TopologyNode) -> String {
    node.config
        .get("method")
        .and_then(|v| v.as_str())
        .unwrap_or("GET")
        .to_uppercase()
}

/// 读取 webhook 节点的超时秒数（默认 10）
fn read_timeout_secs(node: &TopologyNode) -> u64 {
    node.config
        .get("timeout_secs")
        .and_then(|v| v.as_u64())
        .unwrap_or(DEFAULT_WEBHOOK_TIMEOUT_SECS)
}

/// 根据 method 构建 reqwest 请求构建器，注入 headers 与 body
fn build_webhook_request(
    client: &reqwest::Client,
    method: &str,
    url: &str,
    node: &TopologyNode,
    context: &IvrExecutionContext,
) -> Option<reqwest::RequestBuilder> {
    let req = match method {
        "GET" => client.get(url),
        "POST" => client.post(url),
        "PUT" => client.put(url),
        "DELETE" => client.delete(url),
        "PATCH" => client.patch(url),
        other => {
            warn!(call_id = %context.call_id, node_id = %node.id, method = other, "webhook 不支持的 method");
            return None;
        }
    };
    let req = inject_headers(req, node, context);
    let req = inject_body(req, node);
    Some(req)
}

/// 注入请求头（值支持模板渲染）
fn inject_headers(
    req: reqwest::RequestBuilder,
    node: &TopologyNode,
    context: &IvrExecutionContext,
) -> reqwest::RequestBuilder {
    let Some(headers) = node.config.get("headers").and_then(|v| v.as_object()) else {
        return req;
    };
    let mut req = req;
    for (k, v) in headers {
        if let Some(s) = v.as_str() {
            req = req.header(k, context.render_template(s));
        }
    }
    req
}

/// 注入请求体（若配置了 body 则作为 JSON 发送）
fn inject_body(req: reqwest::RequestBuilder, node: &TopologyNode) -> reqwest::RequestBuilder {
    match node.config.get("body") {
        Some(body) => req.json(body),
        None => req,
    }
}

/// 处理 webhook 响应：解析 JSON、写入上下文、返回对应端口
async fn handle_webhook_response(
    resp: reqwest::Response,
    node: &TopologyNode,
    context: &mut IvrExecutionContext,
) -> NodeExecuteResult {
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    let parsed =
        serde_json::from_str(&text).unwrap_or_else(|_| serde_json::Value::String(text.clone()));
    context.last_webhook_response = Some(parsed.clone());
    info!(
        call_id = %context.call_id,
        node_id = %node.id,
        status = %status,
        "webhook 响应已存储"
    );
    if status.is_success() {
        NodeExecuteResult::Continue {
            port: "success".to_string(),
        }
    } else {
        error_port()
    }
}

/// 返回 error 端口
fn error_port() -> NodeExecuteResult {
    NodeExecuteResult::Continue {
        port: "error".to_string(),
    }
}
