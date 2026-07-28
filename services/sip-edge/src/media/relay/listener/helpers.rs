use super::*;

#[cfg(test)]
pub(super) fn set_socket_buffer_size(socket: &tokio::net::UdpSocket) {
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        let fd = socket.as_raw_fd();
        let buf_size = 262144_i32; // 256KB
        unsafe {
            libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_RCVBUF,
                &buf_size as *const i32 as *const libc::c_void,
                std::mem::size_of::<i32>() as libc::socklen_t,
            );
            libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_SNDBUF,
                &buf_size as *const i32 as *const libc::c_void,
                std::mem::size_of::<i32>() as libc::socklen_t,
            );
        }
    }
}

#[inline]
pub(super) async fn send_media_nonblocking(
    socket: &tokio::net::UdpSocket,
    packet: &[u8],
    target: SocketAddr,
) -> std::io::Result<()> {
    match socket.try_send_to(packet, target) {
        Ok(_) => Ok(()),
        Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
            socket.send_to(packet, target).await?;
            Ok(())
        }
        Err(e) => Err(e),
    }
}
