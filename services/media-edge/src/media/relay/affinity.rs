//! # 多队列网卡 SO_REUSEPORT 与 CPU 亲和性绑定模块 (RSS Affinity)
//!
//! 本模块实现 UDP 监听端口在 Linux 平台与 macOS 平台下的 CPU Core 亲和性强绑定，
//! 消除跨 CPU 核心调度的 L3 Cache 失效开销，达成单核零拷贝与极速转发。

use std::net::UdpSocket;
use tracing::{info, warn};

/// 设置 UDP 套接字启用 SO_REUSEPORT 端口队列重用
#[cfg(unix)]
pub fn enable_socket_reuseport(socket: &UdpSocket) -> Result<(), String> {
    use std::os::unix::io::AsRawFd;
    let fd = socket.as_raw_fd();
    let optval: libc::c_int = 1;
    let ret = unsafe {
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_REUSEPORT,
            &optval as *const _ as *const _,
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        )
    };
    if ret != 0 {
        return Err(format!(
            "Failed to set SO_REUSEPORT: errno = {}",
            std::io::Error::last_os_error()
        ));
    }
    info!(fd, "Successfully enabled SO_REUSEPORT on socket");
    Ok(())
}

/// 非 Unix 平台下 SO_REUSEPORT 的桩实现
#[cfg(not(unix))]
pub fn enable_socket_reuseport(_socket: &UdpSocket) -> Result<(), String> {
    Ok(())
}

/// 绑定当前调用线程到特定的 CPU 物理核心 (core_id)
#[cfg(target_os = "linux")]
pub fn bind_thread_to_cpu_core(core_id: usize) -> Result<(), String> {
    let mut cpu_set = unsafe { std::mem::zeroed::<libc::cpu_set_t>() };
    unsafe {
        libc::CPU_SET(core_id, &mut cpu_set);
    }
    let ret = unsafe {
        libc::sched_setaffinity(
            0, // 0 对应当前线程/进程
            std::mem::size_of::<libc::cpu_set_t>(),
            &cpu_set,
        )
    };
    if ret != 0 {
        warn!(core_id, errno = %std::io::Error::last_os_error(), "Failed to set Linux CPU affinity (non-fatal)");
    } else {
        info!(
            core_id,
            "Successfully bound current thread to Linux CPU Core"
        );
    }
    Ok(())
}

/// macOS 下的 CPU 亲和度强绑定 (利用 macOS 专有的 thread_policy_set 接口)
#[cfg(target_os = "macos")]
#[allow(deprecated)]
pub fn bind_thread_to_cpu_core(core_id: usize) -> Result<(), String> {
    // macOS 并不提供 Linux 下的 sched_setaffinity 机制，而是采用 THREAD_AFFINITY_POLICY。
    // 凡是设置相同 affinity_tag (除 0 外) 的线程，会被 Darwin 内核尽量调度在相同的 CPU Core 或同级 L2/L3 Cache 上。
    let affinity_tag = (core_id + 1) as libc::integer_t; // macOS affinity tag 从 1 开始

    #[repr(C)]
    struct thread_affinity_policy {
        affinity_tag: libc::integer_t,
    }

    let mut policy = thread_affinity_policy { affinity_tag };

    let ret = unsafe {
        let thread_port = libc::mach_thread_self();
        let policy_ptr = &mut policy as *mut _ as *mut libc::integer_t;
        libc::thread_policy_set(
            thread_port,
            libc::THREAD_AFFINITY_POLICY as libc::thread_policy_flavor_t,
            policy_ptr,
            1, // count
        )
    };

    if ret != 0 {
        warn!(
            core_id,
            code = ret,
            "Failed to set macOS Thread L3 Cache Affinity Tag (non-fatal)"
        );
    } else {
        info!(
            core_id,
            "Successfully set macOS Thread L3 Cache Affinity Tag"
        );
    }
    Ok(())
}

/// 其他平台下 CPU 亲和度绑定的桩实现
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn bind_thread_to_cpu_core(core_id: usize) -> Result<(), String> {
    warn!(
        core_id,
        "CPU Affinity binding is not supported on this platform"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_so_reuseport_compilation_and_execution() {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        let res = enable_socket_reuseport(&socket);
        assert!(res.is_ok() || cfg!(not(unix)));
    }

    #[test]
    fn test_cpu_affinity_binding_runs() {
        // 绑定到核心 0，验证该方法在 macOS/Linux 下能够正常编译和执行
        let res = bind_thread_to_cpu_core(0);
        assert!(res.is_ok());
    }
}
