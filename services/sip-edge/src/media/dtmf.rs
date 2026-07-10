//! # DTMF 处理
//!
//! 本模块实现了 DTMF（Dual-Tone Multi-Frequency）信号的检测和处理，包括：
//!
//! - **RFC 2833**：RTP telephone-event 检测
//! - **SIP INFO**：SIP INFO 方式的 DTMF 传递
//! - **DTMF 累积**：按通话累积 DTMF 数字序列
//! - **审计记录**：DTMF 事件写入审计表
//!
//! ## 支持的 DTMF 数字
//!
//! ```text
//! 0-9, *, #, A-D
//! ```
//!
//! ## 事件格式
//!
//! RFC 2833 telephone-event：
//! - Event: DTMF 数字（0-15）
//! - End: 事件结束标志
//! - Duration: 事件持续时间

use crate::media::relay::MediaRelayState;
use cdr_core::DtmfEventRecord;
use rtp_core::{RtpPacketView, TelephoneEvent};
use tracing::debug;

#[derive(Debug, Clone)]
pub struct DtmfState {
    pub call_id: String,
    pub payload_type: u8,
    pub last_timestamp: Option<u32>,
}

impl MediaRelayState {
    pub fn register_port_dtmf_tracking(&self, call_id: &str, port: u16, payload_type: u8) {
        self.dtmf_states.insert(
            port,
            DtmfState {
                call_id: call_id.to_string(),
                payload_type,
                last_timestamp: None,
            },
        );
    }

    pub fn get_dtmf_digits(&self, call_id: &str) -> Option<String> {
        let inner = self.state.lock().expect("media relay lock poisoned");
        inner.dtmf_accumulators.get(call_id).cloned()
    }

    pub fn clear_dtmf_digits(&self, call_id: &str) {
        let mut inner = self.state.lock().expect("media relay lock poisoned");
        inner.dtmf_accumulators.remove(call_id);
    }

    pub fn register_info_dtmf_digit(&self, call_id: &str, digit: char) {
        let mut inner = self.state.lock().expect("media relay lock poisoned");
        let acc = inner
            .dtmf_accumulators
            .entry(call_id.to_string())
            .or_default();
        acc.push(digit);
        let record = DtmfEventRecord::from_sip_info(call_id, digit);
        inner
            .dtmf_event_log
            .entry(call_id.to_string())
            .or_default()
            .push(record);
        debug!(call_id, digit = %digit, "reconstructed DTMF digit from SIP INFO");
    }

    pub fn take_dtmf_events(&self, call_id: &str) -> Vec<DtmfEventRecord> {
        let mut inner = self.state.lock().expect("media relay lock poisoned");
        inner.dtmf_event_log.remove(call_id).unwrap_or_default()
    }

    pub fn clear_dtmf_events(&self, call_id: &str) {
        let mut inner = self.state.lock().expect("media relay lock poisoned");
        inner.dtmf_event_log.remove(call_id);
    }

    pub(crate) fn process_dtmf_packet(&self, local_port: u16, packet: RtpPacketView<'_>) {
        let (call_id, last_timestamp) = {
            let Some(state) = self.dtmf_states.get(&local_port) else {
                return;
            };
            if packet.payload_type != state.payload_type {
                return;
            }
            (state.call_id.clone(), state.last_timestamp)
        };

        let Ok(event) = TelephoneEvent::parse(packet.payload) else {
            return;
        };
        let Some(digit) = event.digit() else {
            return;
        };

        let timestamp = packet.timestamp;
        if Some(timestamp) != last_timestamp {
            if let Some(mut state) = self.dtmf_states.get_mut(&local_port) {
                state.last_timestamp = Some(timestamp);
            }
            let mut inner = self.state.lock().expect("media relay lock poisoned");
            let acc = inner.dtmf_accumulators.entry(call_id.clone()).or_default();
            acc.push(digit);
            let record =
                DtmfEventRecord::from_rtp(&call_id, digit, timestamp, event.duration, event.volume);
            inner
                .dtmf_event_log
                .entry(call_id.clone())
                .or_default()
                .push(record);
            drop(inner);
            self.metrics.entry(local_port).or_default().dtmf_events += 1;
            debug!(
                call_id,
                digit = %digit,
                timestamp,
                duration = event.duration,
                end = event.end,
                volume = event.volume,
                "reconstructed RTP DTMF digit"
            );
        }
    }
}
