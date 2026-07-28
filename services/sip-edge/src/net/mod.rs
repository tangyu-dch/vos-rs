pub(crate) mod nat;
pub(crate) mod pool;
pub(crate) mod stun_client;
pub(crate) mod transport;
pub(crate) mod upnp;

pub(crate) use pool::{BufferPool, PooledBuffer};

pub(crate) use nat::{run_stun_discovery, run_upnp_port_mapping};
#[cfg(test)]
pub use transport::handle_ws_connection;
pub use transport::{create_tls_connector, handle_stream_connection, SipStream, Transport};

/// 在 Unix 平台上为 UDP socket 设置 IP_TOS（DSCP 标记）。
///
/// `dscp` 为 0 时不执行任何操作。DSCP 值左移 2 位后写入 IP 头部 TOS 字段
/// （RFC 2474），常见值：46=EF（Expedited Forwarding）、34=AF41。
#[cfg(unix)]
pub(crate) fn apply_dscp(socket: &std::net::UdpSocket, dscp: u8) {
    if dscp == 0 {
        return;
    }
    let tos = (dscp as libc::c_int) << 2;
    let fd = socket.as_raw_fd();
    // SAFETY: setsockopt with valid fd and properly typed arguments.
    let ret = unsafe {
        libc::setsockopt(
            fd,
            libc::IPPROTO_IP,
            libc::IP_TOS,
            &tos as *const _ as *const libc::c_void,
            std::mem::size_of_val(&tos) as libc::socklen_t,
        )
    };
    if ret < 0 {
        tracing::warn!(
            dscp,
            errno = std::io::Error::last_os_error().raw_os_error(),
            "failed to set IP_TOS"
        );
    } else {
        tracing::debug!(dscp, tos, "set IP_TOS on socket");
    }
}

#[cfg(not(unix))]
pub(crate) fn apply_dscp(_socket: &std::net::UdpSocket, _dscp: u8) {}

#[cfg(unix)]
use std::os::unix::io::AsRawFd;
