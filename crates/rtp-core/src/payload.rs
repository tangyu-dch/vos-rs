#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AudioCodec {
    Pcmu,
    Pcma,
    G722,
    G729,
    Opus,
}

impl AudioCodec {
    pub fn name(self) -> &'static str {
        match self {
            Self::Pcmu => "PCMU",
            Self::Pcma => "PCMA",
            Self::G722 => "G722",
            Self::G729 => "G729",
            Self::Opus => "OPUS",
        }
    }

    pub fn clock_rate(self) -> u32 {
        match self {
            Self::Pcmu | Self::Pcma | Self::G729 | Self::G722 => 8_000,
            Self::Opus => 48_000,
        }
    }

    pub fn static_payload_type(self) -> Option<u8> {
        match self {
            Self::Pcmu => Some(0),
            Self::Pcma => Some(8),
            Self::G722 => Some(9),
            Self::G729 => Some(18),
            Self::Opus => None, // Opus is dynamic
        }
    }

    pub fn from_static_payload_type(payload_type: u8) -> Option<Self> {
        match payload_type {
            0 => Some(Self::Pcmu),
            8 => Some(Self::Pcma),
            9 => Some(Self::G722),
            18 => Some(Self::G729),
            _ => None,
        }
    }

    pub fn from_rtpmap(encoding_name: &str, clock_rate: u32) -> Option<Self> {
        let name = encoding_name.to_ascii_uppercase();
        match name.as_str() {
            "PCMU" if clock_rate == 8_000 => Some(Self::Pcmu),
            "PCMA" if clock_rate == 8_000 => Some(Self::Pcma),
            "G722" if clock_rate == 8_000 => Some(Self::G722),
            "G729" if clock_rate == 8_000 => Some(Self::G729),
            "OPUS" if clock_rate == 48_000 => Some(Self::Opus),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StaticAudioPayload {
    pub payload_type: u8,
    pub codec: AudioCodec,
}

impl StaticAudioPayload {
    pub fn from_payload_type(payload_type: u8) -> Option<Self> {
        let codec = AudioCodec::from_static_payload_type(payload_type)?;
        Some(Self {
            payload_type,
            codec,
        })
    }
}
