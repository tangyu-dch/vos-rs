//! RTP/RTCP packet inspection and paired-port helpers.

use crate::config::MediaConfig;
use crate::metrics::RtcpQualitySnapshot;
use crate::time::compact_ntp_middle_32_now;
use rtp_core::{RtcpPacket, RtpPacketView};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaPacketKind {
    Rtp,
    Rtcp,
}

impl MediaPacketKind {
    /// Returns the protocol label used in logs and metrics.
    pub fn label(self) -> &'static str {
        match self {
            Self::Rtp => "RTP",
            Self::Rtcp => "RTCP",
        }
    }

    /// Parses a packet into the summary required by the relay hot path.
    pub fn inspect<'a>(
        self,
        packet: &'a [u8],
    ) -> Result<MediaPacketSummary<'a>, rtp_core::RtpError> {
        match self {
            Self::Rtp => RtpPacketView::parse(packet).map(|rtp_packet| MediaPacketSummary {
                rtp_packet: Some(rtp_packet),
                ..MediaPacketSummary::default()
            }),
            Self::Rtcp => {
                let packets = RtcpPacket::parse_compound(packet)?;
                rtcp_summary(&packets)
            }
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MediaPacketSummary<'a> {
    pub rtp_packet: Option<RtpPacketView<'a>>,
    pub rtcp_quality: RtcpQualitySnapshot,
}

/// Aggregates parsed RTCP sender and receiver reports into relay quality metrics.
pub fn rtcp_summary(
    packets: &[RtcpPacket],
) -> Result<MediaPacketSummary<'static>, rtp_core::RtpError> {
    let mut rtcp_quality = RtcpQualitySnapshot::default();
    let arrival_ntp_middle_32 = compact_ntp_middle_32_now();

    for packet in packets {
        if let Some(report) = packet.sender_report()? {
            rtcp_quality.reports += 1;
            rtcp_quality.sender_reports += 1;
            for block in &report.report_blocks {
                rtcp_quality.record_report_block(block, arrival_ntp_middle_32);
            }
            continue;
        }

        if let Some(report) = packet.receiver_report()? {
            rtcp_quality.reports += 1;
            rtcp_quality.receiver_reports += 1;
            for block in &report.report_blocks {
                rtcp_quality.record_report_block(block, arrival_ntp_middle_32);
            }
        }
    }

    Ok(MediaPacketSummary {
        rtp_packet: None,
        rtcp_quality,
    })
}

/// Returns the RTCP port paired with an RTP port.
pub fn rtcp_port_for(rtp_port: u16) -> Option<u16> {
    rtp_port.checked_add(1)
}

/// Normalizes an RTP or RTCP relay port to the corresponding RTP port.
pub fn rtp_port_for(relay_port: u16) -> Option<u16> {
    if relay_port % 2 == 0 {
        Some(relay_port)
    } else {
        relay_port.checked_sub(1)
    }
}

/// Advances to the next configured RTP port, wrapping at the range boundary.
pub fn next_rtp_port(port: u16, config: &MediaConfig) -> u16 {
    match port.checked_add(2) {
        Some(next) if next <= config.port_max => next,
        _ => config.port_min,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inspects_rtp_packets() {
        let encoded = rtp_core::RtpPacket::new(0, 7, 160, 42, vec![0xff])
            .unwrap()
            .encode()
            .unwrap();
        let summary = MediaPacketKind::Rtp.inspect(&encoded).unwrap();

        assert_eq!(summary.rtp_packet.unwrap().sequence_number, 7);
        assert_eq!(summary.rtcp_quality, RtcpQualitySnapshot::default());
    }

    #[test]
    fn maps_and_wraps_paired_ports() {
        let config = MediaConfig::new("127.0.0.1", 40_000, 40_004);

        assert_eq!(rtcp_port_for(40_000), Some(40_001));
        assert_eq!(rtcp_port_for(u16::MAX), None);
        assert_eq!(rtp_port_for(40_001), Some(40_000));
        assert_eq!(next_rtp_port(40_004, &config), 40_000);
    }
}
