//! RTP 热路径固定缓冲区池。

use crossbeam_queue::ArrayQueue;
use std::sync::Arc;

/// 常规 RTP/SRTP 数据包缓冲区大小。
pub const PACKET_BUFFER_SIZE: usize = 2_048;
const OVERSIZED_PACKET_TRAILER_CAPACITY: usize = 32;

#[repr(align(64))]
#[derive(Debug)]
struct PacketBuffer {
    data: [u8; PACKET_BUFFER_SIZE],
    len: usize,
}

impl Default for PacketBuffer {
    fn default() -> Self {
        Self {
            data: [0; PACKET_BUFFER_SIZE],
            len: 0,
        }
    }
}

/// 离开作用域时自动归还池中的数据缓冲区。
pub struct RecycledBuffer {
    buffer: Option<Box<PacketBuffer>>,
    pool: Arc<ArrayQueue<Box<PacketBuffer>>>,
}

impl RecycledBuffer {
    /// 返回缓冲区固定容量。
    pub const fn capacity(&self) -> usize {
        PACKET_BUFFER_SIZE
    }

    /// 返回当前有效数据。
    pub fn as_slice(&self) -> &[u8] {
        let Some(buffer) = self.buffer.as_ref() else {
            return &[];
        };
        &buffer.data[..buffer.len]
    }

    /// 返回完整可写容量；调用方写入后必须设置有效长度。
    pub fn as_mut_capacity(&mut self) -> &mut [u8] {
        let Some(buffer) = self.buffer.as_mut() else {
            return &mut [];
        };
        &mut buffer.data
    }

    /// 返回当前有效数据的可写切片。
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        let Some(buffer) = self.buffer.as_mut() else {
            return &mut [];
        };
        &mut buffer.data[..buffer.len]
    }

    /// 从数据切片复制内容，超过池容量时返回 `false`。
    pub fn copy_from_slice(&mut self, source: &[u8]) -> bool {
        if source.len() > PACKET_BUFFER_SIZE {
            return false;
        }
        let Some(buffer) = self.buffer.as_mut() else {
            return false;
        };
        buffer.data[..source.len()].copy_from_slice(source);
        buffer.len = source.len();
        true
    }

    /// 设置有效数据长度，超过池容量时返回 `false`。
    pub fn set_len(&mut self, len: usize) -> bool {
        if len > PACKET_BUFFER_SIZE {
            return false;
        }
        let Some(buffer) = self.buffer.as_mut() else {
            return false;
        };
        buffer.len = len;
        true
    }
}

/// 优先使用池缓冲区，超大 UDP 数据包自动退化为独立 `Vec`。
pub enum ReusablePacket {
    /// 来自无锁池的固定容量缓冲区。
    Pooled(RecycledBuffer),
    /// 超大包使用的独立缓冲区。
    Owned {
        /// 包含预留尾部空间的数据存储。
        data: Vec<u8>,
        /// 当前有效数据长度。
        len: usize,
    },
}

impl ReusablePacket {
    /// 返回当前有效数据。
    pub fn as_slice(&self) -> &[u8] {
        match self {
            Self::Pooled(buffer) => buffer.as_slice(),
            Self::Owned { data, len } => &data[..*len],
        }
    }

    /// 返回当前有效数据的可写切片。
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        match self {
            Self::Pooled(buffer) => buffer.as_mut_slice(),
            Self::Owned { data, len } => &mut data[..*len],
        }
    }

    /// 返回可用于原地追加认证标签的完整容量。
    pub fn as_mut_capacity(&mut self) -> &mut [u8] {
        match self {
            Self::Pooled(buffer) => buffer.as_mut_capacity(),
            Self::Owned { data, .. } => data,
        }
    }

    /// 更新有效长度，长度不得超过底层已预留容量。
    pub fn set_len(&mut self, len: usize) -> bool {
        match self {
            Self::Pooled(buffer) => buffer.set_len(len),
            Self::Owned { data, len: current } if len <= data.len() => {
                *current = len;
                true
            }
            Self::Owned { .. } => false,
        }
    }
}

impl Drop for RecycledBuffer {
    fn drop(&mut self) {
        if let Some(mut buffer) = self.buffer.take() {
            buffer.len = 0;
            let _ = self.pool.push(buffer);
        }
    }
}

/// 有界无锁 RTP 缓冲区池；池耗尽时临时分配并在归还时自动丢弃溢出项。
#[derive(Debug)]
pub struct PacketBufferPool {
    pool: Arc<ArrayQueue<Box<PacketBuffer>>>,
}

impl PacketBufferPool {
    /// 预分配指定数量的固定数据缓冲区。
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        let pool = Arc::new(ArrayQueue::new(capacity));
        for _ in 0..capacity {
            let _ = pool.push(Box::new(PacketBuffer::default()));
        }
        Self { pool }
    }

    /// 租借一个缓冲区。
    pub fn lease(&self) -> RecycledBuffer {
        let buffer = self
            .pool
            .pop()
            .unwrap_or_else(|| Box::new(PacketBuffer::default()));
        RecycledBuffer {
            buffer: Some(buffer),
            pool: Arc::clone(&self.pool),
        }
    }

    /// 复制数据到池缓冲区；超出固定容量时保留正确性并退化为 `Vec`。
    pub fn copy(&self, source: &[u8]) -> ReusablePacket {
        let mut buffer = self.lease();
        if buffer.copy_from_slice(source) {
            ReusablePacket::Pooled(buffer)
        } else {
            let capacity = source
                .len()
                .saturating_add(OVERSIZED_PACKET_TRAILER_CAPACITY);
            let mut data = vec![0; capacity];
            data[..source.len()].copy_from_slice(source);
            ReusablePacket::Owned {
                data,
                len: source.len(),
            }
        }
    }

    #[cfg(test)]
    fn available(&self) -> usize {
        self.pool.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_buffer_is_reset_and_recycled() {
        let pool = PacketBufferPool::new(1);
        {
            let mut buffer = pool.lease();
            assert!(buffer.copy_from_slice(&[1, 2, 3]));
            assert_eq!(buffer.as_slice(), &[1, 2, 3]);
        }
        assert_eq!(pool.available(), 1);
        assert!(pool.lease().as_slice().is_empty());
    }

    #[test]
    fn test_oversized_payload_is_rejected() {
        let pool = PacketBufferPool::new(1);
        let mut buffer = pool.lease();
        assert!(!buffer.copy_from_slice(&vec![0; PACKET_BUFFER_SIZE + 1]));
    }

    #[test]
    fn test_copy_falls_back_for_oversized_packet() {
        let pool = PacketBufferPool::new(1);
        let source = vec![7; PACKET_BUFFER_SIZE + 1];
        let packet = pool.copy(&source);
        assert_eq!(packet.as_slice(), source);
        assert!(matches!(packet, ReusablePacket::Owned { .. }));
    }
}
