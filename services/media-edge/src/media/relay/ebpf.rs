//! eBPF/XDP 媒体转发扩展点。
//!
//! 当前发布包未包含经过验证的内核程序，因此所有平台都明确使用用户态 RTP 快路径。
//! 这里保留稳定接口，避免把未初始化的实验性 BPF Map 误判为已启用加速。

/// 拒绝加载尚未随发布包提供的 XDP 程序。
pub fn init_ebpf_xdp(_iface: &str, _elf_bytes: &[u8]) -> Result<(), String> {
    Err("当前构建未启用经过验证的 eBPF/XDP 媒体转发程序".to_string())
}

/// 用户态快路径不需要额外注册内核转发规则。
pub fn register_ebpf_relay(
    _src_ip: std::net::Ipv4Addr,
    _src_port: u16,
    _dst_ip: std::net::Ipv4Addr,
    _dst_port: u16,
) -> Result<(), String> {
    Ok(())
}

/// 用户态快路径不需要移除内核转发规则。
pub fn unregister_ebpf_relay(_src_ip: std::net::Ipv4Addr, _src_port: u16) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unavailable_ebpf_is_reported_explicitly() {
        assert!(init_ebpf_xdp("eth0", &[]).is_err());
    }
}
