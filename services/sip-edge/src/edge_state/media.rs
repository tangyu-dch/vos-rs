//! # 媒体资源管理
//!
//! 本模块扩展 [`EdgeState`][super::EdgeState]，处理 B2BUA 会话与媒体层的交互：
//!
//! - [`remember_gateway_media`]: 收到 B-leg 响应后绑定 gateway RTP 端点，启动录音。
//! - [`remember_gateway_remote_tag`]: 从 B-leg 响应中提取并更新 gateway dialog To tag。
//! - [`clear_media_targets`]: 释放会话的 RTP relay 端口与会议成员资格。
//!
//! 所有媒体操作均以 `session_id` 为索引，避免直接使用 wire Call-ID。

use call_core::CallId;
use sdp_core::RtpEndpoint;
use tracing::{debug, info, warn};

use crate::media::relay::MediaRelayMetrics;
use crate::media::MediaConfig;
use crate::sip::dialog;

use super::models::InboundTransaction;
use super::EdgeState;

impl EdgeState {
    /// 绑定 gateway RTP 端点并启动录音。
    ///
    /// `call_id` 可以是 A-leg 或 B-leg Call-ID，[`CallSessionStore`][super::models::CallSessionStore]
    /// 会自动解析到 `session_id`。当 gateway relay 端口已就绪时：
    ///
    /// 1. 调用 [`MediaRelayState::pair_ports`] 配对 caller/gateway 端口
    /// 2. 调用 [`MediaRelayState::start_call_recording`] 启动录音（按 session_id）
    /// 3. 通过 [`CallManager::set_recording_path`] 反向通知 CDR 层
    pub(crate) fn remember_gateway_media(
        &self,
        call_id: &str,
        gateway_rtp: Option<RtpEndpoint>,
        caller_relay_rtp: RtpEndpoint,
        media_config: &MediaConfig,
    ) {
        let recording_config = self.recording_media_config(media_config);
        let media_session = self.inbound_transactions.get(call_id).map(|transaction| {
            (
                transaction
                    .gateway_relay_rtp
                    .as_ref()
                    .map(|endpoint| endpoint.port),
                transaction.session_id.clone(),
                transaction.dialogs.caller.call_id.clone(),
            )
        });

        if let Some(mut transaction) = self.inbound_transactions.get_mut(call_id) {
            transaction.gateway_rtp = gateway_rtp;
            if let Some((Some(gw_port), session_id, caller_call_id)) = media_session {
                self.media_relay.pair_ports(gw_port, caller_relay_rtp.port);
                match self.media_relay.start_call_recording(
                    &session_id,
                    caller_relay_rtp.port,
                    gw_port,
                    &recording_config,
                ) {
                    Ok(Some(path)) => {
                        self.call_manager.set_recording_path(
                            &CallId::new(caller_call_id.clone()),
                            format!("local:{}", path.display()),
                        );
                        debug!(session_id, caller_call_id, path = %path.display(), "started call recording");
                    }
                    Ok(None) => {}
                    Err(error) => {
                        warn!(session_id, caller_call_id, %error, "failed to start call recording");
                    }
                }
            }
            transaction.caller_relay_rtp = Some(caller_relay_rtp);
        }
    }

    /// 从 B-leg 响应中提取并更新 gateway dialog 的 remote tag。
    ///
    /// 处理 fork 场景：若已有 tag 且与新 tag 不同，仅在 180-299 状态码区间内更新；
    /// 其它情况记录告警并保留原 tag。
    pub(crate) fn remember_gateway_remote_tag(
        &self,
        call_id: &str,
        response: &sip_core::SipResponse,
    ) {
        let Some(to_tag) = response
            .headers
            .get("to")
            .and_then(|value| dialog::tag_param(value.as_str()))
        else {
            return;
        };
        let Some(mut transaction) = self.inbound_transactions.get_mut(call_id) else {
            return;
        };

        match &transaction.dialogs.gateway.remote_tag {
            Some(existing_tag) if existing_tag != &to_tag => {
                if response.status_code >= 180 && response.status_code <= 299 {
                    debug!(
                        call_id,
                        existing_tag,
                        new_tag = %to_tag,
                        status_code = response.status_code,
                        "updating dialog To tag from provisional/final response"
                    );
                    transaction.dialogs.gateway.remote_tag = Some(to_tag);
                } else {
                    debug!(
                        call_id,
                        existing_tag,
                        new_tag = %to_tag,
                        status_code = response.status_code,
                        "ignoring non-provisional/final response with different To tag; B2BUA tracks single B-leg dialog"
                    );
                }
            }
            Some(_) => {}
            None => {
                transaction.dialogs.gateway.remote_tag = Some(to_tag);
            }
        }
    }

    /// 释放会话的 RTP relay 端口、监控器和会议成员资格。
    ///
    /// 在 [`teardown_call_transaction`][super::session::teardown_call_transaction]
    /// 中调用，确保会话结束时媒体资源被同步回收。
    pub(crate) fn clear_media_targets(&self, transaction: &InboundTransaction) {
        let metrics_log_enabled = self.media_metrics_log;
        if let Some(endpoint) = &transaction.gateway_relay_rtp {
            self.media_relay.clear_monitors(endpoint.port);
            let metrics = self.media_relay.metrics_for_port(endpoint.port);
            log_media_target_metrics("gateway", endpoint.port, metrics, metrics_log_enabled);
            self.media_relay.clear_target(endpoint.port);

            // 如果是参会成员，清理出会议室
            let mgr = self.media_relay.conference_manager.clone();
            let port = endpoint.port;
            tokio::spawn(async move {
                mgr.leave_conference(port).await;
            });
        }
        if let Some(endpoint) = &transaction.caller_relay_rtp {
            self.media_relay.clear_monitors(endpoint.port);
            let metrics = self.media_relay.metrics_for_port(endpoint.port);
            log_media_target_metrics("caller", endpoint.port, metrics, metrics_log_enabled);
            self.media_relay.clear_target(endpoint.port);

            // 如果是参会成员，清理出会议室
            let mgr = self.media_relay.conference_manager.clone();
            let port = endpoint.port;
            tokio::spawn(async move {
                mgr.leave_conference(port).await;
            });
        }

        let totals = self.media_relay.metrics_totals();
        debug!(
            received_packets = totals.received_packets,
            forwarded_packets = totals.forwarded_packets,
            dropped_invalid_packets = totals.dropped_invalid_packets,
            dropped_no_target_packets = totals.dropped_no_target_packets,
            send_errors = totals.send_errors,
            learned_source_updates = totals.learned_source_updates,
            rtcp_quality = ?totals.rtcp_quality,
            recorded_packets = totals.recorded_packets,
            recording_dropped_packets = totals.recording_dropped_packets,
            recording_errors = totals.recording_errors,
            dtmf_events = totals.dtmf_events,
            "RTP relay metrics totals"
        );
    }
}

/// 按 `info`/`debug` 级别输出 RTP relay 端口的清理前指标。
fn log_media_target_metrics(
    leg: &'static str,
    port: u16,
    metrics: MediaRelayMetrics,
    info_enabled: bool,
) {
    if info_enabled {
        info!(
            leg,
            port,
            received_packets = metrics.received_packets,
            forwarded_packets = metrics.forwarded_packets,
            dropped_invalid_packets = metrics.dropped_invalid_packets,
            dropped_no_target_packets = metrics.dropped_no_target_packets,
            send_errors = metrics.send_errors,
            learned_source_updates = metrics.learned_source_updates,
            rtcp_quality = ?metrics.rtcp_quality,
            recorded_packets = metrics.recorded_packets,
            recording_dropped_packets = metrics.recording_dropped_packets,
            recording_errors = metrics.recording_errors,
            dtmf_events = metrics.dtmf_events,
            "clearing RTP relay target"
        );
    } else {
        debug!(
            leg,
            port,
            received_packets = metrics.received_packets,
            forwarded_packets = metrics.forwarded_packets,
            dropped_invalid_packets = metrics.dropped_invalid_packets,
            dropped_no_target_packets = metrics.dropped_no_target_packets,
            send_errors = metrics.send_errors,
            learned_source_updates = metrics.learned_source_updates,
            rtcp_quality = ?metrics.rtcp_quality,
            recorded_packets = metrics.recorded_packets,
            recording_dropped_packets = metrics.recording_dropped_packets,
            recording_errors = metrics.recording_errors,
            dtmf_events = metrics.dtmf_events,
            "clearing RTP relay target"
        );
    }
}
