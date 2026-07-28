//! IVR 菜单循环状态机模块。
//!
//! 实现 [`run_ivr_menu_loop`]：在独立 task 中监听 DTMF 按键输入，
//! 根据当前菜单的按键动作表执行跳转或动作，并处理超时与重试逻辑。

use crate::edge_state::IvrMenu;
use crate::{EdgeConfig, EdgeState};
use sip_core::SipRequest;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, info, warn};

use super::actions::execute_ivr_action;

/// IVR 菜单循环主入口。
///
/// 在独立 task 中循环执行：
/// 1. 播放当前菜单的欢迎提示音
/// 2. 监听 DTMF 按键输入（每次循环 100ms 检查）
/// 3. 命中按键动作时执行对应动作（跳转子菜单或转接/挂断等）
/// 4. 超过 `timeout_secs` 未输入则释放呼叫
/// 5. 同一菜单内连续 3 次无效按键则挂断
#[allow(clippy::too_many_arguments)]
pub(super) async fn run_ivr_menu_loop(
    edge_state: Arc<EdgeState>,
    edge_config: Arc<EdgeConfig>,
    call_id: String,
    media_session_id: String,
    a_port: u16,
    request: SipRequest,
    peer: SocketAddr,
    mut current_menu: IvrMenu,
) {
    loop {
        let welcome_prompt = current_menu.welcome_prompt.clone();
        let timeout_secs = if current_menu.timeout_secs > 0 {
            current_menu.timeout_secs
        } else {
            10
        };

        debug!(call_id = %call_id, prompt = %welcome_prompt, "播放 IVR 欢迎提示音");
        let _ = edge_state
            .media_relay
            .start_playback(
                a_port,
                std::path::PathBuf::from(&welcome_prompt),
                crate::media::relay::PlaybackMode::Exclusive,
                false,
            )
            .await;

        let start_time = std::time::Instant::now();
        let timeout = Duration::from_secs(timeout_secs as u64);
        let mut accum = String::new();
        let mut retries = 0;

        // 内层 loop 命中菜单跳转时通过 `break Some(menu_id)` 携带目标，
        // 其他退出路径均直接 `return`，因此 `next_menu_id` 在 break 后必有值。
        let next_menu_id: Option<String> = loop {
            tokio::time::sleep(Duration::from_millis(100)).await;

            // 检查呼叫是否已被挂断或清理
            if !edge_state.inbound_transactions.contains_key(&call_id) {
                return;
            }

            if let Some(digits) = edge_state.media_relay.get_dtmf_digits(&media_session_id) {
                if digits.len() > accum.len() {
                    let new_digit = digits.chars().last().unwrap();
                    accum.push(new_digit);
                    info!(call_id = %call_id, digit = %new_digit, "IVR 监听到按键输入");

                    // 监听到按键，立刻停止欢迎词播放
                    edge_state.media_relay.stop_playback(a_port);

                    if let Some(action) = current_menu.actions.get(&new_digit.to_string()) {
                        if let Some(prompt) = &action.waiting_prompt {
                            if !prompt.trim().is_empty() {
                                info!(call_id = %call_id, prompt = %prompt, "播放按键触发等待/提示音频");
                                let _ = edge_state
                                    .media_relay
                                    .start_playback(
                                        a_port,
                                        std::path::PathBuf::from(prompt),
                                        crate::media::relay::PlaybackMode::Exclusive,
                                        false,
                                    )
                                    .await;
                            }
                        }

                        if action.action_type == "menu" {
                            info!(call_id = %call_id, target = %action.action_target, "执行 IVR 菜单跳转动作");
                            break Some(action.action_target.clone());
                        } else {
                            execute_ivr_action(
                                &edge_state,
                                &edge_config,
                                &call_id,
                                a_port,
                                action,
                                &request,
                                peer,
                            )
                            .await;
                            return;
                        }
                    } else {
                        retries += 1;
                        accum.clear();
                        if retries >= 3 {
                            info!(call_id = %call_id, "IVR 超过最大重试次数，挂断呼叫");
                            edge_state
                                .call_manager
                                .terminate_call_with_reason(&call_id, "IVR Max Retries Exceeded");
                            return;
                        } else {
                            info!(call_id = %call_id, digit = %new_digit, retries, "IVR 无效按键，等待重试");
                        }
                    }
                }
            }

            if start_time.elapsed() > timeout {
                info!(call_id = %call_id, "IVR 输入超时，释放呼叫");
                edge_state.media_relay.stop_playback(a_port);
                edge_state
                    .call_manager
                    .terminate_call_with_reason(&call_id, "IVR Timeout");
                return;
            }
        };

        if let Some(menu_id) = next_menu_id {
            if let Some(new_menu) = edge_state
                .ivr_menus
                .read()
                .ok()
                .and_then(|lock| lock.get(&menu_id).cloned())
            {
                current_menu = new_menu;
            } else {
                warn!(call_id = %call_id, menu_id = %menu_id, "IVR 跳转目标菜单不存在");
                edge_state
                    .call_manager
                    .terminate_call_with_reason(&call_id, "IVR Target Menu Not Found");
                return;
            }
        } else {
            break;
        }
    }
}
