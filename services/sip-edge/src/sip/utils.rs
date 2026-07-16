/// 快速从原始 SIP 字节中提取 Call-ID 值（无需完整解析）。
///
/// 按行扫描报文头部，匹配 `Call-ID:` 或紧凑形式 `i:`，
/// 提取其值用于 Worker 路由哈希，确保同一 Dialog 的所有消息
/// 始终路由到同一个处理 Worker，避免并发竞态。
///
/// # 返回
/// - `Some(call_id)` — 成功提取
/// - `None` — 报文格式异常或不含 Call-ID（fallback 到 peer-IP 哈希）
pub(crate) fn extract_call_id_fast(packet: &[u8]) -> Option<&[u8]> {
    // SIP 消息头部为 ASCII，每行以 CRLF 或 LF 结尾
    let text = std::str::from_utf8(packet).ok()?;

    // 跳过请求行或状态行（第一行）
    let headers_start = text.find('\n').map(|i| i + 1)?;
    let headers = &text[headers_start..];

    // 遍历每一行，匹配 Call-ID 头（大小写不敏感）
    for line in headers.lines() {
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            // 空行：头部结束
            break;
        }
        // 匹配 "Call-ID:" 及紧凑形式 "i:"
        let value = if trimmed.len() > 8 && trimmed[..8].eq_ignore_ascii_case("call-id:") {
            trimmed[8..].trim()
        } else if trimmed.len() > 2 && trimmed[..2].eq_ignore_ascii_case("i:") {
            trimmed[2..].trim()
        } else {
            continue;
        };

        if value.is_empty() {
            return None;
        }
        // 在原始 packet 中找到 value 的位置并返回字节切片（零拷贝）
        if let Some(pos) = packet
            .windows(value.len())
            .position(|w| w == value.as_bytes())
        {
            return Some(&packet[pos..pos + value.len()]);
        }
        return None;
    }
    None
}
