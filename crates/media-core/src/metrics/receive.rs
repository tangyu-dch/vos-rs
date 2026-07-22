use crate::time::unix_timestamp_millis;
use rtp_core::RtpPacketView;

#[derive(Debug, Clone, Copy, Default)]
pub struct RtpReceiveStats {
    pub ssrc: u32,
    pub base_sequence: u16,
    pub highest_sequence: u16,
    pub received: u32,
    pub jitter: u32,
    pub last_transit: Option<i64>,
    pub last_report_unix_ms: u128,
}

impl RtpReceiveStats {
    pub fn observe(&mut self, packet: RtpPacketView<'_>) {
        if self.received == 0 {
            self.ssrc = packet.ssrc;
            self.base_sequence = packet.sequence_number;
            self.highest_sequence = packet.sequence_number;
        } else if packet.sequence_number.wrapping_sub(self.highest_sequence) < 0x8000 {
            self.highest_sequence = packet.sequence_number;
        }
        self.received = self.received.saturating_add(1);

        let arrival_units = (unix_timestamp_millis() as i64).saturating_mul(8);
        let transit = arrival_units.saturating_sub(i64::from(packet.timestamp));
        if let Some(previous) = self.last_transit {
            let delta = (transit - previous).unsigned_abs();
            let jitter = u64::from(self.jitter);
            self.jitter =
                ((jitter.saturating_mul(15) + delta) / 16).min(u64::from(u32::MAX)) as u32;
        }
        self.last_transit = Some(transit);
    }

    pub fn receiver_report(&mut self) -> Option<Vec<u8>> {
        let now = unix_timestamp_millis();
        if self.received == 0 {
            return None;
        }
        if self.last_report_unix_ms == 0 {
            self.last_report_unix_ms = now;
            return None;
        }
        if now.saturating_sub(self.last_report_unix_ms) < 5_000 {
            return None;
        }
        self.last_report_unix_ms = now;
        let expected = u32::from(self.highest_sequence.wrapping_sub(self.base_sequence)) + 1;
        let lost = expected.saturating_sub(self.received);
        let fraction_lost = if expected == 0 {
            0
        } else {
            ((u64::from(lost) * 256) / u64::from(expected)).min(255) as u8
        };
        let mut payload = Vec::with_capacity(28);
        payload.extend_from_slice(&self.ssrc.wrapping_add(1).to_be_bytes());
        payload.extend_from_slice(&self.ssrc.to_be_bytes());
        payload.push(fraction_lost);
        let cumulative_lost = i32::try_from(lost).unwrap_or(i32::MAX).clamp(0, 0x7f_ffff);
        payload.extend_from_slice(&cumulative_lost.to_be_bytes()[1..]);
        payload.extend_from_slice(&u32::from(self.highest_sequence).to_be_bytes());
        payload.extend_from_slice(&self.jitter.to_be_bytes());
        payload.extend_from_slice(&0_u32.to_be_bytes());
        payload.extend_from_slice(&0_u32.to_be_bytes());
        rtp_core::RtcpPacket::new(1, rtp_core::RtcpPacketType::ReceiverReport, payload)
            .ok()?
            .encode()
            .ok()
    }
}
