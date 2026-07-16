//! # Sans-I/O (无 I/O 耦合) RTP 转发状态机
//!
//! 本模块实现纯函数式、不带任何 I/O 及网络系统调用的 RTP 转发状态机内核。
//! 极大地提升了系统的测试性、可移植性以及多网卡驱动复用能力。

use rtp_core::RtpPacketView;
use std::net::SocketAddr;

/// Sans-I/O 转发决策动作
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelayAction {
    /// 执行数据包转发，发往指定目的地址
    Forward { packet: Vec<u8>, target: SocketAddr },
    /// 丢弃数据包，并给出理由
    Drop { reason: &'static str },
    /// 上送用户态控制面（例如处理 STUN / DTLS 握手）
    PassToUser,
}

/// 纯函数式 RTP 路由处理器
pub struct SansIoRelayKernel {
    pub local_port: u16,
    pub target: Option<SocketAddr>,
    pub peer_port: Option<u16>,
    pub muted: bool,
    pub dtmf_payload_type: Option<u8>,
}

impl SansIoRelayKernel {
    /// 输入一个原始 UDP 数据包，进行无 I/O 路由决策
    pub fn handle_packet(&self, packet: &[u8], _source: SocketAddr) -> RelayAction {
        if packet.is_empty() {
            return RelayAction::Drop {
                reason: "empty packet",
            };
        }

        // 1. 过滤识别 STUN & DTLS
        let first_byte = packet[0];
        if first_byte < 2 || (20..=63).contains(&first_byte) {
            return RelayAction::PassToUser;
        }

        // 2. 检查静音阻断状态
        if self.muted {
            return RelayAction::Drop {
                reason: "port is muted",
            };
        }

        // 3. 解析 RTP 包头（作为基本完整性校验）
        let _rtp = match RtpPacketView::parse(packet) {
            Ok(r) => r,
            Err(_) => {
                return RelayAction::Drop {
                    reason: "invalid RTP header",
                }
            }
        };

        // 4. 路由与反射决策
        let Some(target) = self.target else {
            return RelayAction::Drop {
                reason: "no target address configured",
            };
        };

        // 决策通过：执行纯字节数组转发，不进行任何 Socket I/O
        RelayAction::Forward {
            packet: packet.to_vec(),
            target,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sans_io_forward_success() {
        // 构造一个合法的简易 RTP 包 (12字节 Header + payload)
        let mut mock_rtp = vec![0u8; 12];
        mock_rtp[0] = 0x80; // Version 2
        mock_rtp[1] = 0x00; // Payload Type 0 (PCMU)
        mock_rtp[2] = 0x00;
        mock_rtp[3] = 0x01; // Seq 1
        mock_rtp[4..8].copy_from_slice(&[0, 0, 0, 1]); // TS 1
        mock_rtp[8..12].copy_from_slice(&[0, 0, 0, 1]); // SSRC 1

        let target_addr: SocketAddr = "127.0.0.1:40000".parse().unwrap();
        let kernel = SansIoRelayKernel {
            local_port: 30000,
            target: Some(target_addr),
            peer_port: None,
            muted: false,
            dtmf_payload_type: None,
        };

        let source_addr: SocketAddr = "127.0.0.1:50000".parse().unwrap();
        let action = kernel.handle_packet(&mock_rtp, source_addr);

        assert_eq!(
            action,
            RelayAction::Forward {
                packet: mock_rtp,
                target: target_addr
            }
        );
    }

    #[test]
    fn test_sans_io_muted_drop() {
        let mut mock_rtp = vec![0u8; 12];
        mock_rtp[0] = 0x80;

        let kernel = SansIoRelayKernel {
            local_port: 30000,
            target: Some("127.0.0.1:40000".parse().unwrap()),
            peer_port: None,
            muted: true, // 静音启用
            dtmf_payload_type: None,
        };

        let action = kernel.handle_packet(&mock_rtp, "127.0.0.1:50000".parse().unwrap());
        assert_eq!(
            action,
            RelayAction::Drop {
                reason: "port is muted"
            }
        );
    }

    #[test]
    fn test_sans_io_dtls_pass_to_user() {
        let dtls_client_hello = vec![22, 3, 1, 0, 42]; // DTLS Handshake (ContentType 22)
        let kernel = SansIoRelayKernel {
            local_port: 30000,
            target: Some("127.0.0.1:40000".parse().unwrap()),
            peer_port: None,
            muted: false,
            dtmf_payload_type: None,
        };

        let action = kernel.handle_packet(&dtls_client_hello, "127.0.0.1:50000".parse().unwrap());
        assert_eq!(action, RelayAction::PassToUser);
    }
}
