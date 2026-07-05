use crate::{RtpError, RtpResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TelephoneEvent {
    pub event: u8,
    pub end: bool,
    pub reserved: bool,
    pub volume: u8,
    pub duration: u16,
}

impl TelephoneEvent {
    pub fn parse(payload: &[u8]) -> RtpResult<Self> {
        if payload.len() < 4 {
            return Err(RtpError::TelephoneEventPayloadTooShort);
        }

        let flags = payload[1];
        Ok(Self {
            event: payload[0],
            end: flags & 0x80 != 0,
            reserved: flags & 0x40 != 0,
            volume: flags & 0x3f,
            duration: u16::from_be_bytes([payload[2], payload[3]]),
        })
    }

    pub fn digit(self) -> Option<char> {
        match self.event {
            0..=9 => Some((b'0' + self.event) as char),
            10 => Some('*'),
            11 => Some('#'),
            12..=15 => Some((b'A' + (self.event - 12)) as char),
            _ => None,
        }
    }
}
