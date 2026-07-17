//! Webhook 事件与 VCI 控制协议。
//!
//! 本模块只定义跨服务协议，不包含 HTTP、NATS 或持久化实现。

use serde::{Deserialize, Serialize};

/// 当前 Webhook 事件协议版本。
pub const WEBHOOK_SCHEMA_VERSION: &str = "1.0";

/// 可投递的版本化 Webhook 事件信封。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WebhookEvent {
    /// 全局唯一事件 ID（UUID v4）。
    pub event_id: String,
    /// 协议版本。
    pub schema_version: String,
    /// SIP Call-ID。
    pub call_id: String,
    /// 进程内全局递增序号，用于还原事件产生顺序。
    pub sequence: u64,
    /// 事件产生时间，Unix 毫秒时间戳。
    pub occurred_at_ms: i64,
    /// 呼叫生命周期事件。
    #[serde(flatten)]
    pub event: CallEvent,
}

/// 第三方监听的呼叫生命周期事件。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event_type", content = "data", rename_all = "snake_case")]
pub enum CallEvent {
    /// 已接收呼叫并完成初始路由选择（A-leg 入局）。
    CallInitiated {
        /// 主叫标识，通常来自 SIP From。
        caller: Option<String>,
        /// 被叫号码。
        callee: Option<String>,
        /// 呼叫方向：`"inbound"` 表示外部呼入，`"outbound"` 表示平台主动外呼。
        direction: String,
        /// 腿标识：`"a_leg"`（主叫侧）或 `"b_leg"`（被叫侧）。
        leg: String,
    },
    /// 通过 `originate` 命令向目标发起了外呼（B-leg 发起）。
    CallOriginated {
        /// 被叫目标 URI。
        target_uri: String,
        /// 使用的主叫号码。
        caller_id: String,
        /// 腿标识，固定为 `"b_leg"`。
        leg: String,
    },
    /// 被叫开始振铃。
    CallRinging {
        /// 触发事件的 SIP 状态码。
        sip_status: u16,
        /// 腿标识：`"a_leg"` 或 `"b_leg"`。
        leg: String,
    },
    /// 被叫已经接通。
    CallAnswered {
        /// 触发事件的 SIP 状态码。
        sip_status: u16,
        /// 腿标识：`"a_leg"` 或 `"b_leg"`。
        leg: String,
    },
    /// 两路呼叫已通过 `bridge` 命令完成媒体桥接。
    CallBridged {
        /// A-leg 的 Call-ID。
        call_id_a: String,
        /// B-leg 的 Call-ID。
        call_id_b: String,
    },
    /// 接收到 DTMF 按键。
    DtmfReceived {
        /// 按键字符序列（例如 `"1"`、`"123#"`）。
        digits: String,
        /// 腿标识：`"a_leg"` 或 `"b_leg"`。
        leg: String,
    },
    /// 呼叫正常结束或失败。
    CallFinished {
        /// 已接通通话时长，单位为秒。
        duration_secs: u64,
        /// SIP 结束状态码；主动挂断时为空。
        sip_status: Option<u16>,
        /// Q.850 释放原因；当前状态机无法提供时为空。
        q850_cause: Option<u8>,
        /// 可读的结束原因。
        reason: String,
        /// 腿标识：`"a_leg"` 或 `"b_leg"`。
        leg: String,
    },
}


/// VCI（VOS Call Instruction）2.0 呼叫控制动作协议。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum VciInstruction {
    /// 播放提示音。
    Play { url: String, loop_count: u32 },
    /// 收集 DTMF 按键。
    Gather {
        play_url: Option<String>,
        max_digits: usize,
        timeout_ms: u64,
        inter_digit_timeout_ms: Option<u64>,
        finish_on_key: Option<String>,
        barge_in: bool,
    },
    /// 转接或多路并发外呼。
    Dial {
        targets: Vec<String>,
        sim_ring: bool,
        caller_id: Option<String>,
        timeout_secs: Option<u32>,
        record_call: bool,
    },
    /// 挂断电话。
    Hangup {
        reason_code: u8,
        sip_cause: Option<u16>,
    },
    /// 控制通话录音。
    Record {
        max_length_secs: u32,
        play_beep: bool,
        trim_silence: bool,
        silence_threshold_db: Option<i16>,
    },
    /// 将音频流转发到 WebSocket。
    Stream {
        websocket_url: String,
        format: String,
        barge_in: bool,
    },
    /// 文本转语音播报。
    Say {
        text: String,
        voice: String,
        speed: f32,
        pitch: i16,
    },
    /// 呼叫入队。
    Queue {
        queue_id: String,
        moh_url: String,
        priority: u32,
    },
    /// 加入会议室。
    Conference {
        room_id: String,
        start_muted: bool,
        end_on_exit: bool,
        max_participants: u32,
    },
    /// 重定向到另一个控制 Webhook。
    Redirect { url: String },
    /// 静默等待。
    Pause { duration_ms: u64 },
    /// 发送 DTMF 按键。
    PlayDigits { digits: String, duration_ms: u32 },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_webhook_event_round_trip_preserves_protocol_envelope() {
        let event = WebhookEvent {
            event_id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
            schema_version: WEBHOOK_SCHEMA_VERSION.to_string(),
            call_id: "call-1".to_string(),
            sequence: 42,
            occurred_at_ms: 1_720_000_000_123,
            event: CallEvent::CallAnswered { sip_status: 200, leg: "b_leg".to_string() },
        };

        let json = serde_json::to_string(&event).expect("事件应可序列化");
        let decoded: WebhookEvent = serde_json::from_str(&json).expect("事件应可反序列化");

        assert_eq!(decoded, event);
        assert!(json.contains("\"schema_version\":\"1.0\""));
        assert!(json.contains("\"event_type\":\"call_answered\""));
    }

    #[test]
    fn test_vci_dial_instruction_deserialization() {
        let json = r#"{
            "action": "dial",
            "targets": ["1001", "1002"],
            "sim_ring": true,
            "caller_id": "8888",
            "record_call": true
        }"#;

        let instruction: VciInstruction =
            serde_json::from_str(json).expect("Dial 指令应可反序列化");

        assert!(matches!(
            instruction,
            VciInstruction::Dial {
                targets,
                sim_ring: true,
                ..
            } if targets.len() == 2
        ));
    }
}
