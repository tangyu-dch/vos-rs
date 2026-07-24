//! Copilot SIP 信令梯形图（Call Ladder Diagram）生成与渲染。
//!
//! 拆分自 copilot.rs，包含：
//! - SipLadderStep 类型
//! - ladder_from_flows：从 SIP 信令流记录生成梯形图步骤
//! - ladder_from_cdr：从 CDR 事件合成梯形图步骤
//! - generate_ascii_ladder：渲染为 ASCII 文本格式

use cdr_core::{CdrEvent, SipFlowRecord};
use serde::{Deserialize, Serialize};

/// SIP 信令梯形图事件节点
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SipLadderStep {
    pub timestamp: String,
    pub direction: String,
    pub method_or_status: String,
    pub summary: String,
}

pub fn ladder_from_flows(flows: &[SipFlowRecord], start_ms: i64) -> Vec<SipLadderStep> {
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

pub fn ladder_from_cdr(cdr: &CdrEvent) -> Vec<SipLadderStep> {
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

/// 动态渲染 ASCII 格式的 SIP 交互梯形图 (Call Ladder Diagram)
pub fn generate_ascii_ladder(steps: &[SipLadderStep]) -> String {
    let mut out = String::new();
    out.push_str(
        "   [ Caller (UAC) ]            [ sip-edge B2BUA ]            [ Gateway (UAS) ]\n",
    );
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
