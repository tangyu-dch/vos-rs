//! VCI 呼叫控制命令的数据模型。
//!
//! 这些类型由 [`super::handle_command`] 入口反序列化，用于在 NATS
//! 通道上驱动 B2BUA 中的呼叫行为（Dial/Hangup/Play/Gather/...）。

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct DialParams {
    pub target_gateway: Option<String>,
    pub target_uri: Option<String>,
    pub caller_id: Option<String>,
    pub timeout_secs: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct PlayParams {
    pub url: String,
    pub loop_count: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct GatherParams {
    pub play_url: Option<String>,
    pub max_digits: usize,
    pub timeout_ms: u64,
}

#[derive(Debug, Deserialize)]
pub struct HangupParams {
    pub sip_cause: Option<u16>,
}

#[derive(Debug, Deserialize)]
pub struct StreamParams {
    pub websocket_url: String,
    pub format: String,
    pub barge_in: bool,
}

/// VCI 支持的所有呼叫控制动作。
#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum CommandAction {
    Dial {
        #[serde(flatten)]
        params: DialParams,
    },
    Play {
        #[serde(flatten)]
        params: PlayParams,
    },
    Gather {
        #[serde(flatten)]
        params: GatherParams,
    },
    Hangup {
        #[serde(flatten)]
        params: HangupParams,
    },
    Stream {
        #[serde(flatten)]
        params: StreamParams,
    },
    Record {
        max_length_secs: u32,
        play_beep: bool,
    },
    Say {
        text: String,
        voice: String,
    },
    Queue {
        queue_id: String,
        moh_url: String,
    },
    Conference {
        room_id: String,
        start_muted: bool,
    },
    Redirect {
        url: String,
    },
    Pause {
        duration_ms: u64,
    },
    PlayDigits {
        digits: String,
    },
    Originate {
        target_uri: String,
        caller_id: String,
    },
    Bridge {
        call_id_a: String,
        call_id_b: String,
    },
}

/// 单个 VCI 命令的载体。
#[derive(Debug, Deserialize)]
pub struct CallCommand {
    pub call_id: String,
    #[serde(flatten)]
    pub action: CommandAction,
}
