//! IVR 拓扑引擎全量单元测试

use super::types::*;
use super::executors::flow;
use super::executors::basic;
use super::executors::media;
use super::voice_engine::VoiceEngineManager;
use serde_json::json;

/// 构造测试用拓扑图：Start -> SetVar -> Condition (vip==true) -> [port: match] Prompt -> Hangup
fn build_sample_topology() -> IvrTopology {
    let nodes = vec![
        TopologyNode {
            id: "node-start".to_string(),
            node_type: "start".to_string(),
            title: "开始".to_string(),
            description: None,
            config: json!({}),
            position: Some(Position { x: 0.0, y: 0.0 }),
        },
        TopologyNode {
            id: "node-set-var".to_string(),
            node_type: "set_var".to_string(),
            title: "设置VIP变量".to_string(),
            description: None,
            config: json!({
                "variables": {
                    "vip": "true"
                }
            }),
            position: Some(Position { x: 100.0, y: 0.0 }),
        },
        TopologyNode {
            id: "node-condition".to_string(),
            node_type: "condition".to_string(),
            title: "条件分支".to_string(),
            description: None,
            config: json!({
                "variable": "vip",
                "operator": "eq",
                "value": "true"
            }),
            position: Some(Position { x: 200.0, y: 0.0 }),
        },
        TopologyNode {
            id: "node-prompt-vip".to_string(),
            node_type: "prompt".to_string(),
            title: "VIP欢迎语".to_string(),
            description: None,
            config: json!({
                "prompt_url": "http://example.com/vip.wav"
            }),
            position: Some(Position { x: 300.0, y: 0.0 }),
        },
        TopologyNode {
            id: "node-prompt-normal".to_string(),
            node_type: "prompt".to_string(),
            title: "普通欢迎语".to_string(),
            description: None,
            config: json!({
                "prompt_url": "http://example.com/normal.wav"
            }),
            position: Some(Position { x: 300.0, y: 100.0 }),
        },
        TopologyNode {
            id: "node-hangup".to_string(),
            node_type: "hangup".to_string(),
            title: "挂断".to_string(),
            description: None,
            config: json!({ "reason": "normal" }),
            position: Some(Position { x: 400.0, y: 0.0 }),
        },
    ];

    let edges = vec![
        TopologyEdge {
            id: "edge-1".to_string(),
            source: "node-start".to_string(),
            target: "node-set-var".to_string(),
            source_port: None,
            label: None,
        },
        TopologyEdge {
            id: "edge-2".to_string(),
            source: "node-set-var".to_string(),
            target: "node-condition".to_string(),
            source_port: None,
            label: None,
        },
        TopologyEdge {
            id: "edge-3".to_string(),
            source: "node-condition".to_string(),
            target: "node-prompt-vip".to_string(),
            source_port: Some("match".to_string()),
            label: None,
        },
        TopologyEdge {
            id: "edge-4".to_string(),
            source: "node-condition".to_string(),
            target: "node-prompt-normal".to_string(),
            source_port: Some("nomatch".to_string()),
            label: None,
        },
        TopologyEdge {
            id: "edge-5".to_string(),
            source: "node-prompt-vip".to_string(),
            target: "node-hangup".to_string(),
            source_port: None,
            label: None,
        },
    ];

    IvrTopology { nodes, edges }
}

#[test]
fn test_topology_graph_construction_and_lookups() {
    let topo = build_sample_topology();
    let graph = TopologyGraph::build(&topo);

    assert_eq!(graph.start_node_id.as_deref(), Some("node-start"));

    let condition = graph.get_node("node-condition");
    assert!(condition.is_some());
    assert_eq!(condition.unwrap().node_type, "condition");

    // 默认输出边
    let next_from_start = graph.next_node("node-start", "default");
    assert_eq!(next_from_start.map(|n| n.id.as_str()), Some("node-set-var"));

    // 条件分支端口边
    let next_vip = graph.next_node("node-condition", "match");
    assert_eq!(next_vip.map(|n| n.id.as_str()), Some("node-prompt-vip"));

    let next_normal = graph.next_node("node-condition", "nomatch");
    assert_eq!(next_normal.map(|n| n.id.as_str()), Some("node-prompt-normal"));
}

#[test]
fn test_execution_context_variables_and_template_render() {
    let mut ctx = IvrExecutionContext::new("call-12345".to_string(), "1001".to_string(), "8888".to_string());
    ctx.set_var("customer_name", json!("张三"));
    ctx.set_var("account_balance", json!("100.50"));

    assert_eq!(ctx.get_var("customer_name"), Some(&json!("张三")));
    assert_eq!(ctx.get_var("account_balance"), Some(&json!("100.50")));
    assert_eq!(ctx.get_var("non_existent"), None);

    let rendered = ctx.render_template("尊敬的 {{customer_name}}，您的余额为 {{account_balance}} 元。");
    assert_eq!(rendered, "尊敬的 张三，您的余额为 100.50 元。");
}

#[tokio::test]
async fn test_condition_executor_operators() {
    let mut ctx = IvrExecutionContext::new("call-test".to_string(), "1001".to_string(), "8888".to_string());
    ctx.set_var("score", json!("95"));
    ctx.set_var("status", json!("ACTIVE"));
    ctx.set_var("phone", json!("13800138000"));

    // 1. 等于 eq
    let node_eq = TopologyNode {
        id: "c1".to_string(),
        node_type: "condition".to_string(),
        title: "测试eq".to_string(),
        description: None,
        config: json!({ "variable": "status", "operator": "eq", "value": "ACTIVE" }),
        position: Some(Position { x: 0.0, y: 0.0 }),
    };
    let res = flow::execute_condition(&node_eq, &mut ctx);
    match res {
        NodeExecuteResult::Continue { port } => assert_eq!(port, "match"),
        _ => panic!("Expected Continue port match"),
    }

    // 2. 大于 gt
    let node_gt = TopologyNode {
        id: "c2".to_string(),
        node_type: "condition".to_string(),
        title: "测试gt".to_string(),
        description: None,
        config: json!({ "variable": "score", "operator": "gt", "value": "90" }),
        position: Some(Position { x: 0.0, y: 0.0 }),
    };
    let res_gt = flow::execute_condition(&node_gt, &mut ctx);
    match res_gt {
        NodeExecuteResult::Continue { port } => assert_eq!(port, "match"),
        _ => panic!("Expected Continue port match"),
    }

    // 3. 包含 contains
    let node_contains = TopologyNode {
        id: "c3".to_string(),
        node_type: "condition".to_string(),
        title: "测试contains".to_string(),
        description: None,
        config: json!({ "variable": "phone", "operator": "contains", "value": "0013" }),
        position: Some(Position { x: 0.0, y: 0.0 }),
    };
    let res_contains = flow::execute_condition(&node_contains, &mut ctx);
    match res_contains {
        NodeExecuteResult::Continue { port } => assert_eq!(port, "match"),
        _ => panic!("Expected Continue port match"),
    }
}

#[tokio::test]
async fn test_set_var_and_loop_executors() {
    let mut ctx = IvrExecutionContext::new("call-loop".to_string(), "1001".to_string(), "8888".to_string());

    // 1. 设置变量节点
    let node_set = TopologyNode {
        id: "set-1".to_string(),
        node_type: "set_var".to_string(),
        title: "设置计数器".to_string(),
        description: None,
        config: json!({
            "variables": {
                "retry_count": "0"
            }
        }),
        position: Some(Position { x: 0.0, y: 0.0 }),
    };
    let res_set = flow::execute_set_var(&node_set, &mut ctx);
    match res_set {
        NodeExecuteResult::Continue { port } => assert_eq!(port, "default"),
        _ => panic!("Expected Continue default"),
    }
    assert_eq!(ctx.get_var("retry_count"), Some(&json!("0")));

    // 2. 循环控制节点 (允许最多3次)
    let node_loop = TopologyNode {
        id: "loop-1".to_string(),
        node_type: "loop".to_string(),
        title: "重试循环".to_string(),
        description: None,
        config: json!({ "loop_id": "l1", "max_iterations": 3 }),
        position: Some(Position { x: 0.0, y: 0.0 }),
    };

    // 第一次循环 (1 <= 3) -> loop
    let r1 = flow::execute_loop(&node_loop, &mut ctx);
    match r1 {
        NodeExecuteResult::Continue { port } => assert_eq!(port, "loop"),
        _ => panic!("Expected Continue loop"),
    }

    // 第二次循环
    let r2 = flow::execute_loop(&node_loop, &mut ctx);
    match r2 {
        NodeExecuteResult::Continue { port } => assert_eq!(port, "loop"),
        _ => panic!("Expected Continue loop"),
    }

    // 第三次循环
    let r3 = flow::execute_loop(&node_loop, &mut ctx);
    match r3 {
        NodeExecuteResult::Continue { port } => assert_eq!(port, "loop"),
        _ => panic!("Expected Continue loop"),
    }

    // 第四次循环 -> exit
    let r4 = flow::execute_loop(&node_loop, &mut ctx);
    match r4 {
        NodeExecuteResult::Continue { port } => assert_eq!(port, "exit"),
        _ => panic!("Expected Continue exit"),
    }
}

#[tokio::test]
async fn test_basic_executors_start_prompt_hangup() {
    let mut ctx = IvrExecutionContext::new("call-basic".to_string(), "1001".to_string(), "8888".to_string());

    let node_start = TopologyNode {
        id: "start".to_string(),
        node_type: "start".to_string(),
        title: "开始".to_string(),
        description: None,
        config: json!({}),
        position: Some(Position { x: 0.0, y: 0.0 }),
    };
    match basic::execute(&node_start, &mut ctx).await {
        NodeExecuteResult::Continue { port } => assert_eq!(port, "default"),
        _ => panic!("Expected Continue default"),
    }

    let node_hangup = TopologyNode {
        id: "hangup".to_string(),
        node_type: "hangup".to_string(),
        title: "挂断".to_string(),
        description: None,
        config: json!({ "reason": "normal" }),
        position: Some(Position { x: 0.0, y: 0.0 }),
    };
    match basic::execute(&node_hangup, &mut ctx).await {
        NodeExecuteResult::Hangup { reason } => assert_eq!(reason, "normal"),
        _ => panic!("Expected Hangup"),
    }
}

#[tokio::test]
async fn test_route_and_media_executors() {
    let mut ctx = IvrExecutionContext::new("call-media".to_string(), "1001".to_string(), "8888".to_string());

    // 路由/呼转分支节点
    let node_route = TopologyNode {
        id: "route-1".to_string(),
        node_type: "route".to_string(),
        title: "转人工坐席".to_string(),
        description: None,
        config: json!({ "target": "8001", "target_type": "extension" }),
        position: Some(Position { x: 0.0, y: 0.0 }),
    };
    match flow::execute_route(&node_route, &mut ctx) {
        NodeExecuteResult::Transfer { target, transfer_type } => {
            assert_eq!(target, "8001");
            assert_eq!(transfer_type, "extension");
        }
        _ => panic!("Expected Transfer"),
    }

    // 转接节点
    let node_transfer = TopologyNode {
        id: "transfer-1".to_string(),
        node_type: "transfer_queue".to_string(),
        title: "转接客服队列".to_string(),
        description: None,
        config: json!({ "target": "queue-support", "target_type": "queue" }),
        position: Some(Position { x: 0.0, y: 0.0 }),
    };
    match media::execute_transfer(&node_transfer, &mut ctx) {
        NodeExecuteResult::Transfer { target, transfer_type } => {
            assert_eq!(target, "queue-support");
            assert_eq!(transfer_type, "queue");
        }
        _ => panic!("Expected Transfer"),
    }
}

#[tokio::test]
async fn test_voice_engine_manager_disabled_default() {
    let manager = VoiceEngineManager::from_env();
    assert!(manager.tts.is_none());
    assert!(manager.asr.is_none());
}
