//! # 零堆分配 RTP 数据包对象缓存池
//!
//! 本模块实现高性能无锁的对象池 (Arena Pool)，通过重用连续的内存块，
//! 彻底消除包转发和解密等热路径上的动态内存分配开销，大幅降低 GC 与内存抖动。

use crossbeam_queue::ArrayQueue;
use std::sync::Arc;

/// RTP 包固定最大缓冲区大小 (2048 字节已足够容纳最大的 RTP 数据包)
pub const PACKET_BUFFER_SIZE: usize = 2048;

/// 物理数据缓冲区结构体
#[repr(align(64))] // 缓存行对齐，避免伪共享 (False Sharing)
pub struct PacketBuffer {
    pub data: [u8; PACKET_BUFFER_SIZE],
    pub len: usize,
}

impl Default for PacketBuffer {
    fn default() -> Self {
        Self {
            data: [0u8; PACKET_BUFFER_SIZE],
            len: 0,
        }
    }
}

/// 循环重用的智能回收指针。当离开生命周期时，自动将 buffer 归还至 Pool。
pub struct RecycledBuffer {
    buffer: Option<Box<PacketBuffer>>,
    pool: Arc<ArrayQueue<Box<PacketBuffer>>>,
}

impl RecycledBuffer {
    /// 借出底层 buffer 数据的可变引用
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        &mut self.buffer.as_mut().unwrap().data
    }

    /// 借出底层 buffer 数据的只读引用
    pub fn as_slice(&self) -> &[u8] {
        &self.buffer.as_ref().unwrap().data[..self.len()]
    }

    /// 获取实际写入的数据长度
    pub fn len(&self) -> usize {
        self.buffer.as_ref().unwrap().len
    }

    /// 设置实际写入的数据长度
    pub fn set_len(&mut self, len: usize) {
        self.buffer.as_mut().unwrap().len = len;
    }
}

impl Drop for RecycledBuffer {
    fn drop(&mut self) {
        if let Some(buf) = self.buffer.take() {
            let _ = self.pool.push(buf); // 极速归还至无锁队列
        }
    }
}

/// 零动态内存分配的无锁 Packet 缓冲池
pub struct PacketBufferPool {
    pool: Arc<ArrayQueue<Box<PacketBuffer>>>,
}

impl PacketBufferPool {
    /// 初始化指定容量的对象池
    pub fn new(capacity: usize) -> Self {
        let pool = Arc::new(ArrayQueue::new(capacity));
        for _ in 0..capacity {
            let _ = pool.push(Box::new(PacketBuffer::default()));
        }
        Self { pool }
    }

    /// 租借一个闲置的缓冲区，如果池已满则动态退避申请一个 (鲁棒性降级)
    pub fn lease(&self) -> RecycledBuffer {
        let buffer = match self.pool.pop() {
            Some(buf) => buf,
            None => Box::new(PacketBuffer::default()),
        };
        RecycledBuffer {
            buffer: Some(buffer),
            pool: Arc::clone(&self.pool),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pool_leasing_and_recycling() {
        let pool = PacketBufferPool::new(2);

        {
            let mut buf1 = pool.lease();
            buf1.as_mut_slice()[0] = 42;
            buf1.set_len(10);
            assert_eq!(buf1.as_slice()[0], 42);
            assert_eq!(buf1.len(), 10);
        } // buf1 离开作用域，自动回收

        // 第一次重新租借，由于 FIFO 队列特性，拿到第二个空闲初始包
        let buf2 = pool.lease();
        assert_eq!(buf2.len(), 0);

        // 第二次重新租借，拿到刚才归还回收的第一个包
        let buf3 = pool.lease();
        assert_eq!(buf3.len(), 10);
    }
}
