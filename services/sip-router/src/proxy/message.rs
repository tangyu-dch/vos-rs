use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
};

pub(crate) fn add_router_via(
    packet: &[u8],
    advertised_addr: &str,
    transport: &str,
    branch: &str,
    output: &mut Vec<u8>,
) -> Result<(), &'static str> {
    let split = packet
        .iter()
        .position(|byte| *byte == b'\n')
        .ok_or("SIP 起始行不完整")?
        + 1;
    let via = format!("Via: SIP/2.0/{transport} {advertised_addr};branch={branch};rport\r\n");
    output.extend_from_slice(&packet[..split]);
    output.extend_from_slice(via.as_bytes());
    output.extend_from_slice(&packet[split..]);
    Ok(())
}

pub(crate) fn router_branch(packet: &[u8], transport: &str) -> Result<String, &'static str> {
    let call_id = header_value(packet, &["call-id", "i"]).ok_or("SIP 请求缺少 Call-ID")?;
    let via = header_value(packet, &["via", "v"]).ok_or("SIP 请求缺少 Via")?;
    let cseq = header_value(packet, &["cseq"]).ok_or("SIP 请求缺少 CSeq")?;
    let mut hasher = DefaultHasher::new();
    transport.hash(&mut hasher);
    call_id.hash(&mut hasher);
    via.hash(&mut hasher);
    cseq.hash(&mut hasher);
    Ok(format!("z9hG4bK-vosrs-{:016x}", hasher.finish()))
}

pub(crate) fn top_via_branch(packet: &[u8]) -> Option<String> {
    header_value(packet, &["via", "v"]).and_then(|via| parameter(via, "branch"))
}

pub(crate) fn header_value<'a>(packet: &'a [u8], accepted_names: &[&str]) -> Option<&'a str> {
    let text = std::str::from_utf8(packet).ok()?;
    text.lines().skip(1).find_map(|line| {
        if line.trim().is_empty() {
            return None;
        }
        let (name, value) = line.split_once(':')?;
        accepted_names
            .iter()
            .any(|accepted| name.trim().eq_ignore_ascii_case(accepted))
            .then(|| value.trim())
    })
}

pub(super) fn request_method(packet: &[u8]) -> Option<&str> {
    std::str::from_utf8(packet)
        .ok()?
        .lines()
        .next()?
        .split_whitespace()
        .next()
}

pub(super) fn response_status(packet: &[u8]) -> Option<u16> {
    std::str::from_utf8(packet)
        .ok()?
        .lines()
        .next()?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}

fn parameter(value: &str, name: &str) -> Option<String> {
    value.split(';').skip(1).find_map(|part| {
        let (key, value) = part.trim().split_once('=')?;
        key.eq_ignore_ascii_case(name)
            .then(|| value.trim().to_string())
    })
}

pub(crate) fn remove_top_via(packet: &[u8], output: &mut Vec<u8>) -> Result<(), &'static str> {
    let text = std::str::from_utf8(packet).map_err(|_| "SIP 响应不是 UTF-8")?;
    let line_start = text.find('\n').ok_or("SIP 起始行不完整")? + 1;
    let relative_end = text[line_start..].find('\n').ok_or("Via 行不完整")? + 1;
    let line_end = line_start + relative_end;
    let first_header = text[line_start..line_end].trim_start();
    if !first_header.to_ascii_lowercase().starts_with("via:")
        && !first_header.to_ascii_lowercase().starts_with("v:")
    {
        return Err("路由器 Via 不是首个响应头");
    }
    output.extend_from_slice(&packet[..line_start]);
    output.extend_from_slice(&packet[line_end..]);
    Ok(())
}
