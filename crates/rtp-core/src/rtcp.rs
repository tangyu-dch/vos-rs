use crate::{RtpError, RtpResult, RTP_VERSION};

const RTCP_HEADER_LEN: usize = 4;
const SENDER_REPORT_FIXED_PAYLOAD_LEN: usize = 24;
const RECEIVER_REPORT_FIXED_PAYLOAD_LEN: usize = 4;
const REPORT_BLOCK_LEN: usize = 24;
const MAX_RTCP_COUNT: u8 = 31;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RtcpPacketType {
    SenderReport,
    ReceiverReport,
    SourceDescription,
    Goodbye,
    ApplicationDefined,
    TransportLayerFeedback,
    PayloadSpecificFeedback,
    ExtendedReport,
    Other(u8),
}

impl RtcpPacketType {
    pub fn as_u8(self) -> u8 {
        match self {
            Self::SenderReport => 200,
            Self::ReceiverReport => 201,
            Self::SourceDescription => 202,
            Self::Goodbye => 203,
            Self::ApplicationDefined => 204,
            Self::TransportLayerFeedback => 205,
            Self::PayloadSpecificFeedback => 206,
            Self::ExtendedReport => 207,
            Self::Other(packet_type) => packet_type,
        }
    }
}

impl From<u8> for RtcpPacketType {
    fn from(packet_type: u8) -> Self {
        match packet_type {
            200 => Self::SenderReport,
            201 => Self::ReceiverReport,
            202 => Self::SourceDescription,
            203 => Self::Goodbye,
            204 => Self::ApplicationDefined,
            205 => Self::TransportLayerFeedback,
            206 => Self::PayloadSpecificFeedback,
            207 => Self::ExtendedReport,
            packet_type => Self::Other(packet_type),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RtcpPacket {
    pub count: u8,
    pub packet_type: RtcpPacketType,
    pub payload: Vec<u8>,
    pub padding_len: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RtcpReportBlock {
    pub ssrc: u32,
    pub fraction_lost: u8,
    pub cumulative_lost: i32,
    pub extended_highest_sequence_number: u32,
    pub interarrival_jitter: u32,
    pub last_sender_report: u32,
    pub delay_since_last_sender_report: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RtcpSenderReport {
    pub sender_ssrc: u32,
    pub ntp_timestamp_msw: u32,
    pub ntp_timestamp_lsw: u32,
    pub rtp_timestamp: u32,
    pub sender_packet_count: u32,
    pub sender_octet_count: u32,
    pub report_blocks: Vec<RtcpReportBlock>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RtcpReceiverReport {
    pub reporter_ssrc: u32,
    pub report_blocks: Vec<RtcpReportBlock>,
}

impl RtcpPacket {
    pub fn new(count: u8, packet_type: RtcpPacketType, payload: Vec<u8>) -> RtpResult<Self> {
        validate_count(count)?;
        Ok(Self {
            count,
            packet_type,
            payload,
            padding_len: 0,
        })
    }

    pub fn parse(raw: &[u8]) -> RtpResult<Self> {
        let packet_len = packet_len(raw)?;
        if raw.len() != packet_len {
            return Err(RtpError::RtcpInvalidLength);
        }

        parse_one(raw)
    }

    pub fn parse_compound(raw: &[u8]) -> RtpResult<Vec<Self>> {
        let mut packets = Vec::new();
        let mut offset = 0;

        while offset < raw.len() {
            let packet_len = packet_len(&raw[offset..])?;
            let packet = parse_one(&raw[offset..offset + packet_len])?;
            packets.push(packet);
            offset += packet_len;
        }

        if packets.is_empty() {
            return Err(RtpError::RtcpPacketTooShort);
        }

        Ok(packets)
    }

    pub fn encode(&self) -> RtpResult<Vec<u8>> {
        validate_count(self.count)?;

        let total_len = RTCP_HEADER_LEN + self.payload.len() + usize::from(self.padding_len);
        if total_len % 4 != 0 || total_len < RTCP_HEADER_LEN {
            return Err(RtpError::RtcpInvalidLength);
        }
        if self.padding_len == 0 && self.payload.len() % 4 != 0 {
            return Err(RtpError::RtcpInvalidLength);
        }
        if self.padding_len > 0 && usize::from(self.padding_len) > total_len - RTCP_HEADER_LEN {
            return Err(RtpError::RtcpInvalidPadding);
        }

        let length_words =
            u16::try_from(total_len / 4 - 1).map_err(|_| RtpError::RtcpInvalidLength)?;
        let mut bytes = Vec::with_capacity(total_len);

        let mut first = RTP_VERSION << 6;
        if self.padding_len > 0 {
            first |= 0x20;
        }
        first |= self.count;
        bytes.push(first);
        bytes.push(self.packet_type.as_u8());
        bytes.extend_from_slice(&length_words.to_be_bytes());
        bytes.extend_from_slice(&self.payload);

        if self.padding_len > 0 {
            if self.padding_len == 1 {
                bytes.push(1);
            } else {
                bytes.extend(std::iter::repeat_n(0, usize::from(self.padding_len - 1)));
                bytes.push(self.padding_len);
            }
        }

        Ok(bytes)
    }

    pub fn sender_report(&self) -> RtpResult<Option<RtcpSenderReport>> {
        if self.packet_type != RtcpPacketType::SenderReport {
            return Ok(None);
        }
        if self.payload.len() != SENDER_REPORT_FIXED_PAYLOAD_LEN + report_blocks_len(self.count) {
            return Err(RtpError::RtcpInvalidReportLength);
        }

        Ok(Some(RtcpSenderReport {
            sender_ssrc: read_u32(&self.payload, 0)?,
            ntp_timestamp_msw: read_u32(&self.payload, 4)?,
            ntp_timestamp_lsw: read_u32(&self.payload, 8)?,
            rtp_timestamp: read_u32(&self.payload, 12)?,
            sender_packet_count: read_u32(&self.payload, 16)?,
            sender_octet_count: read_u32(&self.payload, 20)?,
            report_blocks: parse_report_blocks(
                self.count,
                &self.payload[SENDER_REPORT_FIXED_PAYLOAD_LEN..],
            )?,
        }))
    }

    pub fn receiver_report(&self) -> RtpResult<Option<RtcpReceiverReport>> {
        if self.packet_type != RtcpPacketType::ReceiverReport {
            return Ok(None);
        }
        if self.payload.len() != RECEIVER_REPORT_FIXED_PAYLOAD_LEN + report_blocks_len(self.count) {
            return Err(RtpError::RtcpInvalidReportLength);
        }

        Ok(Some(RtcpReceiverReport {
            reporter_ssrc: read_u32(&self.payload, 0)?,
            report_blocks: parse_report_blocks(
                self.count,
                &self.payload[RECEIVER_REPORT_FIXED_PAYLOAD_LEN..],
            )?,
        }))
    }
}

fn parse_one(raw: &[u8]) -> RtpResult<RtcpPacket> {
    let packet_len = packet_len(raw)?;
    let version = raw[0] >> 6;
    if version != RTP_VERSION {
        return Err(RtpError::UnsupportedVersion(version));
    }

    let has_padding = raw[0] & 0x20 != 0;
    let count = raw[0] & 0x1f;
    let packet_type = RtcpPacketType::from(raw[1]);

    let padding_len = if has_padding {
        let padding_len = *raw.last().ok_or(RtpError::RtcpInvalidPadding)?;
        if padding_len == 0 || usize::from(padding_len) > packet_len - RTCP_HEADER_LEN {
            return Err(RtpError::RtcpInvalidPadding);
        }
        padding_len
    } else {
        0
    };

    let payload_end = packet_len - usize::from(padding_len);
    Ok(RtcpPacket {
        count,
        packet_type,
        payload: raw[RTCP_HEADER_LEN..payload_end].to_vec(),
        padding_len,
    })
}

fn packet_len(raw: &[u8]) -> RtpResult<usize> {
    if raw.len() < RTCP_HEADER_LEN {
        return Err(RtpError::RtcpPacketTooShort);
    }

    let length_words = usize::from(u16::from_be_bytes([raw[2], raw[3]]));
    let packet_len = (length_words + 1)
        .checked_mul(4)
        .ok_or(RtpError::RtcpInvalidLength)?;
    if packet_len < RTCP_HEADER_LEN || raw.len() < packet_len {
        return Err(RtpError::RtcpInvalidLength);
    }

    Ok(packet_len)
}

fn validate_count(count: u8) -> RtpResult<()> {
    if count > MAX_RTCP_COUNT {
        Err(RtpError::RtcpCountOutOfRange(count))
    } else {
        Ok(())
    }
}

fn parse_report_blocks(count: u8, raw: &[u8]) -> RtpResult<Vec<RtcpReportBlock>> {
    if raw.len() != report_blocks_len(count) {
        return Err(RtpError::RtcpInvalidReportLength);
    }

    raw.chunks_exact(REPORT_BLOCK_LEN)
        .map(parse_report_block)
        .collect()
}

fn parse_report_block(raw: &[u8]) -> RtpResult<RtcpReportBlock> {
    if raw.len() != REPORT_BLOCK_LEN {
        return Err(RtpError::RtcpInvalidReportLength);
    }

    Ok(RtcpReportBlock {
        ssrc: read_u32(raw, 0)?,
        fraction_lost: raw[4],
        cumulative_lost: read_i24(raw, 5)?,
        extended_highest_sequence_number: read_u32(raw, 8)?,
        interarrival_jitter: read_u32(raw, 12)?,
        last_sender_report: read_u32(raw, 16)?,
        delay_since_last_sender_report: read_u32(raw, 20)?,
    })
}

fn report_blocks_len(count: u8) -> usize {
    usize::from(count) * REPORT_BLOCK_LEN
}

fn read_u32(raw: &[u8], offset: usize) -> RtpResult<u32> {
    let bytes = raw
        .get(offset..offset + 4)
        .ok_or(RtpError::RtcpInvalidReportLength)?;
    Ok(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn read_i24(raw: &[u8], offset: usize) -> RtpResult<i32> {
    let bytes = raw
        .get(offset..offset + 3)
        .ok_or(RtpError::RtcpInvalidReportLength)?;
    let unsigned = i32::from(bytes[0]) << 16 | i32::from(bytes[1]) << 8 | i32::from(bytes[2]);
    if unsigned & 0x80_0000 != 0 {
        Ok(unsigned | !0xFF_FFFF)
    } else {
        Ok(unsigned)
    }
}
