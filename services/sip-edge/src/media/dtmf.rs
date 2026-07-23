//! DTMF integration with relay state, metrics, and CDR audit records.

use crate::media::relay::MediaRelayState;
use cdr_core::DtmfEventRecord;
use rtp_core::RtpPacketView;
use tracing::{debug, warn};

pub use media_core::dtmf::DtmfTracker as DtmfState;

impl MediaRelayState {
    pub fn register_port_dtmf_tracking(&self, call_id: &str, port: u16, payload_type: u8) {
        if !self.port_is_local(port) {
            if let Some(target) = self.remote_target_for_port(port) {
                let _ = self.call_remote_target(
                    target,
                    "register_port_dtmf_tracking",
                    serde_json::json!({
                        "call_id": call_id,
                        "port": port,
                        "payload_type": payload_type,
                    }),
                );
            }
            return;
        }

        self.dtmf_states
            .insert(port, DtmfState::new(call_id, payload_type));
        self.mark_relay_features_changed(port);
    }

    pub fn get_dtmf_digits(&self, call_id: &str) -> Option<String> {
        if !self.call_is_local(call_id) {
            if let Some(target) = self.remote_target_for_call(call_id) {
                if let Ok(res) = self.call_remote_target(
                    target,
                    "get_dtmf_digits",
                    serde_json::json!({ "call_id": call_id }),
                ) {
                    return res.as_str().map(|s| s.to_string());
                }
            }
            return None;
        }

        let inner = match self.state.lock() {
            Ok(inner) => inner,
            Err(_) => {
                warn!(call_id, "DTMF 状态锁已中毒，无法读取 DTMF");
                return None;
            }
        };
        inner.dtmf_accumulators.get(call_id).cloned()
    }

    pub fn clear_dtmf_digits(&self, call_id: &str) {
        if !self.call_is_local(call_id) {
            if let Some(target) = self.remote_target_for_call(call_id) {
                let _ = self.call_remote_target(
                    target,
                    "clear_dtmf_digits",
                    serde_json::json!({ "call_id": call_id }),
                );
            }
            return;
        }

        let Ok(mut inner) = self.state.lock() else {
            warn!(call_id, "DTMF 状态锁已中毒，跳过清理");
            return;
        };
        inner.dtmf_accumulators.remove(call_id);
    }

    pub fn register_info_dtmf_digit(&self, call_id: &str, digit: char) {
        let Ok(mut inner) = self.state.lock() else {
            warn!(call_id, "DTMF 状态锁已中毒，跳过 SIP INFO DTMF");
            return;
        };
        inner
            .dtmf_accumulators
            .entry(call_id.to_string())
            .or_default()
            .push(digit);
        inner
            .dtmf_event_log
            .entry(call_id.to_string())
            .or_default()
            .push(DtmfEventRecord::from_sip_info(call_id, digit));
        debug!(call_id, digit = %digit, "reconstructed DTMF digit from SIP INFO");
    }

    pub fn take_dtmf_events(&self, call_id: &str) -> Vec<DtmfEventRecord> {
        if !self.call_is_local(call_id) {
            if let Some(target) = self.remote_target_for_call(call_id) {
                if let Ok(res) = self.call_remote_target(
                    target,
                    "take_dtmf_events",
                    serde_json::json!({ "call_id": call_id }),
                ) {
                    if let Ok(events) = serde_json::from_value::<Vec<DtmfEventRecord>>(res) {
                        return events;
                    }
                }
            }
            return Vec::new();
        }

        let Ok(mut inner) = self.state.lock() else {
            warn!(call_id, "DTMF 状态锁已中毒，无法读取事件");
            return Vec::new();
        };
        inner.dtmf_event_log.remove(call_id).unwrap_or_default()
    }

    pub fn clear_dtmf_events(&self, call_id: &str) {
        if !self.call_is_local(call_id) {
            if let Some(target) = self.remote_target_for_call(call_id) {
                let _ = self.call_remote_target(
                    target,
                    "clear_dtmf_events",
                    serde_json::json!({ "call_id": call_id }),
                );
            }
            return;
        }

        let Ok(mut inner) = self.state.lock() else {
            warn!(call_id, "DTMF 状态锁已中毒，跳过事件清理");
            return;
        };
        inner.dtmf_event_log.remove(call_id);
    }

    pub(crate) fn process_dtmf_packet(&self, local_port: u16, packet: RtpPacketView<'_>) {
        let observation = self
            .dtmf_states
            .get_mut(&local_port)
            .and_then(|mut state| state.observe(packet));
        let Some(observation) = observation else {
            return;
        };

        let Ok(mut inner) = self.state.lock() else {
            warn!(
                call_id = observation.call_id,
                "DTMF 状态锁已中毒，跳过 RTP DTMF"
            );
            return;
        };
        inner
            .dtmf_accumulators
            .entry(observation.call_id.clone())
            .or_default()
            .push(observation.digit);
        inner
            .dtmf_event_log
            .entry(observation.call_id.clone())
            .or_default()
            .push(DtmfEventRecord::from_rtp(
                &observation.call_id,
                observation.digit,
                observation.rtp_timestamp,
                observation.duration,
                observation.volume,
            ));
        drop(inner);

        self.metrics.entry(local_port).or_default().dtmf_events += 1;
        debug!(
            call_id = observation.call_id,
            digit = %observation.digit,
            timestamp = observation.rtp_timestamp,
            duration = observation.duration,
            end = observation.end,
            volume = observation.volume,
            "reconstructed RTP DTMF digit"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cdr_core::DtmfSource;
    use rtp_core::RtpPacket;

    fn process(relay: &MediaRelayState, timestamp: u32, payload: Vec<u8>) {
        let bytes = RtpPacket::new(101, 1, timestamp, 42, payload)
            .unwrap()
            .encode()
            .unwrap();
        relay.process_dtmf_packet(40_000, RtpPacketView::parse(&bytes).unwrap());
    }

    #[tokio::test]
    async fn dtmf_adapter_records_and_counts_each_timestamp_once() {
        let relay = MediaRelayState::new();
        relay.register_port_dtmf_tracking("call-dtmf", 40_000, 101);

        process(&relay, 160, vec![5, 0, 0, 80]);
        process(&relay, 160, vec![5, 0x80, 1, 64]);
        process(&relay, 320, vec![11, 0x8a, 1, 64]);

        assert_eq!(relay.get_dtmf_digits("call-dtmf").as_deref(), Some("5#"));
        assert_eq!(relay.metrics.get(&40_000).unwrap().dtmf_events, 2);
        let events = relay.take_dtmf_events("call-dtmf");
        assert_eq!(events.len(), 2);
        assert_eq!(events[1].source, DtmfSource::Rtp);
        assert_eq!(events[1].rtp_timestamp, Some(320));
        assert_eq!(events[1].duration_ms, Some(320));
        assert_eq!(events[1].volume, Some(10));
    }
}
