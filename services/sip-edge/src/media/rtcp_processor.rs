use crate::media::metrics::RtcpQualitySnapshot;
use crate::media::utils::compact_ntp_middle_32_now;
use crate::media::MediaConfig;
use rtp_core::{RtcpPacket, RtpPacketView};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MediaPacketKind {
    Rtp,
    Rtcp,
}

impl MediaPacketKind {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Rtp => "RTP",
            Self::Rtcp => "RTCP",
        }
    }

    pub(crate) fn inspect<'a>(
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
pub(crate) struct MediaPacketSummary<'a> {
    pub(crate) rtp_packet: Option<RtpPacketView<'a>>,
    pub(crate) rtcp_quality: RtcpQualitySnapshot,
}

pub(crate) fn rtcp_summary(
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

pub(crate) fn rtcp_port_for(rtp_port: u16) -> Option<u16> {
    rtp_port.checked_add(1)
}

pub(crate) fn rtp_port_for(relay_port: u16) -> Option<u16> {
    if relay_port % 2 == 0 {
        Some(relay_port)
    } else {
        relay_port.checked_sub(1)
    }
}

pub(crate) fn next_rtp_port(port: u16, config: &MediaConfig) -> u16 {
    match port.checked_add(2) {
        Some(next) if next <= config.port_max => next,
        _ => config.port_min,
    }
}
