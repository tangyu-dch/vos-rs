//! VCI 简单命令代理：Record / Say / Queue / Conference / Redirect / Pause / PlayDigits。
//!
//! 这些命令本身没有独立的状态机逻辑，只是把 VCI 协议参数转换为
//! [`call_core::VciInstruction`]，再委托给交互式控制执行器统一处理。

use std::sync::Arc;

use crate::config::EdgeConfig;
use crate::edge_state::EdgeState;
use crate::sip::handlers::interactive_control;

pub(super) async fn handle_record(
    call_id: &str,
    max_length_secs: u32,
    play_beep: bool,
    edge_state: &Arc<EdgeState>,
    edge_config: &EdgeConfig,
) {
    let inst = call_core::VciInstruction::Record {
        max_length_secs,
        play_beep,
        trim_silence: false,
        silence_threshold_db: None,
    };
    interactive_control::execute_instruction(
        inst,
        call_id.to_string(),
        edge_state.clone(),
        edge_config.clone(),
    )
    .await;
}

pub(super) async fn handle_say(
    call_id: &str,
    text: String,
    voice: String,
    edge_state: &Arc<EdgeState>,
    edge_config: &EdgeConfig,
) {
    let inst = call_core::VciInstruction::Say {
        text,
        voice,
        speed: 1.0,
        pitch: 0,
    };
    interactive_control::execute_instruction(
        inst,
        call_id.to_string(),
        edge_state.clone(),
        edge_config.clone(),
    )
    .await;
}

pub(super) async fn handle_queue(
    call_id: &str,
    queue_id: String,
    moh_url: String,
    edge_state: &Arc<EdgeState>,
    edge_config: &EdgeConfig,
) {
    let inst = call_core::VciInstruction::Queue {
        queue_id,
        moh_url,
        priority: 1,
    };
    interactive_control::execute_instruction(
        inst,
        call_id.to_string(),
        edge_state.clone(),
        edge_config.clone(),
    )
    .await;
}

pub(super) async fn handle_conference(
    call_id: &str,
    room_id: String,
    start_muted: bool,
    edge_state: &Arc<EdgeState>,
    edge_config: &EdgeConfig,
) {
    let inst = call_core::VciInstruction::Conference {
        room_id,
        start_muted,
        end_on_exit: true,
        max_participants: 20,
    };
    interactive_control::execute_instruction(
        inst,
        call_id.to_string(),
        edge_state.clone(),
        edge_config.clone(),
    )
    .await;
}

pub(super) async fn handle_redirect(
    call_id: &str,
    url: String,
    edge_state: &Arc<EdgeState>,
    edge_config: &EdgeConfig,
) {
    let inst = call_core::VciInstruction::Redirect { url };
    interactive_control::execute_instruction(
        inst,
        call_id.to_string(),
        edge_state.clone(),
        edge_config.clone(),
    )
    .await;
}

pub(super) async fn handle_pause(
    call_id: &str,
    duration_ms: u64,
    edge_state: &Arc<EdgeState>,
    edge_config: &EdgeConfig,
) {
    let inst = call_core::VciInstruction::Pause { duration_ms };
    interactive_control::execute_instruction(
        inst,
        call_id.to_string(),
        edge_state.clone(),
        edge_config.clone(),
    )
    .await;
}

pub(super) async fn handle_play_digits(
    call_id: &str,
    digits: String,
    edge_state: &Arc<EdgeState>,
    edge_config: &EdgeConfig,
) {
    let inst = call_core::VciInstruction::PlayDigits {
        digits,
        duration_ms: 100,
    };
    interactive_control::execute_instruction(
        inst,
        call_id.to_string(),
        edge_state.clone(),
        edge_config.clone(),
    )
    .await;
}
