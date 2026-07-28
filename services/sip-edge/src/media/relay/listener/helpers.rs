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

/// TURN 感知的媒体发送：若 TURN 客户端存在，通过中继路径发送并自动管理 CREATE-PERMISSION；
/// 否则回退到直接 UDP 发送。
///
/// 返回 `Ok(())` 表示发送成功（含 TURN 路径），`Err(io::Error)` 表示发送失败。
/// TURN 内部错误被映射为 `io::ErrorKind::Other`。
pub(super) async fn send_media_with_turn(
    relay: &MediaRelayState,
    relay_port: u16,
    socket: &tokio::net::UdpSocket,
    packet: &[u8],
    target: SocketAddr,
    turn_client: Option<&std::sync::Arc<crate::net::turn_client::TurnClient>>,
) -> std::io::Result<()> {
    if let Some(turn) = turn_client {
        // 首次向该对端发送时，创建 TURN CREATE-PERMISSION（异步阻塞但仅一次）
        if !relay.turn_peer_authorized(relay_port, target) {
            match turn.create_permission(target).await {
                Ok(()) => relay.mark_turn_peer_authorized(relay_port, target),
                Err(error) => {
                    tracing::warn!(
                        relay_port,
                        %target,
                        error = %error,
                        "TURN CREATE-PERMISSION 失败，回退到直连"
                    );
                    // 权限失败时回退到直连，避免媒体中断
                    return send_media_nonblocking(socket, packet, target).await;
                }
            }
        }
        turn.send_data(packet, target)
            .await
            .map_err(std::io::Error::other)
    } else {
        send_media_nonblocking(socket, packet, target).await
    }
}
