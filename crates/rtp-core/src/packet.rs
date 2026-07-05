use crate::{RtpError, RtpResult};

pub const RTP_VERSION: u8 = 2;
const FIXED_HEADER_LEN: usize = 12;
const MAX_CSRC_COUNT: usize = 15;
const MAX_PAYLOAD_TYPE: u8 = 127;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RtpPacket {
    pub marker: bool,
    pub payload_type: u8,
    pub sequence_number: u16,
    pub timestamp: u32,
    pub ssrc: u32,
    pub csrcs: Vec<u32>,
    pub extension: Option<RtpHeaderExtension>,
    pub payload: Vec<u8>,
    pub padding_len: u8,
}

impl RtpPacket {
    pub fn new(
        payload_type: u8,
        sequence_number: u16,
        timestamp: u32,
        ssrc: u32,
        payload: Vec<u8>,
    ) -> RtpResult<Self> {
        validate_payload_type(payload_type)?;
        Ok(Self {
            marker: false,
            payload_type,
            sequence_number,
            timestamp,
            ssrc,
            csrcs: Vec::new(),
            extension: None,
            payload,
            padding_len: 0,
        })
    }

    pub fn parse(raw: &[u8]) -> RtpResult<Self> {
        if raw.len() < FIXED_HEADER_LEN {
            return Err(RtpError::PacketTooShort);
        }

        let version = raw[0] >> 6;
        if version != RTP_VERSION {
            return Err(RtpError::UnsupportedVersion(version));
        }

        let has_padding = raw[0] & 0x20 != 0;
        let has_extension = raw[0] & 0x10 != 0;
        let csrc_count = usize::from(raw[0] & 0x0f);
        let marker = raw[1] & 0x80 != 0;
        let payload_type = raw[1] & 0x7f;

        let mut offset = FIXED_HEADER_LEN;
        let csrc_end = offset + csrc_count * 4;
        if raw.len() < csrc_end {
            return Err(RtpError::InvalidCsrcCount);
        }

        let mut csrcs = Vec::with_capacity(csrc_count);
        while offset < csrc_end {
            csrcs.push(read_u32(raw, offset)?);
            offset += 4;
        }

        let extension = if has_extension {
            if raw.len() < offset + 4 {
                return Err(RtpError::InvalidExtensionLength);
            }

            let profile = read_u16(raw, offset)?;
            let length_words = read_u16(raw, offset + 2)?;
            offset += 4;

            let extension_len = usize::from(length_words) * 4;
            if raw.len() < offset + extension_len {
                return Err(RtpError::InvalidExtensionLength);
            }

            let data = raw[offset..offset + extension_len].to_vec();
            offset += extension_len;
            Some(RtpHeaderExtension {
                profile,
                length_words,
                data,
            })
        } else {
            None
        };

        let padding_len = if has_padding {
            let padding_len = *raw.last().ok_or(RtpError::InvalidPadding)?;
            if padding_len == 0 || usize::from(padding_len) > raw.len().saturating_sub(offset) {
                return Err(RtpError::InvalidPadding);
            }
            padding_len
        } else {
            0
        };

        let payload_end = raw.len() - usize::from(padding_len);
        Ok(Self {
            marker,
            payload_type,
            sequence_number: read_u16(raw, 2)?,
            timestamp: read_u32(raw, 4)?,
            ssrc: read_u32(raw, 8)?,
            csrcs,
            extension,
            payload: raw[offset..payload_end].to_vec(),
            padding_len,
        })
    }

    pub fn encode(&self) -> RtpResult<Vec<u8>> {
        validate_payload_type(self.payload_type)?;
        if self.csrcs.len() > MAX_CSRC_COUNT {
            return Err(RtpError::InvalidCsrcCount);
        }

        let extension_len = self
            .extension
            .as_ref()
            .map(|ext| ext.data.len())
            .unwrap_or(0);
        let mut bytes = Vec::with_capacity(
            FIXED_HEADER_LEN
                + self.csrcs.len() * 4
                + self.extension.as_ref().map(|_| 4).unwrap_or(0)
                + extension_len
                + self.payload.len()
                + usize::from(self.padding_len),
        );

        let mut first = RTP_VERSION << 6;
        if self.padding_len > 0 {
            first |= 0x20;
        }
        if self.extension.is_some() {
            first |= 0x10;
        }
        first |= u8::try_from(self.csrcs.len()).map_err(|_| RtpError::InvalidCsrcCount)?;
        bytes.push(first);

        bytes.push(if self.marker {
            self.payload_type | 0x80
        } else {
            self.payload_type
        });
        write_u16(&mut bytes, self.sequence_number);
        write_u32(&mut bytes, self.timestamp);
        write_u32(&mut bytes, self.ssrc);

        for csrc in &self.csrcs {
            write_u32(&mut bytes, *csrc);
        }

        if let Some(extension) = &self.extension {
            extension.validate()?;
            write_u16(&mut bytes, extension.profile);
            write_u16(&mut bytes, extension.length_words);
            bytes.extend_from_slice(&extension.data);
        }

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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RtpHeaderExtension {
    pub profile: u16,
    pub length_words: u16,
    pub data: Vec<u8>,
}

impl RtpHeaderExtension {
    pub fn new(profile: u16, data: Vec<u8>) -> RtpResult<Self> {
        if data.len() % 4 != 0 {
            return Err(RtpError::InvalidExtensionLength);
        }

        let length_words =
            u16::try_from(data.len() / 4).map_err(|_| RtpError::InvalidExtensionLength)?;
        Ok(Self {
            profile,
            length_words,
            data,
        })
    }

    fn validate(&self) -> RtpResult<()> {
        if self.data.len() % 4 != 0 || self.data.len() / 4 != usize::from(self.length_words) {
            return Err(RtpError::InvalidExtensionLength);
        }
        Ok(())
    }
}

fn validate_payload_type(payload_type: u8) -> RtpResult<()> {
    if payload_type > MAX_PAYLOAD_TYPE {
        Err(RtpError::PayloadTypeOutOfRange(payload_type))
    } else {
        Ok(())
    }
}

fn read_u16(raw: &[u8], offset: usize) -> RtpResult<u16> {
    let bytes = raw
        .get(offset..offset + 2)
        .ok_or(RtpError::PacketTooShort)?;
    Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
}

fn read_u32(raw: &[u8], offset: usize) -> RtpResult<u32> {
    let bytes = raw
        .get(offset..offset + 4)
        .ok_or(RtpError::PacketTooShort)?;
    Ok(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn write_u16(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend_from_slice(&value.to_be_bytes());
}

fn write_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_be_bytes());
}
