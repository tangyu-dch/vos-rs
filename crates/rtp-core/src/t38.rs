//! T.38 UDPTL（UDP Transport Layer）协议实现（ITU-T T.38）。
//!
//! T.38 用于在 IP 网络上传输实时 Group 3 传真（FoIP）。
//! UDPTL 是 T.38 的传输层，提供序号、错误恢复（FEC/冗余）能力。
//!
//! ## UDPTL 包结构
//!
//! UDPTL 包使用 ASN.1 PER（Packed Encoding Rules）编码，包含：
//! - 序列号（16 bit）
//! - 主要数据字段（primary IFP packet）
//! - 错误恢复字段（error_recovery，可选）：FEC 或 redundancy
//!
//! ## 实现范围
//!
//! 本模块实现 UDPTL 包的解析、构造和中继转发：
//! - [`UdptlPacket`]：零拷贝解析 UDPTL 包
//! - [`UdptlBuilder`]：构造 UDPTL 包
//! - [`UdptlRelay`]：T.38 中继器，按序号转发 IFP 包
//! - 支持 FEC（前向纠错）与冗余模式
//!
//! ## ASN.1 PER 简化编码
//!
//! UDPTL 包的 ASN.1 PER 编码采用以下简化规则：
//! - 序列号：16-bit big-endian
//! - primary field 长度：1 字节（最大 255，常见值 < 200）
//! - primary field 数据：可变长度
//! - error_recovery 选择项：1 字节 type（0=FEC, 1=redundancy）
//! - FEC：1 字节 count + count 个 (offset, data) 对
//! - redundancy：1 字节 count + count 个 (length, data) 对

use crate::error::{RtpError, RtpResult};

/// UDPTL 序列号空间大小（16-bit）。
pub const UDPTL_SEQ_MODULO: u32 = 65_536;

/// UDPTL 序列号回绕阈值（用于判断新旧包）。
pub const UDPTL_SEQ_THRESHOLD: u16 = 8_192;

/// UDPTL 错误恢复模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UdptlErrorRecovery {
    /// 无错误恢复（仅 primary field）。
    None,
    /// FEC 前向纠错（携带之前的 IFP 包异或校验和）。
    Fec,
    /// 冗余模式（携带之前 N 个 IFP 包的副本）。
    Redundancy,
}

/// UDPTL 包的零拷贝视图。
#[derive(Debug, Clone)]
pub struct UdptlPacketView<'a> {
    /// 序列号。
    pub sequence_number: u16,
    /// 主要 IFP 数据。
    pub primary_field: &'a [u8],
    /// 错误恢复模式。
    pub recovery: UdptlErrorRecovery,
    /// FEC 或冗余数据（按顺序排列）。
    pub recovery_fields: Vec<&'a [u8]>,
}

impl<'a> UdptlPacketView<'a> {
    /// 解析 UDPTL 包。
    ///
    /// UDPTL 包格式（简化 ASN.1 PER）：
    /// ```text
    /// +------+------+-----------+----------+-----------+
    /// | seq  | plen | primary   | rtype    | recovery  |
    /// | 2B   | 1B   | [plen]B   | 0/1B     | variable  |
    /// +------+------+-----------+----------+-----------+
    /// ```
    /// - `seq`：16-bit 序列号
    /// - `plen`：primary field 长度
    /// - `primary`：primary IFP 数据
    /// - `rtype`：错误恢复类型（0=FEC, 1=redundancy），若不存在则无恢复字段
    /// - `recovery`：FEC 数据或冗余 IFP 包列表
    pub fn parse(raw: &'a [u8]) -> RtpResult<Self> {
        if raw.len() < 3 {
            return Err(RtpError::InvalidPacketFormat(
                "UDPTL packet too short".to_string(),
            ));
        }

        let seq = u16::from_be_bytes([raw[0], raw[1]]);
        let primary_len = raw[2] as usize;
        if raw.len() < 3 + primary_len {
            return Err(RtpError::InvalidPacketFormat(format!(
                "UDPTL primary field truncated: expected {primary_len} bytes, got {}",
                raw.len() - 3
            )));
        }

        let primary_field = &raw[3..3 + primary_len];
        let mut offset = 3 + primary_len;
        let mut recovery = UdptlErrorRecovery::None;
        let mut recovery_fields = Vec::new();

        if offset < raw.len() {
            // 存在 error_recovery 字段
            let rtype = raw[offset];
            offset += 1;
            match rtype {
                0 => {
                    // FEC 模式
                    recovery = UdptlErrorRecovery::Fec;
                    if offset >= raw.len() {
                        return Err(RtpError::InvalidPacketFormat(
                            "UDPTL FEC field truncated".to_string(),
                        ));
                    }
                    let fec_count = raw[offset] as usize;
                    offset += 1;
                    for _ in 0..fec_count {
                        if offset + 2 > raw.len() {
                            return Err(RtpError::InvalidPacketFormat(
                                "UDPTL FEC entry truncated".to_string(),
                            ));
                        }
                        let entry_len = u16::from_be_bytes([raw[offset], raw[offset + 1]]) as usize;
                        offset += 2;
                        if offset + entry_len > raw.len() {
                            return Err(RtpError::InvalidPacketFormat(
                                "UDPTL FEC entry data truncated".to_string(),
                            ));
                        }
                        recovery_fields.push(&raw[offset..offset + entry_len]);
                        offset += entry_len;
                    }
                }
                1 => {
                    // 冗余模式
                    recovery = UdptlErrorRecovery::Redundancy;
                    if offset >= raw.len() {
                        return Err(RtpError::InvalidPacketFormat(
                            "UDPTL redundancy field truncated".to_string(),
                        ));
                    }
                    let red_count = raw[offset] as usize;
                    offset += 1;
                    for _ in 0..red_count {
                        if offset >= raw.len() {
                            return Err(RtpError::InvalidPacketFormat(
                                "UDPTL redundancy entry length truncated".to_string(),
                            ));
                        }
                        let entry_len = raw[offset] as usize;
                        offset += 1;
                        if offset + entry_len > raw.len() {
                            return Err(RtpError::InvalidPacketFormat(
                                "UDPTL redundancy entry data truncated".to_string(),
                            ));
                        }
                        recovery_fields.push(&raw[offset..offset + entry_len]);
                        offset += entry_len;
                    }
                }
                _ => {
                    return Err(RtpError::InvalidPacketFormat(format!(
                        "UDPTL unknown error_recovery type: {rtype}"
                    )));
                }
            }
        }

        Ok(Self {
            sequence_number: seq,
            primary_field,
            recovery,
            recovery_fields,
        })
    }

    /// 返回完整 UDPTL 包的字节长度。
    pub fn total_len(&self) -> usize {
        let mut len = 3 + self.primary_field.len();
        if self.recovery != UdptlErrorRecovery::None {
            len += 1; // rtype
            len += 1; // count
            for field in &self.recovery_fields {
                len += match self.recovery {
                    UdptlErrorRecovery::Fec => 2,        // u16 length prefix
                    UdptlErrorRecovery::Redundancy => 1, // u8 length prefix
                    UdptlErrorRecovery::None => 0,
                };
                len += field.len();
            }
        }
        len
    }
}

/// UDPTL 包构造器。
pub struct UdptlBuilder {
    sequence_number: u16,
    primary_field: Vec<u8>,
    recovery: UdptlErrorRecovery,
    recovery_fields: Vec<Vec<u8>>,
}

impl UdptlBuilder {
    /// 创建一个新的 UDPTL 包构造器。
    pub fn new(sequence_number: u16, primary_field: Vec<u8>) -> Self {
        Self {
            sequence_number,
            primary_field,
            recovery: UdptlErrorRecovery::None,
            recovery_fields: Vec::new(),
        }
    }

    /// 添加 FEC 错误恢复字段。
    pub fn with_fec(mut self, fec_fields: Vec<Vec<u8>>) -> Self {
        self.recovery = UdptlErrorRecovery::Fec;
        self.recovery_fields = fec_fields;
        self
    }

    /// 添加冗余错误恢复字段。
    pub fn with_redundancy(mut self, redundancy_fields: Vec<Vec<u8>>) -> Self {
        self.recovery = UdptlErrorRecovery::Redundancy;
        self.recovery_fields = redundancy_fields;
        self
    }

    /// 编码为字节序列。
    pub fn build(&self) -> RtpResult<Vec<u8>> {
        if self.primary_field.len() > 255 {
            return Err(RtpError::InvalidPacketFormat(format!(
                "UDPTL primary field too large: {} bytes (max 255)",
                self.primary_field.len()
            )));
        }
        if self.recovery_fields.len() > 255 {
            return Err(RtpError::InvalidPacketFormat(format!(
                "UDPTL recovery fields count too large: {} (max 255)",
                self.recovery_fields.len()
            )));
        }

        let mut bytes = Vec::with_capacity(self.estimated_size());
        bytes.extend_from_slice(&self.sequence_number.to_be_bytes());
        bytes.push(self.primary_field.len() as u8);
        bytes.extend_from_slice(&self.primary_field);

        if self.recovery != UdptlErrorRecovery::None {
            bytes.push(match self.recovery {
                UdptlErrorRecovery::Fec => 0,
                UdptlErrorRecovery::Redundancy => 1,
                UdptlErrorRecovery::None => 0,
            });
            bytes.push(self.recovery_fields.len() as u8);
            for field in &self.recovery_fields {
                match self.recovery {
                    UdptlErrorRecovery::Fec => {
                        if field.len() > u16::MAX as usize {
                            return Err(RtpError::InvalidPacketFormat(format!(
                                "UDPTL FEC field too large: {} bytes",
                                field.len()
                            )));
                        }
                        bytes.extend_from_slice(&(field.len() as u16).to_be_bytes());
                    }
                    UdptlErrorRecovery::Redundancy => {
                        if field.len() > 255 {
                            return Err(RtpError::InvalidPacketFormat(format!(
                                "UDPTL redundancy field too large: {} bytes",
                                field.len()
                            )));
                        }
                        bytes.push(field.len() as u8);
                    }
                    UdptlErrorRecovery::None => {}
                }
                bytes.extend_from_slice(field);
            }
        }

        Ok(bytes)
    }

    fn estimated_size(&self) -> usize {
        let mut size = 3 + self.primary_field.len();
        if self.recovery != UdptlErrorRecovery::None {
            size += 2;
            for field in &self.recovery_fields {
                size += match self.recovery {
                    UdptlErrorRecovery::Fec => 2,
                    UdptlErrorRecovery::Redundancy => 1,
                    UdptlErrorRecovery::None => 0,
                };
                size += field.len();
            }
        }
        size
    }
}

/// UDPTL 序列号比较工具。
///
/// 处理 16-bit 序号回绕：返回 `a - b` 的有符号差值，
/// 正数表示 a 在 b 之后，负数表示 a 在 b 之前。
pub fn seq_diff(a: u16, b: u16) -> i32 {
    let diff = (a as i32) - (b as i32);
    if diff > (UDPTL_SEQ_MODULO as i32 / 2) {
        diff - UDPTL_SEQ_MODULO as i32
    } else if diff < -(UDPTL_SEQ_MODULO as i32 / 2) {
        diff + UDPTL_SEQ_MODULO as i32
    } else {
        diff
    }
}

/// 判断序列号 `a` 是否比 `b` 新（即 a > b 考虑回绕）。
pub fn seq_is_newer(a: u16, b: u16) -> bool {
    seq_diff(a, b) > 0
}

/// T.38 UDPTL 中继器：按序号转发 IFP 包，支持乱序重排与去重。
///
/// 中继策略：
/// - 接收到的包按序号排序，按顺序转发到对端
/// - 维护最近已转发的序号窗口，避免重复转发
/// - 乱序包在窗口内（默认 32）允许重排后按序转发
/// - 超出窗口的旧包直接丢弃
pub struct UdptlRelay {
    /// 上次转发的序号。
    last_forwarded_seq: u16,
    /// 是否已转发过任何包。
    initialized: bool,
    /// 已接收但尚未按序转发的包缓存（按序号索引）。
    pending: std::collections::BTreeMap<u16, Vec<u8>>,
    /// 接收窗口大小（超出窗口的旧包丢弃）。
    window_size: u16,
    /// 已转发过的序号集合（用于去重，环形缓冲）。
    forwarded_set: std::collections::HashSet<u16>,
}

impl UdptlRelay {
    /// 创建 UDPTL 中继器，指定接收窗口大小（默认 32）。
    pub fn new(window_size: u16) -> Self {
        Self {
            last_forwarded_seq: 0,
            initialized: false,
            pending: std::collections::BTreeMap::new(),
            window_size: window_size.max(1),
            forwarded_set: std::collections::HashSet::new(),
        }
    }

    /// 接收一个 UDPTL 包，返回应立即转发到对端的 IFP 包列表。
    ///
    /// 返回的每个元素是已编码的 UDPTL 包（含序号与 primary field），
    /// 错误恢复字段会被剥离，由对端再生成新的恢复字段。
    pub fn receive(&mut self, packet: &UdptlPacketView<'_>) -> Vec<Vec<u8>> {
        let seq = packet.sequence_number;

        // 去重：已转发过的序号跳过
        if self.forwarded_set.contains(&seq) {
            return Vec::new();
        }

        // 旧包判定：超出窗口的旧序号直接丢弃
        if self.initialized {
            let diff = seq_diff(seq, self.last_forwarded_seq);
            if diff < -(self.window_size as i32) {
                // 过旧的包，丢弃
                return Vec::new();
            }
            if diff <= 0 {
                // 窗口内的旧包，记录但不立即转发
                self.pending.insert(seq, packet.primary_field.to_vec());
                self.forwarded_set.insert(seq);
                return Vec::new();
            }
        }

        // 缓存当前包
        self.pending.insert(seq, packet.primary_field.to_vec());
        self.forwarded_set.insert(seq);

        // 按序转发：从 last_forwarded_seq + 1 开始连续转发
        let mut to_forward = Vec::new();
        if !self.initialized {
            // 首个包：直接转发
            if let Some(data) = self.pending.remove(&seq) {
                let builder = UdptlBuilder::new(seq, data);
                if let Ok(encoded) = builder.build() {
                    to_forward.push(encoded);
                }
            }
            self.last_forwarded_seq = seq;
            self.initialized = true;
        } else {
            // 从 last + 1 开始连续转发
            let mut next_seq = self.last_forwarded_seq.wrapping_add(1);
            while let Some(data) = self.pending.remove(&next_seq) {
                let builder = UdptlBuilder::new(next_seq, data);
                if let Ok(encoded) = builder.build() {
                    to_forward.push(encoded);
                }
                self.last_forwarded_seq = next_seq;
                next_seq = next_seq.wrapping_add(1);
            }
        }

        // 清理过期的 forwarded_set 条目（避免内存膨胀）
        if self.forwarded_set.len() > 256 {
            let cutoff_diff = -(self.window_size as i32 * 2);
            let last = self.last_forwarded_seq;
            self.forwarded_set
                .retain(|&s| seq_diff(s, last) >= cutoff_diff);
        }

        to_forward
    }

    /// 返回上次转发的序号。
    pub fn last_forwarded_seq(&self) -> Option<u16> {
        if self.initialized {
            Some(self.last_forwarded_seq)
        } else {
            None
        }
    }

    /// 返回当前待转发的包数量。
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }
}

impl Default for UdptlRelay {
    fn default() -> Self {
        Self::new(32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_udptl_packet() {
        let raw = [0x12, 0x34, 0x02, 0xAB, 0xCD];
        let packet = UdptlPacketView::parse(&raw).unwrap();
        assert_eq!(packet.sequence_number, 0x1234);
        assert_eq!(packet.primary_field, &[0xAB, 0xCD]);
        assert_eq!(packet.recovery, UdptlErrorRecovery::None);
        assert!(packet.recovery_fields.is_empty());
    }

    #[test]
    fn parses_redundancy_packet() {
        // seq=100, primary=[0xAA], redundancy: 1 entry of [0xBB]
        let raw = [0x00, 0x64, 0x01, 0xAA, 0x01, 0x01, 0x01, 0xBB];
        let packet = UdptlPacketView::parse(&raw).unwrap();
        assert_eq!(packet.sequence_number, 100);
        assert_eq!(packet.primary_field, &[0xAA]);
        assert_eq!(packet.recovery, UdptlErrorRecovery::Redundancy);
        assert_eq!(packet.recovery_fields.len(), 1);
        assert_eq!(packet.recovery_fields[0], &[0xBB]);
    }

    #[test]
    fn parses_fec_packet() {
        // seq=200, primary=[0xCC], FEC: 1 entry of length 2 [0x11, 0x22]
        let raw = [0x00, 0xC8, 0x01, 0xCC, 0x00, 0x01, 0x00, 0x02, 0x11, 0x22];
        let packet = UdptlPacketView::parse(&raw).unwrap();
        assert_eq!(packet.sequence_number, 200);
        assert_eq!(packet.primary_field, &[0xCC]);
        assert_eq!(packet.recovery, UdptlErrorRecovery::Fec);
        assert_eq!(packet.recovery_fields.len(), 1);
        assert_eq!(packet.recovery_fields[0], &[0x11, 0x22]);
    }

    #[test]
    fn rejects_truncated_packet() {
        assert!(UdptlPacketView::parse(&[0x00, 0x01]).is_err());
        assert!(UdptlPacketView::parse(&[0x00, 0x01, 0x05, 0xAB]).is_err());
    }

    #[test]
    fn builds_simple_packet() {
        let bytes = UdptlBuilder::new(0x1234, vec![0xAB, 0xCD]).build().unwrap();
        assert_eq!(bytes, vec![0x12, 0x34, 0x02, 0xAB, 0xCD]);
    }

    #[test]
    fn builds_redundancy_packet() {
        let bytes = UdptlBuilder::new(100, vec![0xAA])
            .with_redundancy(vec![vec![0xBB]])
            .build()
            .unwrap();
        assert_eq!(bytes, vec![0x00, 0x64, 0x01, 0xAA, 0x01, 0x01, 0x01, 0xBB]);
    }

    #[test]
    fn builds_fec_packet() {
        let bytes = UdptlBuilder::new(200, vec![0xCC])
            .with_fec(vec![vec![0x11, 0x22]])
            .build()
            .unwrap();
        assert_eq!(
            bytes,
            vec![0x00, 0xC8, 0x01, 0xCC, 0x00, 0x01, 0x00, 0x02, 0x11, 0x22]
        );
    }

    #[test]
    fn rejects_oversized_primary_field() {
        let large = vec![0_u8; 256];
        let result = UdptlBuilder::new(1, large).build();
        assert!(result.is_err());
    }

    #[test]
    fn seq_diff_handles_wraparound() {
        assert_eq!(seq_diff(10, 5), 5);
        assert_eq!(seq_diff(5, 10), -5);
        // 回绕：65535 -> 0 -> 5，diff(5, 65535) = 6
        assert_eq!(seq_diff(5, 65535), 6);
        assert_eq!(seq_diff(65535, 5), -6);
    }

    #[test]
    fn seq_is_newer_handles_wraparound() {
        assert!(seq_is_newer(10, 5));
        assert!(!seq_is_newer(5, 10));
        assert!(seq_is_newer(5, 65535));
        assert!(!seq_is_newer(65535, 5));
    }

    #[test]
    fn relay_forwards_first_packet_immediately() {
        let mut relay = UdptlRelay::default();
        let raw = [0x00, 0x64, 0x01, 0xAA];
        let packet = UdptlPacketView::parse(&raw).unwrap();
        let forwarded = relay.receive(&packet);
        assert_eq!(forwarded.len(), 1);
        assert_eq!(relay.last_forwarded_seq(), Some(100));
    }

    #[test]
    fn relay_dedupes_duplicate_packets() {
        let mut relay = UdptlRelay::default();
        let raw = [0x00, 0x64, 0x01, 0xAA];
        let packet = UdptlPacketView::parse(&raw).unwrap();
        let _ = relay.receive(&packet);
        let forwarded = relay.receive(&packet);
        assert!(forwarded.is_empty());
    }

    #[test]
    fn relay_reorders_out_of_order_packets() {
        let mut relay = UdptlRelay::default();
        // 首包 seq=100
        let raw1 = [0x00, 0x64, 0x01, 0xAA];
        let p1 = UdptlPacketView::parse(&raw1).unwrap();
        let f1 = relay.receive(&p1);
        assert_eq!(f1.len(), 1);

        // seq=102 先到：不转发（缓存）
        let raw3 = [0x00, 0x66, 0x01, 0xCC];
        let p3 = UdptlPacketView::parse(&raw3).unwrap();
        let f3 = relay.receive(&p3);
        assert!(f3.is_empty());
        assert_eq!(relay.pending_count(), 1);

        // seq=101 后到：转发 101 和 102
        let raw2 = [0x00, 0x65, 0x01, 0xBB];
        let p2 = UdptlPacketView::parse(&raw2).unwrap();
        let f2 = relay.receive(&p2);
        assert_eq!(f2.len(), 2);
        assert_eq!(relay.last_forwarded_seq(), Some(102));
        assert_eq!(relay.pending_count(), 0);
    }

    #[test]
    fn relay_drops_too_old_packets() {
        let mut relay = UdptlRelay::default();
        // 首包 seq=1000
        let raw1 = [0x03, 0xE8, 0x01, 0xAA];
        let p1 = UdptlPacketView::parse(&raw1).unwrap();
        let _ = relay.receive(&p1);

        // seq=500（远在窗口外）：应被丢弃
        let raw2 = [0x01, 0xF4, 0x01, 0xBB];
        let p2 = UdptlPacketView::parse(&raw2).unwrap();
        let f2 = relay.receive(&p2);
        assert!(f2.is_empty());
        assert_eq!(relay.last_forwarded_seq(), Some(1000));
    }

    #[test]
    fn relay_default_window_is_32() {
        let relay = UdptlRelay::default();
        assert_eq!(relay.window_size, 32);
    }

    #[test]
    fn relay_clears_pending_after_forwarding() {
        let mut relay = UdptlRelay::default();
        let raw = [0x00, 0x0A, 0x01, 0xAA];
        let p = UdptlPacketView::parse(&raw).unwrap();
        let _ = relay.receive(&p);
        assert_eq!(relay.pending_count(), 0);
    }

    #[test]
    fn total_len_matches_estimated() {
        let bytes = UdptlBuilder::new(100, vec![0xAA])
            .with_redundancy(vec![vec![0xBB, 0xCC]])
            .build()
            .unwrap();
        let packet = UdptlPacketView::parse(&bytes).unwrap();
        assert_eq!(packet.total_len(), bytes.len());
    }
}
