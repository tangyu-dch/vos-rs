use crate::{SipParseError, SipResult};
use std::fmt;
use std::net::IpAddr;
use std::str::FromStr;

/// RFC 3581 compliant Via header representation.
///
/// Example: `SIP/2.0/UDP 192.168.1.100:5060;branch=z9hG4bK123;rport;received=203.0.113.1`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViaHeader {
    pub protocol_name: String,
    pub protocol_version: String,
    pub transport: String,
    pub host: String,
    pub port: Option<u16>,
    pub branch: Option<String>,
    pub rport: Option<Option<u16>>,
    pub received: Option<IpAddr>,
    pub other_params: Vec<(String, Option<String>)>,
}

impl ViaHeader {
    /// Parse a Via header value string into a `ViaHeader` struct.
    pub fn parse(raw: &str) -> SipResult<Self> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(SipParseError::InvalidHeaderLine(raw.to_string()));
        }

        let mut parts = trimmed.splitn(2, ';');
        let main_part = parts.next().unwrap_or("").trim();
        let params_part = parts.next();

        let mut main_tokens = main_part.split_whitespace();
        let proto_token = main_tokens
            .next()
            .ok_or_else(|| SipParseError::InvalidHeaderLine(raw.to_string()))?;
        let host_port_token = main_tokens
            .next()
            .ok_or_else(|| SipParseError::InvalidHeaderLine(raw.to_string()))?;

        let proto_parts: Vec<&str> = proto_token.split('/').collect();
        if proto_parts.len() != 3 {
            return Err(SipParseError::InvalidHeaderLine(raw.to_string()));
        }

        let protocol_name = proto_parts[0].to_string();
        let protocol_version = proto_parts[1].to_string();
        let transport = proto_parts[2].to_string();

        let (host, port) = parse_host_port(host_port_token)?;

        let mut branch = None;
        let mut rport = None;
        let mut received = None;
        let mut other_params = Vec::new();

        if let Some(params_str) = params_part {
            for param in params_str.split(';') {
                let p = param.trim();
                if p.is_empty() {
                    continue;
                }
                let mut kv = p.splitn(2, '=');
                let key = kv.next().unwrap_or("").trim();
                let val = kv.next().map(|v| v.trim());

                match key.to_ascii_lowercase().as_str() {
                    "branch" => {
                        branch = val.map(|v| v.to_string());
                    }
                    "rport" => match val {
                        Some(v) if !v.is_empty() => {
                            let parsed_port = v.parse::<u16>().map_err(|_| {
                                SipParseError::InvalidHeaderLine(format!("Invalid rport: {v}"))
                            })?;
                            rport = Some(Some(parsed_port));
                        }
                        _ => {
                            rport = Some(None);
                        }
                    },
                    "received" => {
                        if let Some(v) = val {
                            let ip = IpAddr::from_str(v).map_err(|_| {
                                SipParseError::InvalidHeaderLine(format!(
                                    "Invalid received IP: {v}"
                                ))
                            })?;
                            received = Some(ip);
                        }
                    }
                    _ => {
                        other_params.push((key.to_string(), val.map(|v| v.to_string())));
                    }
                }
            }
        }

        Ok(ViaHeader {
            protocol_name,
            protocol_version,
            transport,
            host,
            port,
            branch,
            rport,
            received,
            other_params,
        })
    }

    /// Process RFC 3581 rules when receiving a request from a remote socket address.
    /// If `rport` parameter is present in Via, populate `rport` with `remote_port`
    /// and `received` with `remote_ip`.
    pub fn apply_rfc3581(&mut self, remote_ip: IpAddr, remote_port: u16) {
        if self.rport.is_some() {
            self.rport = Some(Some(remote_port));
            self.received = Some(remote_ip);
        } else if self.host != remote_ip.to_string() {
            self.received = Some(remote_ip);
        }
    }
}

fn parse_host_port(raw: &str) -> SipResult<(String, Option<u16>)> {
    if raw.starts_with('[') {
        if let Some(bracket_end) = raw.find(']') {
            let host = raw[1..bracket_end].to_string();
            let rest = &raw[bracket_end + 1..];
            let port = if let Some(colon_idx) = rest.find(':') {
                let port_str = &rest[colon_idx + 1..];
                Some(
                    port_str
                        .parse::<u16>()
                        .map_err(|_| SipParseError::InvalidHeaderLine(raw.to_string()))?,
                )
            } else {
                None
            };
            return Ok((host, port));
        }
    }

    if let Some(colon_idx) = raw.rfind(':') {
        let host = raw[..colon_idx].to_string();
        let port_str = &raw[colon_idx + 1..];
        let port = port_str
            .parse::<u16>()
            .map_err(|_| SipParseError::InvalidHeaderLine(raw.to_string()))?;
        Ok((host, Some(port)))
    } else {
        Ok((raw.to_string(), None))
    }
}

impl fmt::Display for ViaHeader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}/{}/{} {}",
            self.protocol_name, self.protocol_version, self.transport, self.host
        )?;
        if let Some(port) = self.port {
            write!(f, ":{port}")?;
        }
        if let Some(ref branch) = self.branch {
            write!(f, ";branch={branch}")?;
        }
        if let Some(ref rport_opt) = self.rport {
            match rport_opt {
                Some(port) => write!(f, ";rport={port}")?,
                None => write!(f, ";rport")?,
            }
        }
        if let Some(ref received) = self.received {
            write!(f, ";received={received}")?;
        }
        for (k, v) in &self.other_params {
            match v {
                Some(val) => write!(f, ";{k}={val}")?,
                None => write!(f, ";{k}")?,
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn test_via_parse_with_rport_and_rfc3581() {
        let raw = "SIP/2.0/UDP 192.168.1.100:5060;branch=z9hG4bK12345;rport";
        let mut via = ViaHeader::parse(raw).expect("via parse failed");
        assert_eq!(via.transport, "UDP");
        assert_eq!(via.host, "192.168.1.100");
        assert_eq!(via.port, Some(5060));
        assert_eq!(via.branch.as_deref(), Some("z9hG4bK12345"));
        assert_eq!(via.rport, Some(None));

        let remote_ip = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 1));
        via.apply_rfc3581(remote_ip, 62154);

        assert_eq!(via.rport, Some(Some(62154)));
        assert_eq!(via.received, Some(remote_ip));
        assert_eq!(
            via.to_string(),
            "SIP/2.0/UDP 192.168.1.100:5060;branch=z9hG4bK12345;rport=62154;received=203.0.113.1"
        );
    }
}
