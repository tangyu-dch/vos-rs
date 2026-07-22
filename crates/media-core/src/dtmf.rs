//! Pure RFC 2833/4733 DTMF payload tracking.

use rtp_core::{RtpPacketView, TelephoneEvent};

/// A newly observed DTMF event, including the RTP audit fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DtmfObservation {
    /// Call identifier associated with the tracked RTP port.
    pub call_id: String,
    /// Decoded DTMF digit.
    pub digit: char,
    /// RTP timestamp used to identify retransmissions.
    pub rtp_timestamp: u32,
    /// RFC telephone-event duration value.
    pub duration: u16,
    /// RFC telephone-event volume value.
    pub volume: u8,
    /// Whether the telephone-event end bit was set.
    pub end: bool,
}

/// Tracks the negotiated telephone-event payload and suppresses retransmissions.
#[derive(Debug, Clone)]
pub struct DtmfTracker {
    /// Call identifier associated with the tracked RTP port.
    pub call_id: String,
    /// Negotiated telephone-event payload type.
    pub payload_type: u8,
    /// Timestamp of the most recently emitted logical event.
    pub last_timestamp: Option<u32>,
}

impl DtmfTracker {
    /// Creates tracking state for one RTP port.
    pub fn new(call_id: impl Into<String>, payload_type: u8) -> Self {
        Self {
            call_id: call_id.into(),
            payload_type,
            last_timestamp: None,
        }
    }

    /// Inspects an RTP packet and returns each logical DTMF event exactly once.
    pub fn observe(&mut self, packet: RtpPacketView<'_>) -> Option<DtmfObservation> {
        if packet.payload_type != self.payload_type {
            return None;
        }

        let event = TelephoneEvent::parse(packet.payload).ok()?;
        let digit = event.digit()?;
        if self.last_timestamp == Some(packet.timestamp) {
            return None;
        }
        self.last_timestamp = Some(packet.timestamp);

        Some(DtmfObservation {
            call_id: self.call_id.clone(),
            digit,
            rtp_timestamp: packet.timestamp,
            duration: event.duration,
            volume: event.volume,
            end: event.end,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rtp_core::RtpPacket;

    fn packet(payload_type: u8, timestamp: u32, payload: Vec<u8>) -> Vec<u8> {
        RtpPacket::new(payload_type, 1, timestamp, 42, payload)
            .unwrap()
            .encode()
            .unwrap()
    }

    fn observe(
        tracker: &mut DtmfTracker,
        payload_type: u8,
        timestamp: u32,
        payload: Vec<u8>,
    ) -> Option<DtmfObservation> {
        let bytes = packet(payload_type, timestamp, payload);
        tracker.observe(RtpPacketView::parse(&bytes).unwrap())
    }

    #[test]
    fn filters_payload_type_and_invalid_events_without_advancing_state() {
        let mut tracker = DtmfTracker::new("call-1", 101);

        assert_eq!(observe(&mut tracker, 100, 10, vec![5, 0, 0, 80]), None);
        assert_eq!(observe(&mut tracker, 101, 10, vec![5, 0, 0]), None);
        assert_eq!(observe(&mut tracker, 101, 10, vec![16, 0, 0, 80]), None);
        assert_eq!(tracker.last_timestamp, None);
    }

    #[test]
    fn suppresses_same_timestamp_and_accepts_new_timestamp() {
        let mut tracker = DtmfTracker::new("call-1", 101);

        assert!(observe(&mut tracker, 101, 10, vec![5, 0, 0, 80]).is_some());
        assert_eq!(observe(&mut tracker, 101, 10, vec![5, 0x80, 1, 64]), None);
        assert!(observe(&mut tracker, 101, 20, vec![6, 0x80, 1, 64]).is_some());
    }

    #[test]
    fn preserves_observation_audit_fields() {
        let mut tracker = DtmfTracker::new("call-fields", 101);
        let observation = observe(&mut tracker, 101, 7_200, vec![11, 0x8a, 1, 64]).unwrap();

        assert_eq!(observation.call_id, "call-fields");
        assert_eq!(observation.digit, '#');
        assert_eq!(observation.rtp_timestamp, 7_200);
        assert_eq!(observation.duration, 320);
        assert_eq!(observation.volume, 10);
        assert!(observation.end);
    }
}
