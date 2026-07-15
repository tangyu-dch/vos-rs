//! # 内核态 eBPF/XDP RTP 极速中继模块
//!
//! 本模块实现 Linux 环境下的 eBPF/XDP 程序加载与 BPF Map 路由表下发。
//! 在非 Linux 操作系统下自动降级为 No-op 空实现。

#[cfg(target_os = "linux")]
use aya::{
    maps::HashMap,
    programs::{Xdp, XdpFlags},
    Bpf,
};
#[cfg(target_os = "linux")]
use lazy_static::lazy_static;
#[cfg(target_os = "linux")]
use std::sync::Mutex;

#[cfg(target_os = "linux")]
#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct RtpTuple {
    src_ip: u32,
    src_port: u16,
}

#[cfg(target_os = "linux")]
#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct RelayDest {
    dst_ip: u32,
    dst_port: u16,
}

#[cfg(target_os = "linux")]
lazy_static::lazy_static! {
    static ref BPF_INSTANCE: Mutex<Option<Bpf>> = Mutex::new(None);
}

/// 在指定网卡上加载并挂载 XDP RTP 转发程序
#[cfg(target_os = "linux")]
pub fn init_ebpf_xdp(iface: &str, elf_bytes: &[u8]) -> Result<(), String> {
    let mut bpf = Bpf::load(elf_bytes).map_err(|e| format!("BPF load failed: {e}"))?;
    if let Err(e) = aya_log::BpfLogger::init(&mut bpf) {
        tracing::warn!(%e, "Failed to initialize BpfLogger");
    }

    let program: &mut Xdp = bpf
        .program_mut("rtp_relay_prog")
        .ok_or("Failed to find rtp_relay_prog program")?
        .try_into()
        .map_err(|e| format!("Cast to Xdp program failed: {e}"))?;

    program
        .load()
        .map_err(|e| format!("Xdp load failed: {e}"))?;

    program
        .attach(iface, XdpFlags::default())
        .map_err(|e| format!("Xdp attach to {iface} failed: {e}"))?;

    tracing::info!(iface, "Successfully attached XDP eBPF RTP relay program");

    let mut guard = BPF_INSTANCE.lock().unwrap();
    *guard = Some(bpf);
    Ok(())
}

/// 写入一条 XDP RTP 中继规则到内核 BPF Map
#[cfg(target_os = "linux")]
pub fn register_ebpf_relay(
    src_ip: std::net::Ipv4Addr,
    src_port: u16,
    dst_ip: std::net::Ipv4Addr,
    dst_port: u16,
) -> Result<(), String> {
    let guard = BPF_INSTANCE.lock().unwrap();
    let bpf = match guard.as_ref() {
        Some(b) => b,
        None => return Ok(()), // eBPF 未加载时跳过
    };

    let mut rtp_map: HashMap<_, RtpTuple, RelayDest> = HashMap::try_from(
        bpf.map("rtp_relay_map")
            .ok_or("Failed to locate rtp_relay_map")?,
    )
    .map_err(|e| format!("Get map failed: {e}"))?;

    let key = RtpTuple {
        src_ip: u32::from_ne_bytes(src_ip.octets()),
        src_port: src_port.to_be(),
    };
    let value = RelayDest {
        dst_ip: u32::from_ne_bytes(dst_ip.octets()),
        dst_port: dst_port.to_be(),
    };

    rtp_map
        .insert(key, value, 0)
        .map_err(|e| format!("Failed to insert Map rule: {e}"))?;

    tracing::info!(
        ?src_ip,
        src_port,
        ?dst_ip,
        dst_port,
        "Inserted eBPF kernel-space relay rule"
    );
    Ok(())
}

/// 从内核 BPF Map 移除一条 XDP RTP 中继规则
#[cfg(target_os = "linux")]
pub fn unregister_ebpf_relay(src_ip: std::net::Ipv4Addr, src_port: u16) -> Result<(), String> {
    let guard = BPF_INSTANCE.lock().unwrap();
    let bpf = match guard.as_ref() {
        Some(b) => b,
        None => return Ok(()),
    };

    let mut rtp_map: HashMap<_, RtpTuple, RelayDest> = HashMap::try_from(
        bpf.map("rtp_relay_map")
            .ok_or("Failed to locate rtp_relay_map")?,
    )
    .map_err(|e| format!("Get map failed: {e}"))?;

    let key = RtpTuple {
        src_ip: u32::from_ne_bytes(src_ip.octets()),
        src_port: src_port.to_be(),
    };

    let _ = rtp_map.remove(&key);
    tracing::info!(?src_ip, src_port, "Removed eBPF kernel-space relay rule");
    Ok(())
}

// === 非 Linux 操作系统下的 No-op 桩实现 ===

#[cfg(not(target_os = "linux"))]
pub fn init_ebpf_xdp(_iface: &str, _elf_bytes: &[u8]) -> Result<(), String> {
    tracing::info!("eBPF/XDP is only supported on Linux, skipping hook initialization");
    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub fn register_ebpf_relay(
    _src_ip: std::net::Ipv4Addr,
    _src_port: u16,
    _dst_ip: std::net::Ipv4Addr,
    _dst_port: u16,
) -> Result<(), String> {
    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub fn unregister_ebpf_relay(_src_ip: std::net::Ipv4Addr, _src_port: u16) -> Result<(), String> {
    Ok(())
}
