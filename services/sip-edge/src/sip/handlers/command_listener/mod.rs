//! VCI（Voice Call Interface）命令监听与分发。
//!
//! 入口 [`handle_command`] 反序列化 NATS 通道上的 [`CallCommand`]，
//! 按 [`CommandAction`] 分发到具体子模块执行。
//!
//! 命令分类：
//! - 拨号类：Dial / Originate / Bridge
//! - 拆线类：Hangup
//! - 媒体类：Play / Gather / Stream
//! - 代理类：Record / Say / Queue / Conference / Redirect / Pause / PlayDigits

mod bridge_handler;
mod commands;
mod dial_handler;
mod hangup_handler;
mod instruction_proxy;
mod media_command_handlers;
mod originate_handler;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use tracing::error;

use crate::config::EdgeConfig;
use crate::edge_state::EdgeState;

// 重导出对外构造 CommandAction 所需的类型。
// `StreamParams` 被 interactive_control 子模块通过绝对路径引用，必须重导出。
// `PlayParams` / `GatherParams` 仅在 media_command_handlers 子模块内部使用，不重导出。
pub(crate) use commands::{CallCommand, CommandAction, DialParams, HangupParams, StreamParams};

/// VCI 命令入口：依据 action 类型分发到具体处理器。
pub async fn handle_command(
    command: CallCommand,
    edge_state: &Arc<EdgeState>,
    edge_config: &EdgeConfig,
) {
    let call_id = command.call_id;
    let socket = match edge_state.get_socket() {
        Some(s) => s,
        None => {
            error!(
                call_id,
                "UdpSocket not initialized in EdgeState, cannot send SIP response"
            );
            return;
        }
    };

    match command.action {
        CommandAction::Dial { params } => {
            dial_handler::handle_dial(&call_id, params, edge_state, edge_config, &socket).await;
        }
        CommandAction::Hangup { params } => {
            hangup_handler::handle_hangup(&call_id, params, edge_state, edge_config, &socket).await;
        }
        CommandAction::Play { params } => {
            media_command_handlers::handle_play(&call_id, params, edge_state, edge_config, &socket)
                .await;
        }
        CommandAction::Gather { params } => {
            media_command_handlers::handle_gather(
                &call_id,
                params,
                edge_state,
                edge_config,
                &socket,
            )
            .await;
        }
        CommandAction::Stream { params } => {
            media_command_handlers::handle_stream(
                &call_id,
                params,
                edge_state,
                edge_config,
                &socket,
            )
            .await;
        }
        CommandAction::Record {
            max_length_secs,
            play_beep,
        } => {
            instruction_proxy::handle_record(
                &call_id,
                max_length_secs,
                play_beep,
                edge_state,
                edge_config,
            )
            .await;
        }
        CommandAction::Say { text, voice } => {
            instruction_proxy::handle_say(&call_id, text, voice, edge_state, edge_config).await;
        }
        CommandAction::Queue { queue_id, moh_url } => {
            instruction_proxy::handle_queue(&call_id, queue_id, moh_url, edge_state, edge_config)
                .await;
        }
        CommandAction::Conference {
            room_id,
            start_muted,
        } => {
            instruction_proxy::handle_conference(
                &call_id,
                room_id,
                start_muted,
                edge_state,
                edge_config,
            )
            .await;
        }
        CommandAction::Redirect { url } => {
            instruction_proxy::handle_redirect(&call_id, url, edge_state, edge_config).await;
        }
        CommandAction::Pause { duration_ms } => {
            instruction_proxy::handle_pause(&call_id, duration_ms, edge_state, edge_config).await;
        }
        CommandAction::PlayDigits { digits } => {
            instruction_proxy::handle_play_digits(&call_id, digits, edge_state, edge_config).await;
        }
        CommandAction::Originate {
            target_uri,
            caller_id,
        } => {
            originate_handler::handle_originate(
                &call_id,
                target_uri,
                caller_id,
                edge_state,
                edge_config,
            )
            .await;
        }
        CommandAction::Bridge {
            call_id_a,
            call_id_b,
        } => {
            bridge_handler::handle_bridge(&call_id, call_id_a, call_id_b, edge_state, edge_config)
                .await;
        }
    }
}
