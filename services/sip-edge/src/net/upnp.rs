use std::net::{SocketAddr, UdpSocket};
use std::time::Duration;
use tracing::{debug, info, warn};

const SSDP_ADDR: &str = "239.255.255.250:1900";
const SSDP_MSEARCH: &str = "M-SEARCH * HTTP/1.1\r\nHOST: 239.255.255.250:1900\r\nMAN: \"ssdp:discover\"\r\nST: urn:schemas-upnp-org:device:InternetGatewayDevice:1\r\nMX: 3\r\n\r\n";

pub struct UpnpGateway {
    pub control_url: String,
    pub local_ip: String,
    pub service_type: String,
}

/// Discover UPnP gateway and return its control URL.
pub fn discover_gateway() -> Option<UpnpGateway> {
    let local_ip = local_ip_address()?;

    for attempt in 1..=3 {
        match try_discover(&local_ip) {
            Some(gw) => return Some(gw),
            None => {
                if attempt < 3 {
                    debug!(attempt, "UPnP: SSDP probe failed, retrying");
                    std::thread::sleep(Duration::from_secs(1));
                }
            }
        }
    }
    None
}

fn try_discover(local_ip: &str) -> Option<UpnpGateway> {
    let sock = UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    sock.send_to(SSDP_MSEARCH.as_bytes(), SSDP_ADDR).ok()?;

    let mut buf = [0u8; 2048];
    loop {
        match sock.recv_from(&mut buf) {
            Ok((n, _)) => {
                let resp = String::from_utf8_lossy(&buf[..n]);
                if resp.contains("HTTP/1.1 200 OK") {
                    if let Some(location) = parse_header(&resp, "LOCATION") {
                        if let Some(gw) = parse_igd_description(&location, &local_ip) {
                            return Some(gw);
                        }
                    }
                }
            }
            Err(_) => break,
        }
    }
    None
}

fn parse_header(resp: &str, header: &str) -> Option<String> {
    for line in resp.lines() {
        if let Some(value) = line.strip_prefix(&format!("{header}: ")) {
            return Some(value.trim().to_string());
        }
    }
    None
}

/// Fetch the IGD device description XML and extract the WANIPConnection control URL.
fn parse_igd_description(location: &str, local_ip: &str) -> Option<UpnpGateway> {
    let url_parts = url_parse(location)?;
    let host = url_parts.0;
    let port = url_parts.1;
    let path = url_parts.2;

    let addr = format!("{host}:{port}");
    let sock =
        std::net::TcpStream::connect_timeout(&addr.parse().ok()?, Duration::from_secs(3)).ok()?;

    let request =
        format!("GET {path} HTTP/1.1\r\nHost: {host}:{port}\r\nConnection: close\r\n\r\n");
    use std::io::Write;
    let mut stream = std::io::BufWriter::new(&sock);
    stream.write_all(request.as_bytes()).ok()?;
    stream.flush().ok()?;

    drop(stream);

    let mut response = String::new();
    use std::io::Read;
    let mut reader = std::io::BufReader::new(&sock);
    reader.read_to_string(&mut response).ok()?;

    // Find SOAP action URL for WANIPConnection
    let soap_path = extract_wan_ip_connection_path(&response)?;
    let control_url = format!("http://{host}:{port}{soap_path}");

    info!(control_url = %control_url, "UPnP gateway discovered");

    Some(UpnpGateway {
        control_url,
        local_ip: local_ip.to_string(),
        service_type: "urn:schemas-upnp-org:service:WANIPConnection:1".to_string(),
    })
}

fn url_parse(location: &str) -> Option<(String, u16, String)> {
    let rest = location.strip_prefix("http://")?;
    let (host_port, path) = if let Some(idx) = rest.find('/') {
        (&rest[..idx], &rest[idx..])
    } else {
        (rest, "/")
    };
    let (host, port) = if let Some(idx) = host_port.rfind(':') {
        let port: u16 = host_port[idx + 1..].parse().ok()?;
        (host_port[..idx].to_string(), port)
    } else {
        (host_port.to_string(), 80)
    };
    Some((host, port, path.to_string()))
}

fn extract_wan_ip_connection_path(xml: &str) -> Option<String> {
    // Look for controlURL inside WANIPConnection service
    let mut in_wan = false;
    let mut depth = 0;
    for line in xml.lines() {
        let trimmed = line.trim();
        if trimmed.contains("WANIPConnection") || trimmed.contains("WANPPPConnection") {
            in_wan = true;
            depth = 0;
        }
        if in_wan {
            depth += 1;
            if trimmed.contains("</service>") && depth > 1 {
                in_wan = false;
            }
            if trimmed.starts_with("<controlURL>") {
                let path = trimmed
                    .trim_start_matches("<controlURL>")
                    .trim_end_matches("</controlURL>")
                    .trim();
                return Some(path.to_string());
            }
        }
    }
    None
}

fn local_ip_address() -> Option<String> {
    let sock = UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("8.8.8.8:53").ok()?;
    let addr = sock.local_addr().ok()?;
    Some(addr.ip().to_string())
}

/// Add a port mapping on the UPnP gateway.
pub fn add_port_mapping(
    gw: &UpnpGateway,
    external_port: u16,
    internal_port: u16,
    protocol: &str, // "UDP" or "TCP"
    description: &str,
    lease_secs: u32,
) -> bool {
    let body = format!(
        r#"<?xml version="1.0"?>
<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/"
            s:encodingStyle="http://schemas.xmlsoap.org/soap/encoding/">
  <s:Body>
    <u:AddPortMapping xmlns:u="{}">
      <NewRemoteHost></NewRemoteHost>
      <NewExternalPort>{}</NewExternalPort>
      <NewProtocol>{}</NewProtocol>
      <NewInternalPort>{}</NewInternalPort>
      <NewInternalClient>{}</NewInternalClient>
      <NewEnabled>1</NewEnabled>
      <NewPortMappingDescription>{}</NewPortMappingDescription>
      <NewLeaseDuration>{}</NewLeaseDuration>
    </u:AddPortMapping>
  </s:Body>
</s:Envelope>"#,
        gw.service_type,
        external_port,
        protocol,
        internal_port,
        gw.local_ip,
        description,
        lease_secs,
    );

    soap_request(gw, "AddPortMapping", &body)
}

/// Remove a port mapping from the UPnP gateway.
#[allow(dead_code)]
pub fn remove_port_mapping(gw: &UpnpGateway, external_port: u16, protocol: &str) -> bool {
    let body = format!(
        r#"<?xml version="1.0"?>
<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/"
            s:encodingStyle="http://schemas.xmlsoap.org/soap/encoding/">
  <s:Body>
    <u:DeletePortMapping xmlns:u="{}">
      <NewRemoteHost></NewRemoteHost>
      <NewExternalPort>{}</NewExternalPort>
      <NewProtocol>{}</NewProtocol>
    </u:DeletePortMapping>
  </s:Body>
</s:Envelope>"#,
        gw.service_type, external_port, protocol,
    );

    soap_request(gw, "DeletePortMapping", &body)
}

/// Get the external IP address from the UPnP gateway.
pub fn get_external_ip(gw: &UpnpGateway) -> Option<String> {
    let body = r#"<?xml version="1.0"?>
<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/"
            s:encodingStyle="http://schemas.xmlsoap.org/soap/encoding/">
  <s:Body>
    <u:GetExternalIPAddress xmlns:u="urn:schemas-upnp-org:service:WANIPConnection:1"/>
  </s:Body>
</s:Envelope>"#;

    let url_parts = url_parse(&gw.control_url)?;
    let addr: SocketAddr = format!("{}:{}", url_parts.0, url_parts.1).parse().ok()?;
    let sock = std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(3)).ok()?;

    let soap_action =
        format!("\"urn:schemas-upnp-org:service:WANIPConnection:1#GetExternalIPAddress\"");
    let request = format!(
        "POST {} HTTP/1.1\r\nHost: {}:{}\r\nContent-Type: text/xml; charset=\"utf-8\"\r\nSOAPAction: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        url_parts.2, url_parts.0, url_parts.1, soap_action, body.len(), body
    );

    use std::io::Write;
    let mut stream = std::io::BufWriter::new(&sock);
    stream.write_all(request.as_bytes()).ok()?;
    stream.flush().ok()?;
    drop(stream);

    let mut response = String::new();
    use std::io::Read;
    let mut reader = std::io::BufReader::new(&sock);
    reader.read_to_string(&mut response).ok()?;

    // Extract NewExternalIPAddress from response
    if let Some(idx) = response.find("<NewExternalIPAddress>") {
        let rest = &response[idx + 23..];
        if let Some(end) = rest.find("</NewExternalIPAddress>") {
            let ip = rest[..end].trim();
            info!(external_ip = %ip, "UPnP: got external IP address");
            return Some(ip.to_string());
        }
    }
    warn!("UPnP: failed to parse external IP from response");
    None
}

fn soap_request(gw: &UpnpGateway, action: &str, body: &str) -> bool {
    let url_parts = match url_parse(&gw.control_url) {
        Some(p) => p,
        None => {
            warn!("UPnP: failed to parse control URL");
            return false;
        }
    };

    let addr: SocketAddr = match format!("{}:{}", url_parts.0, url_parts.1).parse() {
        Ok(a) => a,
        Err(e) => {
            warn!(error = %e, "UPnP: invalid gateway address");
            return false;
        }
    };

    let sock = match std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(5)) {
        Ok(s) => {
            let _ = s.set_read_timeout(Some(Duration::from_secs(10)));
            s
        }
        Err(e) => {
            warn!(error = %e, "UPnP: failed to connect to gateway");
            return false;
        }
    };

    let soap_action = format!("\"{}#{}\"", gw.service_type, action);

    let request = format!(
        "POST {} HTTP/1.1\r\nHost: {}:{}\r\nContent-Type: text/xml; charset=\"utf-8\"\r\nSOAPAction: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        url_parts.2, url_parts.0, url_parts.1, soap_action, body.len(), body
    );

    use std::io::Write;
    let mut stream = std::io::BufWriter::new(&sock);
    if stream.write_all(request.as_bytes()).is_err() {
        warn!(action = action, "UPnP: failed to send SOAP request");
        return false;
    }
    let _ = stream.flush();
    drop(stream);

    let mut response = String::new();
    use std::io::Read;
    let mut reader = std::io::BufReader::new(&sock);
    let _ = reader.read_to_string(&mut response);

    let ok = response.contains("200 OK");
    if ok {
        debug!(action = action, "UPnP: SOAP request succeeded");
    } else if let Some(idx) = response.find("<faultstring>") {
        let rest = &response[idx + 13..];
        if let Some(end) = rest.find("</faultstring>") {
            warn!(action = action, fault = %rest[..end], "UPnP: SOAP fault");
        }
    } else {
        warn!(
            action = action,
            response_len = response.len(),
            "UPnP: SOAP request failed"
        );
    }
    ok
}
