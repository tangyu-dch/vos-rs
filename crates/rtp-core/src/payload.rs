#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AudioCodec {
    Pcmu,
    Pcma,
}

impl AudioCodec {
    pub fn name(self) -> &'static str {
        match self {
            Self::Pcmu => "PCMU",
            Self::Pcma => "PCMA",
        }
    }

    pub fn clock_rate(self) -> u32 {
        8_000
    }

    pub fn static_payload_type(self) -> u8 {
        match self {
            Self::Pcmu => 0,
            Self::Pcma => 8,
        }
    }

    pub fn from_static_payload_type(payload_type: u8) -> Option<Self> {
        match payload_type {
            0 => Some(Self::Pcmu),
            8 => Some(Self::Pcma),
            _ => None,
        }
    }

    pub fn from_rtpmap(encoding_name: &str, clock_rate: u32) -> Option<Self> {
        if clock_rate != 8_000 {
            return None;
        }

        match encoding_name.to_ascii_uppercase().as_str() {
            "PCMU" => Some(Self::Pcmu),
            "PCMA" => Some(Self::Pcma),
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
