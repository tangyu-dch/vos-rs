use std::sync::atomic::{AtomicU64, Ordering};

/// 基于连续内存和无锁 AtomicU64 位图的 RTP 端口租用管理器。
///
/// 相比于 `DashSet<u16>`，它具有如下优势：
/// 1. **零堆内存分配**：在启动时一次性预分配连续的位图数组，运行期无任何动态分配。
/// 2. **无锁操作**：使用 CAS 原子操作指令，操作均在 \(O(1)\) 时间复杂度内完成，规避锁排队。
/// 3. **极小内存开销**：10000 个端口仅需 ~1.2KB 内存。
#[derive(Debug)]
pub struct PortLeaseMap {
    bitmap: Vec<AtomicU64>,
    port_min: u16,
    port_max: u16,
}

impl PortLeaseMap {
    /// 创建一个新的端口租用管理器，指定端口租用的闭区间 `[port_min, port_max]`。
    pub fn new(port_min: u16, port_max: u16) -> Self {
        let size = if port_max >= port_min {
            ((port_max - port_min) as usize / 64) + 1
        } else {
            0
        };
        let mut bitmap = Vec::with_capacity(size);
        for _ in 0..size {
            bitmap.push(AtomicU64::new(0));
        }
        Self {
            bitmap,
            port_min,
            port_max,
        }
    }

    /// 标记租用指定的端口。
    ///
    /// # 返回值
    /// - `true` — 租用成功（原先未被租用）
    /// - `false` — 租用失败（越界或已被其他线程租用）
    pub fn insert(&self, port: impl std::borrow::Borrow<u16>) -> bool {
        let p = *port.borrow();
        if p < self.port_min || p > self.port_max {
            return false;
        }
        let offset = p - self.port_min;
        let index = (offset / 64) as usize;
        let bit = offset % 64;
        let mask = 1u64 << bit;
        let atomic_val = &self.bitmap[index];

        let mut current = atomic_val.load(Ordering::Acquire);
        loop {
            if (current & mask) != 0 {
                return false;
            }
            let next = current | mask;
            match atomic_val.compare_exchange_weak(
                current,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return true,
                Err(actual) => current = actual,
            }
        }
    }

    /// 释放先前租用的端口。
    ///
    /// # 返回值
    /// - `true` — 释放成功（原先处于租用状态）
    /// - `false` — 释放失败（越界或本身就未被租用）
    pub fn remove(&self, port: impl std::borrow::Borrow<u16>) -> bool {
        let p = *port.borrow();
        if p < self.port_min || p > self.port_max {
            return false;
        }
        let offset = p - self.port_min;
        let index = (offset / 64) as usize;
        let bit = offset % 64;
        let mask = 1u64 << bit;
        let atomic_val = &self.bitmap[index];

        let mut current = atomic_val.load(Ordering::Acquire);
        loop {
            if (current & mask) == 0 {
                return false;
            }
            let next = current & !mask;
            match atomic_val.compare_exchange_weak(
                current,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return true,
                Err(actual) => current = actual,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn test_single_thread_lease_lifecycle() {
        let lease = PortLeaseMap::new(40000, 40100);

        // 越界保护
        assert!(!lease.insert(39999));
        assert!(!lease.insert(40101));

        // 正常借还
        assert!(lease.insert(40000));
        assert!(!lease.insert(40000)); // 重复插入失败
        assert!(lease.insert(40100));

        assert!(lease.remove(40000));
        assert!(!lease.remove(40000)); // 重复释放失败
        assert!(lease.insert(40000)); // 释放后可再次借用
    }

    #[test]
    fn test_high_concurrency_port_leasing() {
        let lease = Arc::new(PortLeaseMap::new(40000, 40999)); // 1000 个可用端口
        let mut handles = Vec::new();

        // 启动 10 个线程并发争抢端口
        for _thread_idx in 0..10 {
            let lease_clone = Arc::clone(&lease);
            handles.push(thread::spawn(move || {
                let mut succ_count = 0;
                // 每个线程尝试在可用范围内租用端口
                for port in 40000..41000 {
                    if lease_clone.insert(port) {
                        succ_count += 1;
                    }
                }
                succ_count
            }));
        }

        let mut total_allocated = 0;
        for handle in handles {
            total_allocated += handle.join().unwrap();
        }

        // 端口没有被超发，累计被租用数恰好等于可用端口数 1000
        assert_eq!(total_allocated, 1000);
    }
}
