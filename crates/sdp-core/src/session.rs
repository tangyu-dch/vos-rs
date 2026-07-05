use crate::{SdpError, SdpResult};
use std::{collections::HashSet, fmt, str::FromStr};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RtpEndpoint {
    pub address: String,
    pub port: u16,
}

impl RtpEndpoint {
    pub fn new(address: impl Into<String>, port: u16) -> Self {
        Self {
            address: address.into(),
            port,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaDescription {
    pub media_type: String,
    pub port: u16,
    pub protocol: String,
    pub formats: Vec<String>,
    pub connection_address: Option<String>,
    media_line_index: usize,
    connection_line_index: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioFormat {
    pub payload_type: String,
    pub encoding_name: Option<String>,
    pub clock_rate: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionDescription {
    lines: Vec<String>,
    session_connection: Option<ConnectionAddress>,
    session_connection_line_index: Option<usize>,
    media: Vec<MediaDescription>,
}

impl SessionDescription {
    pub fn parse(input: &str) -> SdpResult<Self> {
        input.parse()
    }

    pub fn media(&self) -> &[MediaDescription] {
        &self.media
    }

    pub fn first_audio_rtp_endpoint(&self) -> SdpResult<RtpEndpoint> {
        let media = self.first_audio_rtp_media()?;
        let address = media
            .connection_address
            .as_ref()
            .or_else(|| {
                self.session_connection
                    .as_ref()
                    .map(|connection| &connection.address)
            })
            .ok_or(SdpError::MissingConnectionAddress)?;

        Ok(RtpEndpoint::new(address.clone(), media.port))
    }

    pub fn rewrite_first_audio_rtp_endpoint(&mut self, endpoint: RtpEndpoint) -> SdpResult<()> {
        let media_index = self.first_audio_rtp_media_index()?;
        self.rewrite_media_line(media_index, endpoint.port);
        self.rewrite_connection_line(media_index, endpoint.address);
        Ok(())
    }

    pub fn first_audio_rtp_formats(&self) -> SdpResult<Vec<AudioFormat>> {
        let media_index = self.first_audio_rtp_media_index()?;
        let media = &self.media[media_index];
        Ok(media
            .formats
            .iter()
            .map(|payload_type| {
                let (encoding_name, clock_rate) = self
                    .rtpmap_for_payload(media_index, payload_type)
                    .map(|rtpmap| (Some(rtpmap.encoding_name), Some(rtpmap.clock_rate)))
                    .unwrap_or_else(|| static_audio_format(payload_type));

                AudioFormat {
                    payload_type: payload_type.clone(),
                    encoding_name,
                    clock_rate,
                }
            })
            .collect())
    }

    pub fn retain_first_audio_rtp_payloads(&mut self, payloads: &[String]) -> SdpResult<()> {
        let media_index = self.first_audio_rtp_media_index()?;
        let keep = payloads.iter().cloned().collect::<HashSet<_>>();
        let media = &self.media[media_index];
        let retained = media
            .formats
            .iter()
            .filter(|payload| keep.contains(payload.as_str()))
            .cloned()
            .collect::<Vec<_>>();

        if retained.is_empty() {
            return Err(SdpError::MissingCompatibleAudioCodec);
        }

        // Fast path: if no payloads were removed, skip the expensive rebuild
        if retained.len() == media.formats.len() {
            return Ok(());
        }

        let removed = media
            .formats
            .iter()
            .filter(|payload| !keep.contains(payload.as_str()))
            .cloned()
            .collect::<HashSet<_>>();
        self.media[media_index].formats = retained;
        self.rewrite_media_line(media_index, self.media[media_index].port);

        let section_start = self.media[media_index].media_line_index + 1;
        let section_end = self
            .media
            .get(media_index + 1)
            .map(|media| media.media_line_index)
            .unwrap_or(self.lines.len());

        self.lines = self
            .lines
            .iter()
            .enumerate()
            .filter_map(|(index, line)| {
                let is_removed_payload_attr = index >= section_start
                    && index < section_end
                    && payload_attribute(line)
                        .map(|payload| removed.contains(payload))
                        .unwrap_or(false);
                (!is_removed_payload_attr).then(|| line.clone())
            })
            .collect();

        // Rebuild media descriptions from the modified lines instead of full re-parse
        self.media.clear();
        self.session_connection = None;
        self.session_connection_line_index = None;
        for (line_index, line) in self.lines.iter().enumerate() {
            let trimmed = line.trim_end_matches('\r');
            if trimmed.is_empty() {
                continue;
            }
            if let Ok((kind, value)) = parse_line(trimmed) {
                match kind {
                    'c' => {
                        if let Ok(connection) = parse_connection_line(value, trimmed) {
                            if let Some(media) = self.media.last_mut() {
                                media.connection_address = Some(connection.address.clone());
                                media.connection_line_index = Some(line_index);
                            } else {
                                self.session_connection = Some(connection);
                                self.session_connection_line_index = Some(line_index);
                            }
                        }
                    }
                    'm' => {
                        if let Ok(media_desc) = parse_media_line(value, trimmed, line_index) {
                            self.media.push(media_desc);
                        }
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }

    fn first_audio_rtp_media(&self) -> SdpResult<&MediaDescription> {
        let index = self.first_audio_rtp_media_index()?;
        Ok(&self.media[index])
    }

    fn first_audio_rtp_media_index(&self) -> SdpResult<usize> {
        self.media
            .iter()
            .position(|media| {
                media.media_type.eq_ignore_ascii_case("audio")
                    && media.protocol.to_ascii_uppercase().contains("RTP")
            })
            .ok_or(SdpError::MissingAudioRtpMedia)
    }

    fn rtpmap_for_payload(&self, media_index: usize, payload_type: &str) -> Option<RtpMap> {
        let start = self.media[media_index].media_line_index + 1;
        let end = self
            .media
            .get(media_index + 1)
            .map(|media| media.media_line_index)
            .unwrap_or(self.lines.len());

        self.lines[start..end]
            .iter()
            .filter_map(|line| parse_rtpmap(line))
            .find(|rtpmap| rtpmap.payload_type == payload_type)
    }

    fn rewrite_media_line(&mut self, media_index: usize, port: u16) {
        let media = &mut self.media[media_index];
        media.port = port;
        self.lines[media.media_line_index] = format!(
            "m={} {} {} {}",
            media.media_type,
            media.port,
            media.protocol,
            media.formats.join(" ")
        );
    }

    fn rewrite_connection_line(&mut self, media_index: usize, address: String) {
        if let Some(line_index) = self.media[media_index].connection_line_index {
            let connection = ConnectionAddress::new("IN", address_type_for(&address), address);
            self.lines[line_index] = connection.to_line();
            self.media[media_index].connection_address = Some(connection.address);
            return;
        }

        if let Some(line_index) = self.session_connection_line_index {
            let connection = if let Some(existing) = self.session_connection.as_ref() {
                existing.with_address(address)
            } else {
                ConnectionAddress::new("IN", "IP4", address)
            };
            self.lines[line_index] = connection.to_line();
            self.session_connection = Some(connection);
            return;
        }

        let insert_at = self.media[media_index].media_line_index + 1;
        let connection = ConnectionAddress::new("IN", address_type_for(&address), address);
        self.lines.insert(insert_at, connection.to_line());
        self.shift_indices_after_insert(insert_at);
        self.media[media_index].connection_line_index = Some(insert_at);
        self.media[media_index].connection_address = Some(connection.address);
    }

    fn shift_indices_after_insert(&mut self, insert_at: usize) {
        if let Some(line_index) = &mut self.session_connection_line_index {
            if *line_index >= insert_at {
                *line_index += 1;
            }
        }

        for media in &mut self.media {
            if media.media_line_index >= insert_at {
                media.media_line_index += 1;
            }
            if let Some(line_index) = &mut media.connection_line_index {
                if *line_index >= insert_at {
                    *line_index += 1;
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RtpMap {
    payload_type: String,
    encoding_name: String,
    clock_rate: u32,
}

impl FromStr for SessionDescription {
    type Err = SdpError;

    fn from_str(input: &str) -> SdpResult<Self> {
        let mut description = Self {
            lines: Vec::new(),
            session_connection: None,
            session_connection_line_index: None,
            media: Vec::new(),
        };

        for raw_line in input.lines() {
            let line = raw_line.trim_end_matches('\r');
            if line.is_empty() {
                continue;
            }

            let (kind, value) = parse_line(line)?;
            let line_index = description.lines.len();
            description.lines.push(line.to_string());

            match kind {
                'c' => {
                    let connection = parse_connection_line(value, line)?;
                    if let Some(media) = description.media.last_mut() {
                        media.connection_address = Some(connection.address.clone());
                        media.connection_line_index = Some(line_index);
                    } else {
                        description.session_connection = Some(connection);
                        description.session_connection_line_index = Some(line_index);
                    }
                }
                'm' => {
                    description
                        .media
                        .push(parse_media_line(value, line, line_index)?);
                }
                _ => {}
            }
        }

        Ok(description)
    }
}

impl fmt::Display for SessionDescription {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for line in &self.lines {
            f.write_str(line)?;
            f.write_str("\r\n")?;
        }
        Ok(())
    }
}

impl SessionDescription {
    /// Serialize directly to bytes, avoiding the String intermediate allocation.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(self.lines.iter().map(|l| l.len() + 2).sum());
        for line in &self.lines {
            buf.extend_from_slice(line.as_bytes());
            buf.extend_from_slice(b"\r\n");
        }
        buf
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ConnectionAddress {
    network_type: String,
    address_type: String,
    address: String,
}

impl ConnectionAddress {
    fn new(
        network_type: impl Into<String>,
        address_type: impl Into<String>,
        address: impl Into<String>,
    ) -> Self {
        Self {
            network_type: network_type.into(),
            address_type: address_type.into(),
            address: address.into(),
        }
    }

    fn with_address(&self, address: String) -> Self {
        Self {
            network_type: self.network_type.clone(),
            address_type: address_type_for(&address).to_string(),
            address,
        }
    }

    fn to_line(&self) -> String {
        format!(
            "c={} {} {}",
            self.network_type, self.address_type, self.address
        )
    }
}

fn parse_line(line: &str) -> SdpResult<(char, &str)> {
    let (kind, value) = line
        .split_once('=')
        .ok_or_else(|| SdpError::InvalidLine(line.to_string()))?;
    let mut chars = kind.chars();
    let Some(kind) = chars.next() else {
        return Err(SdpError::InvalidLine(line.to_string()));
    };
    if chars.next().is_some() {
        return Err(SdpError::InvalidLine(line.to_string()));
    }
    Ok((kind, value))
}

fn parse_connection_line(value: &str, line: &str) -> SdpResult<ConnectionAddress> {
    let parts = value.split_whitespace().collect::<Vec<_>>();
    if parts.len() != 3 {
        return Err(SdpError::InvalidConnectionLine(line.to_string()));
    }

    Ok(ConnectionAddress::new(parts[0], parts[1], parts[2]))
}

fn parse_media_line(value: &str, line: &str, line_index: usize) -> SdpResult<MediaDescription> {
    let parts = value.split_whitespace().collect::<Vec<_>>();
    if parts.len() < 4 {
        return Err(SdpError::InvalidMediaLine(line.to_string()));
    }

    let port = parts[1]
        .split_once('/')
        .map(|(port, _)| port)
        .unwrap_or(parts[1])
        .parse::<u16>()
        .map_err(|_| SdpError::InvalidPort(parts[1].to_string()))?;

    Ok(MediaDescription {
        media_type: parts[0].to_string(),
        port,
        protocol: parts[2].to_string(),
        formats: parts[3..].iter().map(|value| value.to_string()).collect(),
        connection_address: None,
        media_line_index: line_index,
        connection_line_index: None,
    })
}

fn address_type_for(address: &str) -> &'static str {
    if address.contains(':') {
        "IP6"
    } else {
        "IP4"
    }
}

fn parse_rtpmap(line: &str) -> Option<RtpMap> {
    let value = line.strip_prefix("a=rtpmap:")?;
    let (payload_type, encoding) = value.split_once(char::is_whitespace)?;
    let mut parts = encoding.split('/');
    let encoding_name = parts.next()?.trim();
    let clock_rate = parts.next()?.trim().parse::<u32>().ok()?;

    Some(RtpMap {
        payload_type: payload_type.trim().to_string(),
        encoding_name: encoding_name.to_string(),
        clock_rate,
    })
}

fn static_audio_format(payload_type: &str) -> (Option<String>, Option<u32>) {
    match payload_type {
        "0" => (Some("PCMU".to_string()), Some(8_000)),
        "8" => (Some("PCMA".to_string()), Some(8_000)),
        _ => (None, None),
    }
}

fn payload_attribute(line: &str) -> Option<&str> {
    ["a=rtpmap:", "a=fmtp:", "a=rtcp-fb:"]
        .into_iter()
        .find_map(|prefix| line.strip_prefix(prefix))
        .and_then(|value| value.split_whitespace().next())
        .filter(|payload| *payload != "*")
}
