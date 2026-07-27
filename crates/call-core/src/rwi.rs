//! RWI (Real-Time WebSocket Interface) 协议定义模块。
//!
//! 提供基于 WebSocket 的实时呼叫事件与下发指令定义。

use serde::{Deserialize, Serialize};

/// RWI 上行/推送呼叫事件。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", content = "data", rename_all = "snake_case")]
pub enum RwiEvent {
    /// 呼叫创建/发起事件。
    CallStarted {
        call_id: String,
        caller: String,
        callee: String,
        direction: String,
        timestamp_ms: i64,
    },
    /// 呼叫振铃事件。
    CallRinging {
        call_id: String,
        sip_status: u16,
        leg: String,
        timestamp_ms: i64,
    },
    /// 呼叫接通事件。
    CallAnswered {
        call_id: String,
        sip_status: u16,
        leg: String,
        timestamp_ms: i64,
    },
    /// 呼叫结束/挂断事件。
    CallEnded {
        call_id: String,
        duration_secs: u64,
        reason: String,
        sip_status: Option<u16>,
        timestamp_ms: i64,
    },
    /// 媒体协议层事件（DTMF 按键、音频状态变动、ASR/TTS 状态等）。
    MediaEvent {
        call_id: String,
        event_type: String,
        payload: String,
        timestamp_ms: i64,
    },
}

impl RwiEvent {
    /// 获取当前事件关联的 SIP Call-ID。
    pub fn call_id(&self) -> &str {
        match self {
            Self::CallStarted { call_id, .. }
            | Self::CallRinging { call_id, .. }
            | Self::CallAnswered { call_id, .. }
            | Self::CallEnded { call_id, .. }
            | Self::MediaEvent { call_id, .. } => call_id,
        }
    }

    /// 获取事件发生的 Unix 毫秒时间戳。
    pub fn timestamp_ms(&self) -> i64 {
        match self {
            Self::CallStarted { timestamp_ms, .. }
            | Self::CallRinging { timestamp_ms, .. }
            | Self::CallAnswered { timestamp_ms, .. }
            | Self::CallEnded { timestamp_ms, .. }
            | Self::MediaEvent { timestamp_ms, .. } => *timestamp_ms,
        }
    }
}

/// RWI 下行/控制指令。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "command", content = "data", rename_all = "snake_case")]
pub enum RwiCommand {
    /// 强插/耳语/监听指令。
    BargeIn {
        call_id: String,
        mode: String,
        target_leg: Option<String>,
    },
    /// 语音播报（TTS / 音频文件）指令。
    Speak {
        call_id: String,
        text: String,
        voice: Option<String>,
        speed: Option<f32>,
    },
    /// 媒体监听/音频流输出到指定 WebSocket。
    Listen {
        call_id: String,
        stream_url: String,
        format: Option<String>,
    },
    /// 呼叫转接指令。
    Transfer {
        call_id: String,
        target: String,
        transfer_type: Option<String>,
    },
    /// 挂断呼叫指令。
    Hangup {
        call_id: String,
        reason_code: Option<u8>,
    },
}

impl RwiCommand {
    /// 获取指令对应的 Call-ID。
    pub fn call_id(&self) -> &str {
        match self {
            Self::BargeIn { call_id, .. }
            | Self::Speak { call_id, .. }
            | Self::Listen { call_id, .. }
            | Self::Transfer { call_id, .. }
            | Self::Hangup { call_id, .. } => call_id,
        }
    }
}

/// RWI WebSocket 消息协议包。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RwiMessage {
    /// 消息 UUID。
    pub id: String,
    /// 协议版本，默认 `"1.0"`。
    pub version: String,
    /// 消息载荷。
    #[serde(flatten)]
    pub payload: RwiPayload,
}

/// RWI 消息载荷。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RwiPayload {
    /// 上行事件。
    Event(RwiEvent),
    /// 下行指令。
    Command(RwiCommand),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rwi_event_serde_roundtrip() -> Result<(), Box<dyn std::error::Error>> {
        let event = RwiEvent::CallStarted {
            call_id: "call-123".to_string(),
            caller: "1001".to_string(),
            callee: "1002".to_string(),
            direction: "inbound".to_string(),
            timestamp_ms: 1720000000000,
        };

        let json = serde_json::to_string(&event)?;
        assert!(json.contains("\"event\":\"call_started\""));

        let decoded: RwiEvent = serde_json::from_str(&json)?;
        assert_eq!(decoded, event);
        assert_eq!(decoded.call_id(), "call-123");
        assert_eq!(decoded.timestamp_ms(), 1720000000000);
        Ok(())
    }

    #[test]
    fn test_rwi_command_serde_roundtrip() -> Result<(), Box<dyn std::error::Error>> {
        let cmd = RwiCommand::BargeIn {
            call_id: "call-456".to_string(),
            mode: "whisper".to_string(),
            target_leg: Some("a_leg".to_string()),
        };

        let json = serde_json::to_string(&cmd)?;
        assert!(json.contains("\"command\":\"barge_in\""));

        let decoded: RwiCommand = serde_json::from_str(&json)?;
        assert_eq!(decoded, cmd);
        assert_eq!(decoded.call_id(), "call-456");
        Ok(())
    }

    #[test]
    fn test_rwi_message_wrapper() -> Result<(), Box<dyn std::error::Error>> {
        let msg = RwiMessage {
            id: "msg-1".to_string(),
            version: "1.0".to_string(),
            payload: RwiPayload::Command(RwiCommand::Hangup {
                call_id: "call-789".to_string(),
                reason_code: Some(16),
            }),
        };

        let json = serde_json::to_string(&msg)?;
        let decoded: RwiMessage = serde_json::from_str(&json)?;
        assert_eq!(decoded, msg);
        Ok(())
    }
}
